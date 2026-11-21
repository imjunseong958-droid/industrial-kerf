//! A ComfyUI server: where it is, and what is asked of it over HTTP.
//!
//! A file goes up with `POST /upload/image`, a graph is queued with
//! `POST /prompt`, `GET /history/<id>` has the job once it has ended and
//! `GET /queue` says where it is until then, and `GET /view` hands a file it
//! made back. Nothing here is a model's: the graph says which one runs.

use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The header a key is sent in unless another is named.
const KEY_HEADER: &str = "X-API-Key";

/// Where a server is and what it asks to be let in with.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Address {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub key: Option<String>,
    /// The header the key is sent in; `Authorization` sends it as a bearer
    /// token.
    #[serde(default)]
    pub header: Option<String>,
}

impl Address {
    /// The server a person has named: `COMFY_URL`, `COMFY_API_KEY`, and
    /// `COMFY_API_HEADER` where they are set, and otherwise what the file
    /// `KERF_COMFY` names, or `~/.kerf/comfy.json`, says under `url`, `key`,
    /// and `header`. An editor is not started from a shell, so the file is
    /// what reaches a making run from one.
    pub fn load() -> Result<Self, String> {
        let variable = |name: &str| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        let file = variable("KERF_COMFY").map(PathBuf::from).or_else(|| {
            // Windows sets no HOME.
            variable("HOME")
                .or_else(|| variable("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".kerf").join("comfy.json"))
        });
        let kept = match &file {
            Some(file) => match std::fs::read_to_string(file) {
                Ok(text) => Some((file.clone(), text)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("cannot read {}: {error}", file.display())),
            },
            None => None,
        };
        Self::of(
            &variable,
            kept.as_ref()
                .map(|(file, text)| (file.as_path(), text.as_str())),
        )
    }

    /// The server `variable` and the kept file's text name between them,
    /// a variable first.
    fn of(
        variable: &dyn Fn(&str) -> Option<String>,
        kept: Option<(&Path, &str)>,
    ) -> Result<Self, String> {
        let mut address = match kept {
            Some((file, text)) => serde_json::from_str::<Self>(text)
                .map_err(|error| format!("{}: {error}", file.display()))?,
            None => Self::default(),
        };
        if let Some(url) = variable("COMFY_URL") {
            address.url = url;
        }
        address.key = variable("COMFY_API_KEY").or(address.key);
        address.header = variable("COMFY_API_HEADER").or(address.header);
        address.url = address.url.trim().trim_end_matches('/').to_owned();
        if address.url.is_empty() {
            return Err(
                "no ComfyUI server is named: set COMFY_URL (and COMFY_API_KEY if it asks for a key), or write {\"url\": \"…\", \"key\": \"…\"} in ~/.kerf/comfy.json"
                    .to_owned(),
            );
        }
        Ok(address)
    }

    /// The header that carries the key, and what it holds.
    fn key_header(&self) -> Option<(String, String)> {
        let key = self.key.as_deref()?;
        let name = self.header.as_deref().unwrap_or(KEY_HEADER);
        Some(if name.eq_ignore_ascii_case("authorization") {
            (name.to_owned(), format!("Bearer {key}"))
        } else {
            (name.to_owned(), key.to_owned())
        })
    }
}

/// A file a job made, as the server names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    pub filename: String,
    pub subfolder: String,
    /// Saved to show and not to keep, among the server's temporary files: what a node that
    /// previews writes. It is fetched where nothing of its kind was saved to keep.
    pub shown: bool,
}

/// Where a job is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Queued, behind this many.
    Waiting(usize),
    Running,
    /// Ended well, having made these.
    Made(Vec<Made>),
    /// Ended badly, with why.
    Failed(String),
    /// Neither queued nor ended: the server does not know it.
    Unknown,
}

/// What a kind of node gives is said under, among what its inputs take: no input has the name.
pub const OUTPUTS: &str = "$outputs";

/// The kinds of input that are a value written on the node, and not what another node hands it.
const PLAIN: [&str; 5] = ["INT", "FLOAT", "STRING", "BOOLEAN", "COMBO"];

