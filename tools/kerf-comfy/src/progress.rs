//! How far along a job is, as the server says it while it runs.
//!
//! ComfyUI says which step of how many a sampler is on over a websocket
//! (`/ws?clientId=…`) and nowhere else: its HTTP answers say only that a job
//! is waiting, running, or done. So beside asking after the job, this
//! listens there for the jobs queued under its own client id:
//!
//! ```json
//! {"type": "progress", "data": {"value": 7, "max": 20, "prompt_id": "…", "node": "12"}}
//! ```
//!
//! It is something to say sooner, not something a making stands on. Where
//! the socket does not open, a proxy that passes no upgrade or a server
//! that has none, or closes midway, a job is asked after as before and
//! what is said is how long it has run.

use serde_json::Value;
use std::sync::{Arc, Mutex};

/// How long the socket is given to open.
const REACH: std::time::Duration = std::time::Duration::from_secs(5);

/// The step a job is on, of how many.
type Step = (u64, u64);

/// What the server has last said of each job, by its id.
#[derive(Clone, Default)]
pub struct Progress {
    steps: Arc<Mutex<std::collections::HashMap<String, Step>>>,
}

impl Progress {
    /// The step job `id` was last said to be on.
    pub fn step(&self, id: &str) -> Option<Step> {
        self.steps.lock().ok()?.get(id).copied()
    }

    /// Take in one thing the server said: only what says a step of a job is kept.
    pub fn heard(&self, said: &str) {
        let Ok(said) = serde_json::from_str::<Value>(said) else {
            return;
        };
        if said["type"] != "progress" {
            return;
        }
        let data = &said["data"];
        let (Some(id), Some(value), Some(max)) =
            (data["prompt_id"].as_str(), data["value"].as_u64(), data["max"].as_u64())
        else {
            return;
        };
        if let Ok(mut steps) = self.steps.lock() {
            steps.insert(id.to_owned(), (value, max.max(1)));
        }
    }
}

/// Where the server's websocket is, for the server at `url`, listening as `client`.
pub fn address(url: &str, client: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| url.strip_prefix("http://").map(|rest| format!("ws://{rest}")))?;
    Some(format!("{}/ws?clientId={client}", rest.trim_end_matches('/')))
}

/// Listen to the server at `url` as `client`, with `key` as the header it asks for, on a
/// thread of its own, and give what it says of jobs' steps. `None` where the socket does
/// not open: the job is then asked after without it.
pub fn follow(url: &str, key: Option<(&str, &str)>, client: &str) -> Option<Progress> {
    use tungstenite::client::IntoClientRequest;
    let mut request = address(url, client)?.into_client_request().ok()?;
    if let Some((name, value)) = key {
        request.headers_mut().insert(
            tungstenite::http::HeaderName::from_bytes(name.as_bytes()).ok()?,
            tungstenite::http::HeaderValue::from_str(value).ok()?,
        );
    }
    // Reached within a few seconds or not at all: a making does not wait on this.
    let uri = request.uri().clone();
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") { 443 } else { 80 });
    let stream = {
        use std::net::ToSocketAddrs;
        let mut found = (uri.host()?, port).to_socket_addrs().ok()?;
        std::net::TcpStream::connect_timeout(&found.next()?, REACH).ok()?
    };
    stream.set_read_timeout(Some(REACH)).ok()?;
    stream.set_write_timeout(Some(REACH)).ok()?;
    let (mut socket, _) = tungstenite::client_tls(request, stream).ok()?;
    // Once it is open, what it says comes when it comes.
    let opened = match socket.get_ref() {
        tungstenite::stream::MaybeTlsStream::Plain(stream) => stream.set_read_timeout(None),
        tungstenite::stream::MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(None),
        _ => Ok(()),
    };
    opened.ok()?;
    let progress = Progress::default();
    let heard = progress.clone();
    std::thread::spawn(move || {
        // Until the server closes it, or this program ends: nothing waits on this thread.
        while let Ok(message) = socket.read() {
            match message {
                tungstenite::Message::Text(said) => heard.heard(said.as_str()),
                tungstenite::Message::Close(_) => break,
                // A preview of the picture so far, a ping: not said here.
                _ => {}
            }
        }
    });
    Some(progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_that_opens_is_listened_to_and_one_that_does_not_is_done_without() {
        // A server that takes the upgrade, says two things and closes.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            // What it was asked, read where it lies before the upgrade takes it.
            let mut asked = [0_u8; 1024];
            let read = stream.peek(&mut asked).unwrap();
            let asked = String::from_utf8_lossy(&asked[..read]).to_lowercase();
            let asked = format!(
                "{} {}",
                asked.lines().next().unwrap_or_default(),
                asked.lines().find(|line| line.starts_with("x-api-key:")).unwrap_or_default()
            );
            let mut socket = tungstenite::accept(stream).unwrap();
            socket
                .send(tungstenite::Message::text(r#"{"type": "progress", "data": {"value": 3, "max": 20, "prompt_id": "job-1"}}"#))
                .unwrap();
            let _ = socket.close(None);
            let _ = socket.flush();
            asked
        });
        let progress = follow(&format!("http://127.0.0.1:{port}"), Some(("X-API-Key", "secret")), "kerf-9").unwrap();
        assert_eq!(served.join().unwrap(), "get /ws?clientid=kerf-9 http/1.1 x-api-key: secret");
        // What it said is there once it is read.
        let waited = std::time::Instant::now();
        while progress.step("job-1").is_none() && waited.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(progress.step("job-1"), Some((3, 20)));
        // Nothing listening there: no progress, and no error either.
        assert!(follow(&format!("http://127.0.0.1:{port}"), None, "kerf-9").is_none());
    }
}
