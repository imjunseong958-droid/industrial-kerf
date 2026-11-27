//! A server that speaks the Kerf generation API (`{"url": "https://…"}`): its models, asked of it
//! and kept in Kerf's cache folder so they are known when it cannot be reached, and a request,
//! sent with the files it names and followed until its file is made.

use kerf_engine::error::{EngineError, Result};
use kerf_engine::recipe::model::Generation;
use kerf_generate_api::{
    path, ApiError, Created, GenerateRequest, GenerationState, ModelList, NamedModel, Sha256,
    AUTHORIZATION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256 as Hasher};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The environment variable a key is read from where the keychain has none for a provider.
pub const KEY_ENV: &str = "KERF_GENERATION_KEY";

/// How long a server's models, once asked, are taken as they are without asking again.
const FRESH: Duration = Duration::from_secs(60);

/// How long a server is given to say which models it reaches.
const LISTING: Duration = Duration::from_secs(5);

/// How often a request under way is asked how it is going.
const POLL: Duration = Duration::from_millis(500);

/// Keys a host gives this process for providers, by the provider's name: where the keychain
/// cannot keep one, the editor keeps it in its own settings and hands it here.
static GIVEN: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// The name a provider's key is kept under in the system's keychain.
pub fn secret_name(provider: &str) -> String {
    format!("generation/{provider}")
}

/// Use `key` for provider `provider` from here on where the keychain has none, or forget the
/// one given (`None`): for a host that keeps keys where the keychain cannot.
pub fn give_key(provider: &str, key: Option<&str>) {
    let mut given = GIVEN.write().unwrap_or_else(PoisonError::into_inner);
    match key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => given.insert(provider.to_owned(), key.to_owned()),
        None => given.remove(provider),
    };
}

/// The key for provider `provider`: the system keychain's, else one the host gave, else
/// [`KEY_ENV`]'s.
pub fn key(provider: &str) -> Option<String> {
    use kerf_secrets::Keychain;
    let kept = kerf_secrets::SystemKeychain::new()
        .get(&secret_name(provider))
        .ok()
        .flatten();
    kept.or_else(|| {
        GIVEN
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(provider)
            .cloned()
    })
    .or_else(|| std::env::var(KEY_ENV).ok())
    .map(|key| key.trim().to_owned())
    .filter(|key| !key.is_empty())
}

/// An agent that hands back every answer, refusals included, and gives up on a server that
/// does not answer within `within` (none for a file of any size).
fn agent(within: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(within)
        .build()
        .new_agent()
}

/// Why a server refused, from its answer: what its [`ApiError`] says, or its status.
fn refused(url: &str, response: &mut ureq::http::Response<ureq::Body>) -> String {
    let status = response.status().as_u16();
    if matches!(status, 401 | 403) {
        return format!(
            "{url} did not take the key: set it in Settings ▸ Generation, or in {KEY_ENV}"
        );
    }
    let said = response
        .body_mut()
        .read_json::<ApiError>()
        .map(|error| error.error)
        .unwrap_or_else(|_| "no reason given".to_owned());
    format!("{url} answered {status}: {said}")
}

/// An error of a server's, as the engine says it.
fn failed(what: impl Into<String>) -> EngineError {
    EngineError::msg(what.into())
}

