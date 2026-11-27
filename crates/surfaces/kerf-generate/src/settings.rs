//! The file a person sets up the providers that reach their models in, `~/.kerf/generation.json`:
//!
//! ```json
//! { "providers": {
//!     "pod":    { "url": "https://example.trycloudflare.com" },
//!     "previs": { "run": ["vork", "render", "{ref}", "--out", "{out}"],
//!                 "takes": { "reads": ["ref"] } } } }
//! ```
//!
//! A provider with a `url` is a server that speaks the Kerf generation API, and says which models
//! it reaches; one with a `run` is a program on the person's computer, which is one model by the
//! provider's name ([`crate::command`]). No key is kept here: a server's key is in the system's
//! keychain, as `generation/<provider>`.

use kerf_generate_api::ModelSpec;
use serde_json::{Map, Value};
use std::path::Path;

use crate::command::{OUT, REQUEST};

/// What reaches a person's models, as the file says it.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderSettings {
    /// A server that speaks the Kerf generation API, at `url`.
    Endpoint { url: String },
    /// A program on the person's computer, and what it takes where that is said.
    Command {
        run: Vec<String>,
        takes: Option<ModelSpec>,
    },
}

/// The file, read: each provider by its name, and the file as written, which a change keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    providers: Vec<(String, ProviderSettings)>,
    written: Map<String, Value>,
}

/// Whether `name` can name a provider: letters, digits, `.`, `-` and `_`. Not `/`, which a line
/// puts between a provider's name and its model's.
pub fn names_a_provider(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || "._-".contains(letter))
}

impl Settings {
    /// The file at `file`: none is no providers. What is wrong in it is said with the provider.
    pub fn read(file: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(file) {
            Ok(text) => Self::parse(&text).map_err(|what| format!("{}: {what}", file.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("cannot read {}: {error}", file.display())),
        }
    }

    /// The file's text, read. What is wrong is said in words a person who wrote it follows.
    pub fn parse(text: &str) -> Result<Self, String> {
        let written: Map<String, Value> = serde_json::from_str(text)
            .map_err(|error| format!("it is not a JSON object: {error}"))?;
        let mut providers = Vec::new();
        match written.get("providers") {
            None | Some(Value::Null) => {}
            Some(Value::Object(named)) => {
                for (name, said) in named {
                    providers.push((name.clone(), provider(name, said)?));
                }
            }
            Some(_) => {
                return Err(
                    "`providers` is not an object of providers by their names, such as {\"pod\": {\"url\": \"https://…\"}}".to_owned(),
                );
            }
        }
        Ok(Self { providers, written })
    }

    /// Each provider by its name, in the order the file names them.
    pub fn providers(&self) -> impl Iterator<Item = (&str, &ProviderSettings)> {
        self.providers
            .iter()
            .map(|(name, provider)| (name.as_str(), provider))
    }

    /// Provider `name` set to a server at `url`, in place of any by that name.
    pub fn set_endpoint(&mut self, name: &str, url: &str) -> Result<(), String> {
        if !names_a_provider(name) {
            return Err(format!(
                "`{name}` cannot name a provider: letters, digits, `.`, `-` and `_`"
            ));
        }
        let url = endpoint_url(url)?;
        let named = self
            .written
            .entry("providers")
            .or_insert_with(|| Value::Object(Map::new()));
        if !named.is_object() {
            *named = Value::Object(Map::new());
        }
        let named = named.as_object_mut().expect("made an object above");
        named.insert(name.to_owned(), serde_json::json!({ "url": url }));
        *self = Self::parse(&Value::Object(self.written.clone()).to_string())?;
        Ok(())
    }

    /// Provider `name` taken away; taking one that is not there leaves the file as it is.
    pub fn remove(&mut self, name: &str) {
        if let Some(Value::Object(named)) = self.written.get_mut("providers") {
            named.remove(name);
        }
        self.providers.retain(|(said, _)| said != name);
    }

    /// The file's text: what was written, with any change.
    pub fn text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&Value::Object(self.written.clone()))
            .expect("JSON values serialize");
        text.push('\n');
        text
    }

    /// Write it to `file`, making its folder where there is none.
    pub fn write(&self, file: &Path) -> std::io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(file, self.text())
    }
}

/// A server's address as a provider holds it: `http` or `https`, with no `/` at its end.
pub fn endpoint_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.contains(char::is_whitespace)
    {
        return Err(format!(
            "`{url}` is not a server's address, such as https://example.trycloudflare.com"
        ));
    }
    Ok(url.to_owned())
}

/// Provider `name` as the file says it in `said`.
fn provider(name: &str, said: &Value) -> Result<ProviderSettings, String> {
    let wrong = |what: String| format!("provider `{name}`: {what}");
    if !names_a_provider(name) {
        return Err(wrong(
            "a provider is named by letters, digits, `.`, `-` and `_`".to_owned(),
        ));
    }
    let fields = said.as_object().ok_or_else(|| {
        wrong("it is not an object, such as {\"url\": \"https://…\"} or {\"run\": [\"<program>\", \"{request}\"]}".to_owned())
    })?;
    match (fields.get("url"), fields.get("run")) {
        (Some(_), Some(_)) => Err(wrong(
            "it has both a `url` and a `run`: a provider is a server or a program".to_owned(),
        )),
        (None, None) => Err(wrong(
            "it has neither a `url`, a server's address, nor a `run`, a program and its arguments"
                .to_owned(),
        )),
        (Some(url), None) => {
            let url = url
                .as_str()
                .ok_or_else(|| wrong("`url` is not text".to_owned()))?;
            Ok(ProviderSettings::Endpoint {
                url: endpoint_url(url).map_err(wrong)?,
            })
        }
        (None, Some(run)) => {
            let run: Vec<String> = serde_json::from_value(run.clone())
                .ok()
                .filter(|run: &Vec<String>| !run.is_empty())
                .ok_or_else(|| {
                    wrong("`run` is not a program and its arguments, as a list of words".to_owned())
                })?;
            if !run
                .iter()
                .any(|word| word.contains(REQUEST) || word.contains(OUT))
            {
                return Err(wrong(format!(
                    "`run` names neither `{REQUEST}` nor `{OUT}`, so its program is never told what is asked or where its file goes"
                )));
            }
            let takes = match fields.get("takes") {
                None | Some(Value::Null) => None,
                Some(takes) => Some(ModelSpec::parse(takes).map_err(wrong)?),
            };
            Ok(ProviderSettings::Command { run, takes })
        }
    }
}
