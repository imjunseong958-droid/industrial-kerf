//! A model a recipe asks for a file, and what reaches it.
//!
//! Kerf has no model of its own and knows none by name. A recipe's `generate` line says what
//! is wanted in words any model takes: a prompt, what guides it, how long, what size, a seed.
//! Kerf writes that as a request ([`GenerateRequest`], the Kerf generation API's), a JSON file
//! beside what the take's steps made ([`REQUEST_FILE`]), and hands it to a [`Generator`], which
//! asks the model however it is reached and writes the file at the request's `out`.
//!
//! The engine knows no server and runs no program for a model: what reaches the models a
//! person has set up is `kerf-generate`'s, which a host hands the engine. A generator says
//! which models it reaches and what each takes ([`Catalog`]), so whoever writes a recipe, an
//! agent included, learns it without anything being run.
//!
//! `signals` holds each signal the line names ([`super::signal`]): a number for each frame of
//! the clip's sequence, at that sequence's rate.
//!
//! For a model whose `reads` has `regions`, `regions` holds what the clips of the guide Kerf
//! draws say they are ([`super::regions`]): each prompt once, with a grey video drawn as the
//! guide is, white where the guide shows the clips that say it:
//! `[{"prompt": "a red car", "mask": "…/region1.mp4"}]`.

use super::{moves::Moves, signal::Signal};
use crate::error::{EngineError, Result};
use crate::process::sidecar::CancellationToken;
use std::path::{Path, PathBuf};

pub use kerf_generate_api::{GenerateRequest, ModelSpec, Region, REQUEST_FILE, REQUEST_VERSION};
pub use kerf_model::making::{names_a_guide, Ask, Source, FILES, MOVES, REGIONS, SIGNAL};

// What a line hands a model is named once in the language and once in the API a server reads:
// the two say the same names.
const _: () = {
    const fn same(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut at = 0;
        while at < a.len() {
            if a[at] != b[at] {
                return false;
            }
            at += 1;
        }
        true
    }
    use kerf_generate_api::reads;
    assert!(same(REGIONS, reads::REGIONS));
    assert!(same(SIGNAL, reads::SIGNAL));
    assert!(same(MOVES, reads::MOVES));
    let mut at = 0;
    while at < FILES.len() {
        assert!(same(FILES[at], reads::FILES[at]));
        at += 1;
    }
};

/// What a making hands a [`Generator`] for one `generate` step.
pub struct Generation<'a> {
    /// What is asked, with each file where it is on this computer, as [`Generation::file`]
    /// holds it.
    pub request: &'a GenerateRequest,
    /// The request's file, written before the generator is asked: a program written for Kerf
    /// is handed it.
    pub file: &'a Path,
    /// The project's folder, which a program a model runs runs in.
    pub directory: &'a Path,
    /// Asked to stop: the generator stops what it started and answers
    /// [`EngineError::Cancelled`].
    pub cancellation: &'a CancellationToken,
}

/// What reaches the models a person has set up: a server, a program on their computer. The
/// engine asks it which models there are and asks it for a step's file, and knows nothing of
/// how either is done.
pub trait Generator {
    /// The models it reaches, each with what it takes, as they are known now.
    fn catalog(&self) -> &Catalog;

    /// Ask for what `generation` asks, writing the file at its request's `out`. What it says
    /// on the way goes to `on_line`, a line at a time. It ends well only once the file is
    /// written.
    fn generate(&self, generation: &Generation, on_line: &mut dyn FnMut(&str)) -> Result<()>;
}

/// The models a [`Generator`] reaches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    /// The file the person sets models up in, there or not.
    pub file: PathBuf,
    pub models: Vec<CatalogModel>,
    /// What to tell a person whose model is not found, beside which there are: that the file
    /// they set models up in has moved, say.
    pub note: Option<String>,
}