/// The models the server at `url` reaches, asked of it now with `key`.
pub fn models(url: &str, key: Option<&str>) -> std::result::Result<Vec<NamedModel>, String> {
    let mut asked = agent(Some(LISTING)).get(format!("{url}{}", path::MODELS));
    if let Some(key) = key {
        asked = asked.header(AUTHORIZATION, format!("Bearer {key}"));
    }
    let mut response = asked
        .call()
        .map_err(|error| format!("cannot reach {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(refused(url, &mut response));
    }
    response
        .body_mut()
        .read_json::<ModelList>()
        .map(|list| list.models)
        .map_err(|error| format!("{url} said its models in a way Kerf does not read: {error}"))
}

/// The folder a server's models are kept in once asked: `generation` in Kerf's cache folder
/// ([`kerf_paths::cache_dir`]), or `KERF_GENERATION_CACHE`, which `off` turns into none.
fn cache_folder() -> Option<PathBuf> {
    match std::env::var_os("KERF_GENERATION_CACHE") {
        Some(value) if value == "off" || value == "0" || value.is_empty() => return None,
        Some(value) => return Some(PathBuf::from(value)),
        None => {}
    }
    kerf_paths::cache_dir().map(|folder| folder.join("generation"))
}

/// A server's models as the cache keeps them, and why it could not be asked, where it could not
/// be the last time.
#[derive(Serialize, Deserialize)]
struct Kept {
    url: String,
    /// The SHA-256 of the key it was asked with, so a key changed since is asked with again.
    key: String,
    /// When it was asked, in seconds since 1970.
    asked: u64,
    models: Vec<NamedModel>,
    #[serde(default)]
    unreached: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The models provider `provider`, the server at `url`, reaches, and why it could not be asked,
/// where it could not: as the cache keeps them where the server was asked lately with the same
/// key, so that one that cannot be reached is not waited on each time; else asked of it now,
/// with the models the cache last kept where it cannot be.
pub(crate) fn listed(provider: &str, url: &str) -> (Vec<NamedModel>, Option<String>) {
    let file = cache_folder().map(|folder| folder.join(format!("{provider}.json")));
    let key = key(provider);
    let key_said = hex::encode(Hasher::digest(key.as_deref().unwrap_or_default()));
    let kept = file
        .as_ref()
        .and_then(|file| std::fs::read(file).ok())
        .and_then(|bytes| serde_json::from_slice::<Kept>(&bytes).ok())
        .filter(|kept| kept.url == url);
    if let Some(kept) = &kept
        && kept.key == key_said
        && now().saturating_sub(kept.asked) < FRESH.as_secs()
    {
        return (kept.models.clone(), kept.unreached.clone());
    }
    let (models, unreached) = match models(url, key.as_deref()) {
        Ok(models) => (models, None),
        Err(why) => (kept.map(|kept| kept.models).unwrap_or_default(), Some(why)),
    };
    if let Some(file) = &file {
        let keep = Kept {
            url: url.to_owned(),
            key: key_said,
            asked: now(),
            models: models.clone(),
            unreached: unreached.clone(),
        };
        let written = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::write(file, serde_json::to_vec(&keep).expect("JSON serializes"))
            });
        if let Err(error) = written {
            log::warn!("the models of `{provider}` could not be kept: {error}");
        }
    }
    (models, unreached)
}

/// The SHA-256 of the file at `file`.
fn sha256_of(file: &Path) -> Result<Sha256> {
    let mut hasher = Hasher::new();
    let mut reading = std::fs::File::open(file)
        .map_err(|error| failed(format!("cannot read {}: {error}", file.display())))?;
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = reading.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Sha256::parse(&hex::encode(hasher.finalize())).expect("a SHA-256 is 64 hex digits"))
}

/// A request under way at a server.
struct Asking<'a> {
    url: &'a str,
    key: String,
    agent: ureq::Agent,
}

impl Asking<'_> {
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.key)
    }

    /// Send the file at `file` where the server has it not, and say it by its SHA-256.
    fn send(&self, file: &Path) -> Result<Sha256> {
        let sha256 = sha256_of(file)?;
        let at = self.at(&path::file(&sha256));
        let mut has = self
            .agent
            .head(&at)
            .header(AUTHORIZATION, self.bearer())
            .call()
            .map_err(|error| failed(format!("cannot reach {}: {error}", self.url)))?;
        match has.status().as_u16() {
            200 => return Ok(sha256),
            404 => {}
            _ => return Err(failed(refused(self.url, &mut has))),
        }
        let body = std::fs::File::open(file)?;
        let mut sent = self
            .agent
            .put(&at)
            .header(AUTHORIZATION, self.bearer())
            .header("Content-Type", "application/octet-stream")
            .send(body)
            .map_err(|error| {
                failed(format!(
                    "cannot send {} to {}: {error}",
                    file.display(),
                    self.url
                ))
            })?;
        if !sent.status().is_success() {
            return Err(failed(refused(self.url, &mut sent)));
        }
        Ok(sha256)
    }

    /// How request `id` is going.
    fn state(&self, id: &str) -> Result<GenerationState> {
        let mut answer = self
            .agent
            .get(self.at(&path::generation(id)))
            .header(AUTHORIZATION, self.bearer())
            .call()
            .map_err(|error| failed(format!("cannot reach {}: {error}", self.url)))?;
        if !answer.status().is_success() {
            return Err(failed(refused(self.url, &mut answer)));
        }
        answer
            .body_mut()
            .read_json::<GenerationState>()
            .map_err(|error| {
                failed(format!(
                    "{} said how it is going in a way Kerf does not read: {error}",
                    self.url
                ))
            })
    }

    /// Ask the server to stop request `id`. Best effort: a making that is cancelled is over
    /// whatever the server says.
    fn cancel(&self, id: &str) {
        let asked = self
            .agent
            .post(self.at(&path::cancel(id)))
            .header(AUTHORIZATION, self.bearer())
            .send_empty();
        if let Err(error) = asked {
            log::warn!("{} was not told to stop {id}: {error}", self.url);
        }
    }

    /// The file `sha256`, written to `out` once it is whole and what was asked for.
    fn fetch(&self, sha256: &Sha256, out: &Path) -> Result<()> {
        let mut answer = self
            .agent
            .get(self.at(&path::file(sha256)))
            .header(AUTHORIZATION, self.bearer())
            .call()
            .map_err(|error| failed(format!("cannot reach {}: {error}", self.url)))?;
        if !answer.status().is_success() {
            return Err(failed(refused(self.url, &mut answer)));
        }
        let mut reading = answer.into_body().into_reader();
        let mut writing = std::fs::File::create(out)?;
        let mut hasher = Hasher::new();
        let mut buffer = vec![0; 1 << 16];
        loop {
            let read = reading.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writing.write_all(&buffer[..read])?;
        }
        writing.flush()?;
        if hex::encode(hasher.finalize()) != sha256.as_str() {
            let _ = std::fs::remove_file(out);
            return Err(failed(format!(
                "the file {} sent is not the one it made: it came cut short, or changed on the way",
                self.url
            )));
        }
        Ok(())
    }
}