/// What a node's inputs take, from the `input` the server says of its kind: `required`
/// and `optional`, each input a kind and what is said of it.
pub fn taken(input: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    let mut taken = serde_json::Map::new();
    for group in ["required", "optional"] {
        for (name, said) in input
            .get(group)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let (kind, more) = (&said[0], &said[1]);
            // A list of values to choose among, as a list or as a COMBO with its options.
            let choices = kind.as_array().or_else(|| {
                (kind == "COMBO")
                    .then(|| more["options"].as_array())
                    .flatten()
            });
            if let Some(choices) = choices {
                taken.insert(name.clone(), serde_json::json!({"choices": choices}));
            } else if let Some(linked) = kind.as_str().filter(|kind| !PLAIN.contains(kind)) {
                // What another node hands it: a model, a latent, pictures.
                taken.insert(name.clone(), serde_json::json!({"type": linked}));
            } else if kind == "INT" || kind == "FLOAT" {
                let mut range = serde_json::Map::new();
                for bound in ["min", "max", "step"] {
                    if let Some(value) = more.get(bound).filter(|value| value.is_number()) {
                        range.insert(bound.to_owned(), value.clone());
                    }
                }
                if !range.is_empty() {
                    taken.insert(name.clone(), Value::Object(range));
                }
            }
        }
    }
    taken
}

/// A server and the agent that talks to it.
pub struct Server {
    agent: ureq::Agent,
    address: Address,
    /// What this run is to the server: its jobs are queued under it, and what the server
    /// says of them while they run is said to it.
    client: String,
}