/// A model a [`Generator`] reaches.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogModel {
    /// What reaches it, by the name the person gave it.
    pub provider: String,
    /// Its own name.
    pub name: String,
    /// What it takes, where that is said.
    pub spec: Option<ModelSpec>,
    /// What reaches it, as a plan says it: a program and its arguments, or a server.
    pub by: String,
}

impl Catalog {
    /// The model `name` names: one by that name, where only one provider has one, or the one
    /// `<provider>/<name>` names. Said, where there is none, with the ones there are, and where
    /// providers share the name, with how each is named.
    pub fn find(&self, name: &str) -> Result<&CatalogModel> {
        if let Some((provider, own)) = name.split_once('/')
            && let Some(model) = self
                .models
                .iter()
                .find(|model| model.provider == provider && model.name == own)
        {
            return Ok(model);
        }
        let mut named = self.models.iter().filter(|model| model.name == name);
        match (named.next(), named.next()) {
            (Some(model), None) => Ok(model),
            (None, _) => Err(EngineError::NoModel {
                name: name.to_owned(),
                file: self.file.clone(),
                have: self.names(),
                note: self.note.clone(),
            }),
            (Some(_), Some(_)) => Err(EngineError::msg(format!(
                "model `{name}` is set up by more than one provider: name it as {}",
                self.models
                    .iter()
                    .filter(|model| model.name == name)
                    .map(|model| format!("`{}/{name}`", model.provider))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ))),
        }
    }

    /// What model `name` takes, where it is set up and says.
    pub fn spec(&self, name: &str) -> Option<&ModelSpec> {
        self.find(name).ok()?.spec.as_ref()
    }

    /// Each model by the name a line gives it: its own, or with its provider's
    /// (`<provider>/<name>`) where another provider has one by the same name.
    pub fn names(&self) -> Vec<String> {
        self.models
            .iter()
            .map(|model| self.name_of(model))
            .collect()
    }

    /// The name a line gives `model`.
    pub fn name_of(&self, model: &CatalogModel) -> String {
        let shared = self
            .models
            .iter()
            .filter(|other| other.name == model.name)
            .count()
            > 1;
        if shared {
            format!("{}/{}", model.provider, model.name)
        } else {
            model.name.clone()
        }
    }
}

/// The request a model is asked of what `ask` asks, with every file where it is:
/// `file` says where one of the project is, and `made` where one a step
/// made is, the one this writes among them. `signals` and `moves` are the ones it
/// names, read from the project, and `regions` what clips of its guide say they are.
pub(crate) fn request(
    ask: &Ask,
    file: &dyn Fn(&str) -> Result<PathBuf>,
    made: &dyn Fn(&str) -> PathBuf,
    signals: &[Signal],
    moves: &[Moves],
    regions: Vec<Region>,
) -> Result<GenerateRequest> {
    let place = |source: &Source| -> Result<String> {
        Ok(match source {
            Source::File(named) => file(named)?,
            Source::Made(name) => made(name),
        }
        .to_string_lossy()
        .into_owned())
    };
    let optional = |source: &Option<Source>| source.as_ref().map(place).transpose();
    Ok(GenerateRequest {
        version: REQUEST_VERSION,
        model: ask.model.clone(),
        prompt: ask.prompt.clone(),
        guide: optional(&ask.guide)?,
        guides: ask
            .guides
            .iter()
            .map(|(name, source)| Ok((name.clone(), place(source)?)))
            .collect::<Result<_>>()?,
        first: optional(&ask.first)?,
        last: optional(&ask.last)?,
        references: ask.references.iter().map(place).collect::<Result<_>>()?,
        seconds: ask.seconds,
        size: ask.size.map(|(width, height)| [width, height]),
        seed: ask.seed,
        signals: signals.iter().map(Signal::handed).collect(),
        moves: moves.iter().map(Moves::handed).collect(),
        regions,
        more: ask.more.clone(),
        out: made(&ask.out).to_string_lossy().into_owned(),
        from: ask.from,
    })
}