/// Ask provider `provider`, the server at `url`, for what `generation` asks of its model
/// `model`, by the server's own name, writing the file at the request's `out`.
pub(crate) fn generate(
    provider: &str,
    url: &str,
    model: &str,
    generation: &Generation,
    on_line: &mut dyn FnMut(&str),
) -> Result<()> {
    let key = key(provider).ok_or_else(|| {
        failed(format!(
            "provider `{provider}` has no key: set it in Settings ▸ Generation, or in {KEY_ENV}"
        ))
    })?;
    let asking = Asking {
        url,
        key,
        agent: agent(None),
    };
    let request: &GenerateRequest = generation.request;
    let out = Path::new(&request.out);
    let mut sent: BTreeMap<String, Sha256> = BTreeMap::new();
    let mut sending = |file: String| -> Result<Sha256> {
        if let Some(sha256) = sent.get(&file) {
            return Ok(sha256.clone());
        }
        if generation.cancellation.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        on_line(&format!(
            "sending {} to {provider}",
            Path::new(&file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        let sha256 = asking.send(Path::new(&file))?;
        sent.insert(file, sha256.clone());
        Ok(sha256)
    };
    let mut asked = request.clone().try_map(&mut sending)?;
    asked.model = model.to_owned();
    asked.out = out
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "out.mp4".to_owned());
    let mut created = asking
        .agent
        .post(asking.at(path::GENERATIONS))
        .header(AUTHORIZATION, asking.bearer())
        .send_json(&asked)
        .map_err(|error| failed(format!("cannot reach {url}: {error}")))?;
    if !created.status().is_success() {
        return Err(failed(refused(url, &mut created)));
    }
    let Created { id } = created.body_mut().read_json().map_err(|error| {
        failed(format!(
            "{url} did not say what it named the request: {error}"
        ))
    })?;
    let mut last = String::new();
    loop {
        if generation.cancellation.is_cancelled() {
            asking.cancel(&id);
            return Err(EngineError::Cancelled);
        }
        let said = match asking.state(&id)? {
            GenerationState::Queued => format!("waiting at {provider}"),
            GenerationState::Running { progress, line } => match (progress, line) {
                (Some(progress), Some(line)) => format!("{:.0}%: {line}", progress * 100.0),
                (Some(progress), None) => format!("{:.0}%", progress * 100.0),
                (None, Some(line)) => line,
                (None, None) => format!("making at {provider}"),
            },
            GenerationState::Done { out: made } => {
                on_line(&format!("fetching what {provider} made"));
                return asking.fetch(&made, out);
            }
            GenerationState::Failed { message } => {
                return Err(failed(format!(
                    "model `{model}` at {provider} could not make it: {message}"
                )));
            }
            GenerationState::Cancelled => {
                return Err(failed(format!("{provider} stopped the request {id}")));
            }
        };
        if said != last {
            on_line(&said);
            last = said;
        }
        // Asked to stop is heard within a tenth of a second, and the server is asked less often.
        for _ in 0..(POLL.as_millis() / 100) {
            if generation.cancellation.is_cancelled() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