impl Server {
    pub fn new(address: Address) -> Self {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(30)))
            // A guide going up or a shot coming down a slow line.
            .timeout_global(Some(Duration::from_secs(900)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        let client = format!(
            "kerf-comfy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_millis())
        );
        Self {
            agent,
            address,
            client,
        }
    }

    /// What the server says of this run's jobs while they run, where it can be listened to.
    pub fn progress(&self) -> Option<crate::progress::Progress> {
        let key = self.address.key_header();
        crate::progress::follow(
            self.url(),
            key.as_ref()
                .map(|(name, value)| (name.as_str(), value.as_str())),
            &self.client,
        )
    }

    /// The server's address, as a person reads it.
    pub fn url(&self) -> &str {
        &self.address.url
    }

    fn keyed<Any>(&self, request: ureq::RequestBuilder<Any>) -> ureq::RequestBuilder<Any> {
        match self.address.key_header() {
            Some((name, value)) => request.header(name, value),
            None => request,
        }
    }

    /// What came of `asked`, `what` being what was asked for: the answer
    /// when the server gave one, whatever its status but a refusal.
    fn answered(
        &self,
        what: &str,
        asked: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let answer =
            asked.map_err(|error| format!("could not {what} at {}: {error}", self.url()))?;
        match answer.status().as_u16() {
            401 | 403 => Err(match self.address.key_header() {
                Some((name, _)) => format!(
                    "{} refused the key (HTTP {}). It was sent as the header `{name}`; COMFY_API_HEADER, or `header` in comfy.json, names another",
                    self.url(),
                    answer.status().as_u16()
                ),
                None => format!(
                    "{} asks for a key (HTTP {}): set COMFY_API_KEY, or `key` in comfy.json",
                    self.url(),
                    answer.status().as_u16()
                ),
            }),
            _ => Ok(answer),
        }
    }

    fn get(&self, what: &str, path: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
        let asked = self
            .keyed(self.agent.get(format!("{}{path}", self.url())))
            .call();
        self.answered(what, asked)
    }

    /// The JSON an answer holds, or what it said in place of it.
    fn json(what: &str, mut answer: ureq::http::Response<ureq::Body>) -> Result<Value, String> {
        let status = answer.status().as_u16();
        let text = answer
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|error| format!("could not {what}: the answer broke off: {error}"))?;
        serde_json::from_str(&text).map_err(|_| {
            let said: String = text.chars().take(200).collect();
            format!(
                "could not {what}: the server answered HTTP {status} with what is not JSON: {}",
                said.trim()
            )
        })
    }

    /// Send `file` up as `name`, and say what the server calls it.
    pub fn upload(&self, file: &Path, name: &str) -> Result<String, String> {
        let bytes = std::fs::read(file)
            .map_err(|error| format!("cannot read {}: {error}", file.display()))?;
        let (kind, body) = form(name, &bytes);
        let asked = self
            .keyed(self.agent.post(format!("{}/upload/image", self.url())))
            .header("Content-Type", kind)
            .send(&body[..]);
        let answer = self.answered("send a file", asked)?;
        let status = answer.status().as_u16();
        let said = Self::json("send a file", answer)?;
        let named = said["name"].as_str().ok_or_else(|| {
            format!(
                "the server did not take {} (HTTP {status}): {said}",
                file.display()
            )
        })?;
        Ok(match said["subfolder"].as_str() {
            Some(folder) if !folder.is_empty() => format!("{folder}/{named}"),
            _ => named.to_owned(),
        })
    }

    /// The inputs a node of kind `class` cannot do without.
    pub fn required(&self, class: &str) -> Result<BTreeSet<String>, String> {
        let what = format!("ask what a {class} node needs");
        let said = Self::json(
            &what,
            self.get(&what, &format!("/object_info/{}", encoded(class)))?,
        )?;
        let required = said[class]["input"]["required"]
            .as_object()
            .ok_or_else(|| format!("the server has no node `{class}`, which the workflow uses"))?;
        Ok(required.keys().cloned().collect())
    }

    /// What each input of a node of kind `class` takes, as the server says it: the values
    /// to choose among (`choices`), or for a number the least and the most it takes and
    /// the step between (`min`, `max`, `step`). An input the server says neither of, a
    /// link among them, is left out.
    pub fn inputs(&self, class: &str) -> Result<Value, String> {
        let what = format!("ask what a {class} node takes");
        let said = Self::json(
            &what,
            self.get(&what, &format!("/object_info/{}", encoded(class)))?,
        )?;
        let node = said[class]["input"]
            .as_object()
            .ok_or_else(|| format!("the server has no node `{class}`, which the workflow uses"))?;
        let mut taken = taken(node);
        // What a node of the kind gives at each output, by its place: what a link to it reads.
        if let Some(outputs) = said[class]["output"].as_array() {
            taken.insert(OUTPUTS.to_owned(), Value::Array(outputs.clone()));
        }
        Ok(Value::Object(taken))
    }

    /// Queue `graph`, and say what the server calls the job.
    pub fn queue(&self, graph: &Value) -> Result<String, String> {
        let asked = self
            .keyed(self.agent.post(format!("{}/prompt", self.url())))
            .send_json(json!({"prompt": graph, "client_id": self.client}));
        let said = Self::json("queue the job", self.answered("queue the job", asked)?)?;
        match said["prompt_id"].as_str() {
            Some(id) => Ok(id.to_owned()),
            None => Err(format!(
                "the server refused the workflow: {}",
                refusal(&said)
            )),
        }
    }

    /// Where job `id` is.
    pub fn state(&self, id: &str) -> Result<State, String> {
        let ended = Self::json(
            "ask after the job",
            self.get("ask after the job", &format!("/history/{}", encoded(id)))?,
        )?;
        if let Some(job) = ended.get(id) {
            return Ok(ending(job));
        }
        let queue = Self::json(
            "ask after the queue",
            self.get("ask after the queue", "/queue")?,
        )?;
        let ids = |list: &str| -> Vec<String> {
            queue[list]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|job| {
                    job.get(1)
                        .or_else(|| job.get("prompt_id"))?
                        .as_str()
                        .map(str::to_owned)
                })
                .collect()
        };
        let (running, pending) = (ids("queue_running"), ids("queue_pending"));
        Ok(if running.iter().any(|job| job == id) {
            State::Running
        } else if let Some(place) = pending.iter().position(|job| job == id) {
            State::Waiting(place + running.len())
        } else {
            State::Unknown
        })
    }

    /// Write the file `made` names to `to`.
    pub fn fetch(&self, made: &Made, to: &Path) -> Result<(), String> {
        let path = format!(
            "/view?filename={}&subfolder={}&type={}",
            encoded(&made.filename),
            encoded(&made.subfolder),
            if made.shown { "temp" } else { "output" }
        );
        let mut answer = self.get("fetch what was made", &path)?;
        if answer.status().as_u16() != 200 {
            return Err(format!(
                "the server would not hand over {} (HTTP {})",
                made.filename,
                answer.status().as_u16()
            ));
        }
        let mut file = std::fs::File::create(to)
            .map_err(|error| format!("cannot write {}: {error}", to.display()))?;
        let mut body = answer.body_mut().as_reader();
        let mut chunk = vec![0; 256 * 1024];
        loop {
            let read = body
                .read(&mut chunk)
                .map_err(|error| format!("{} broke off on the way down: {error}", made.filename))?;
            if read == 0 {
                return Ok(());
            }
            std::io::Write::write_all(&mut file, &chunk[..read])
                .map_err(|error| format!("cannot write {}: {error}", to.display()))?;
        }
    }

    /// Take job `id` out of the queue, or stop it if it is the one running.
    /// A server that does not answer is left alone.
    pub fn cancel(&self, id: &str) {
        for (path, body) in [
            ("/queue", json!({"delete": [id]})),
            ("/interrupt", json!({"prompt_id": id})),
        ] {
            let _ = self
                .keyed(self.agent.post(format!("{}{path}", self.url())))
                .send_json(body);
        }
    }
}

