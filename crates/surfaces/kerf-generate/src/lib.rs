//! Where Kerf asks a model for a file: the one place a `generate` step is carried out.
//!
//! A person sets up what reaches their models in `~/.kerf/generation.json` ([`settings`]), as
//! a chat's provider is set up: a server that speaks the Kerf generation API, reached with an API
//! key ([`endpoint`]), or a program on their own computer ([`command`]). A [`Generator`] is
//! that file read, with the models each provider reaches; the engine is handed it as its
//! [`kerf_engine::recipe::model::Generator`] and knows nothing of either.
//!
//! A model is named by its name alone where one provider has it, and as `<provider>/<model>`
//! where two do (`model=pod/wan-vace`).

pub mod command;
pub mod endpoint;
pub mod settings;

use kerf_engine::error::{EngineError, Result};
use kerf_engine::recipe::model::{Catalog, CatalogModel, Generation};
use settings::{ProviderSettings, Settings};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

pub use endpoint::{give_key, key, models as ask_models, secret_name, KEY_ENV};
pub use kerf_generate_api as api;

/// The file this process reads in place of the one the environment names, set by
/// [`Generator::use_file`].
static FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// What reaches a person's models: their providers, and the models each reaches.
#[derive(Clone, Debug, Default)]
pub struct Generator {
    settings: Settings,
    catalog: Catalog,
    /// Each provider that could not be asked which models it reaches, with why.
    unreached: Vec<(String, String)>,
}

impl Generator {
    /// The person's providers: the file [`Generator::default_file`] says, each server asked
    /// which models it reaches (or as Kerf's cache last kept them, where it cannot be). No file
    /// is no models.
    pub fn load() -> Result<Self> {
        Self::read(&Self::default_file())
    }

    /// Read the person's providers from `file` from here on, in place of the file
    /// `KERF_GENERATION` names or `~/.kerf/generation.json`, leaving this process's environment
    /// as it is: for a host that keeps them apart, as a test does.
    pub fn use_file(file: impl Into<PathBuf>) {
        *FILE.write().unwrap_or_else(PoisonError::into_inner) = Some(file.into());
    }

    /// Where [`Generator::load`] reads: the file [`Generator::use_file`] named, else the file
    /// `KERF_GENERATION` names, or `~/.kerf/generation.json`.
    pub fn default_file() -> PathBuf {
        if let Some(file) = FILE.read().unwrap_or_else(PoisonError::into_inner).clone() {
            return file;
        }
        std::env::var_os("KERF_GENERATION")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| kerf_paths::user_dir().map(|folder| folder.join("generation.json")))
            .unwrap_or_else(|| PathBuf::from("generation.json"))
    }

    /// The providers `file` sets up, and the models they reach. What is wrong in it is said
    /// with the provider; a server that cannot be reached is not wrong, and is said by
    /// [`Generator::unreached`].
    pub fn read(file: &Path) -> Result<Self> {
        let settings = Settings::read(file).map_err(EngineError::msg)?;
        let mut models = Vec::new();
        let mut unreached = Vec::new();
        for (provider, said) in settings.providers() {
            match said {
                ProviderSettings::Command { run, takes } => models.push(CatalogModel {
                    provider: provider.to_owned(),
                    name: provider.to_owned(),
                    spec: takes.clone(),
                    by: command::shown(run),
                }),
                ProviderSettings::Endpoint { url } => {
                    let (listed, why) = endpoint::listed(provider, url);
                    if let Some(why) = why {
                        unreached.push((provider.to_owned(), why));
                    }
                    models.extend(listed.into_iter().map(|model| CatalogModel {
                        provider: provider.to_owned(),
                        name: model.name,
                        spec: Some(model.spec),
                        by: url.clone(),
                    }));
                }
            }
        }
        let note = note(file, &unreached);
        Ok(Self {
            settings,
            catalog: Catalog {
                file: file.to_path_buf(),
                models,
                note,
            },
            unreached,
        })
    }

    /// The file that sets them up, there or not.
    pub fn file(&self) -> &Path {
        &self.catalog.file
    }

    /// The providers, as the file says them.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Each server that could not be asked which models it reaches, by its provider's name,
    /// with why: the models it is known to reach are those Kerf's cache last kept, if any.
    pub fn unreached(&self) -> &[(String, String)] {
        &self.unreached
    }
}

/// What a person whose model is not found is told beside which models there are: that the
/// models file of an older Kerf is read no more, where theirs is still there; that a server
/// could not be asked which models it reaches.
fn note(file: &Path, unreached: &[(String, String)]) -> Option<String> {
    let mut said = Vec::new();
    let old = file.with_file_name("models.json");
    if old.is_file() && !file.is_file() {
        said.push(format!(
            "{} is read no more: its models are set up in {}, under \"providers\"",
            old.display(),
            file.display()
        ));
    }
    for (provider, why) in unreached {
        said.push(format!("provider `{provider}` was not reached: {why}"));
    }
    (!said.is_empty()).then(|| said.join("; "))
}

impl kerf_engine::recipe::model::Generator for Generator {
    fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn generate(&self, generation: &Generation, on_line: &mut dyn FnMut(&str)) -> Result<()> {
        let model = self.catalog.find(&generation.request.model)?;
        let provider = self
            .settings
            .providers()
            .find(|(name, _)| *name == model.provider)
            .map(|(_, said)| said)
            .expect("every model of the catalog is a provider's");
        match provider {
            ProviderSettings::Command { run, takes } => command::generate(
                &generation.request.model,
                run,
                takes.as_ref(),
                generation,
                on_line,
            ),
            ProviderSettings::Endpoint { url } => {
                endpoint::generate(&model.provider, url, &model.name, generation, on_line)
            }
        }
    }
}