/// How a job that has ended ended, from what the server keeps of it.
fn ending(job: &Value) -> State {
    let status = &job["status"];
    let message = |kind: &str| {
        status["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|message| message[0] == kind)
            .map(|message| message[1].clone())
    };
    if let Some(error) = message("execution_error") {
        return State::Failed(format!(
            "{} (node {}): {}",
            error["node_type"].as_str().unwrap_or("a node"),
            error["node_id"].as_str().unwrap_or("?"),
            error["exception_message"]
                .as_str()
                .unwrap_or("it gave no reason")
                .trim()
        ));
    }
    if message("execution_interrupted").is_some() {
        return State::Failed("it was stopped on the server".to_owned());
    }
    if status["status_str"] == "error" {
        return State::Failed("the server gave no reason".to_owned());
    }
    // A node that loads a file says so among the outputs, as an input. What was saved to keep
    // comes before what was saved to show.
    let mut made: Vec<Made> = job["outputs"]
        .as_object()
        .into_iter()
        .flat_map(|outputs| outputs.values())
        .filter_map(Value::as_object)
        .flat_map(|output| output.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter(|file| file["type"] == "output" || file["type"] == "temp")
        .filter_map(|file| {
            Some(Made {
                filename: file["filename"].as_str()?.to_owned(),
                subfolder: file["subfolder"].as_str().unwrap_or_default().to_owned(),
                shown: file["type"] == "temp",
            })
        })
        .collect();
    made.sort_by_key(|file| file.shown);
    State::Made(made)
}

/// Why a server refused a graph, a line a node.
fn refusal(said: &Value) -> String {
    let mut lines = vec![said["error"]["message"]
        .as_str()
        .unwrap_or("it gave no reason")
        .to_owned()];
    for (id, node) in said["node_errors"].as_object().into_iter().flatten() {
        for error in node["errors"].as_array().into_iter().flatten() {
            lines.push(format!(
                "node {id} ({}): {} {}",
                node["class_type"].as_str().unwrap_or("?"),
                error["message"].as_str().unwrap_or_default(),
                error["details"].as_str().unwrap_or_default()
            ));
        }
    }
    lines.join("; ")
}

/// A form that carries `bytes` as the file `name`, to be kept among the
/// server's inputs over one of that name: its content type and its body.
fn form(name: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    // No file holds its own hash, so this line is in none.
    let boundary = format!("kerf-comfy-{}", crate::hash(bytes));
    let mut body = Vec::with_capacity(bytes.len() + 512);
    for (field, value) in [("type", "input"), ("overwrite", "true")] {
        body.extend(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n").bytes(),
        );
    }
    body.extend(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .bytes(),
    );
    body.extend(bytes);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// `text` as part of a URL: letters, digits, and `-._~` as they are, and
/// every other byte as `%XX`.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
