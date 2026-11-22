//! kerf:// project links: a project, a revision and a moment of it.
//!
//! ```text
//! kerf://<project>/rev<N>[@HH:MM:SS:FF][?branch=…&head=…&remote=…]
//! ```
//!
//! `<project>` is the slug of the project's folder name ([`slug`]): lower
//! case, every run of anything but letters and digits one `-`. A name
//! outside ASCII is a slug in its letters (NFC); a browser may write it in
//! its ASCII form (`xn--…`), which reads back to them. Following a link
//! opens the project and seeks to the timecode. A project this computer
//! does not know (none of the recent ones) is an error shown to the person,
//! never a guess. macOS hands a link to the running editor; elsewhere (and
//! from a shell) it comes as the editor's argument, and an editor started
//! for links alone hands them to the one already open through this
//! person's link inbox ([`hand_off`], [`listen`]).

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// The project's slug as written.
    pub project: String,
    pub revision: Option<u64>,
    /// "HH:MM:SS:FF" as written.
    pub timecode: Option<String>,
    pub branch: Option<String>,
}

/// Whether `text` is a kerf:// link at all. Any text may be asked about,
/// a folder named in any script among them.
pub fn is_link(text: &str) -> bool {
    text.as_bytes()
        .get(..7)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"kerf://"))
}

/// Reads a link; the error says why it is not one, in words for a person.
pub fn parse(text: &str) -> Result<Link, String> {
    if !is_link(text) {
        return Err("Not a kerf:// link".into());
    }
    // A fragment (`#…`, as a browser may add one) says nothing here.
    let rest = text[7..].split('#').next().unwrap_or_default();
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    let (project, place) = rest.split_once('/').unwrap_or((rest, ""));
    let project = unicode_host(&percent_decode(project)).to_lowercase();
    if project.is_empty() {
        return Err("The link names no project".into());
    }
    let mut link = Link {
        project,
        ..Link::default()
    };
    let place = place.trim_end_matches('/');
    if !place.is_empty() {
        let (revision, timecode) = match place.split_once('@') {
            Some((revision, timecode)) => (revision, Some(timecode)),
            None => (place, None),
        };
        let number = revision
            .strip_prefix("rev")
            .and_then(|number| number.parse::<u64>().ok())
            .ok_or_else(|| format!("“{revision}” is not a revision (rev and a number)"))?;
        link.revision = Some(number);
        if let Some(timecode) = timecode {
            let parts: Vec<&str> = timecode.split([':', ';']).collect();
            if parts.len() != 4
                || parts
                    .iter()
                    .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(format!("“{timecode}” is not a timecode (HH:MM:SS:FF)"));
            }
            link.timecode = Some(timecode.to_owned());
        }
    }
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some(("branch", value)) = pair.split_once('=') {
                link.branch = Some(percent_decode(value));
            }
        }
    }
    Ok(link)
}

/// The longest label a URL host takes (RFC 1035), in its ASCII form.
const LABEL_MOST: usize = 63;

/// The slug a project name or folder makes: "Atlas Cut" -> "atlas-cut".
/// Letters and digits as they are (lower case, NFC), every other run one
/// `-`; as a URL's host writes it back (its own mapping of a few letters,
/// "¼" as "1⁄4" and so "1-4"), and short enough for one label of it. A
/// slug is its own slug. Empty when the name has no letter or digit.
pub fn slug(name: &str) -> String {
    let mut slug = letters(name);
    // The host's mapping can bring what is no letter (a fraction's slash):
    // taken out again until nothing changes.
    for _ in 0..8 {
        let again = letters(&as_host(&slug));
        if again == slug {
            break;
        }
        slug = again;
    }
    slug
}

/// Letters and digits of `name`, lower case and NFC, each other run one `-`.
fn letters(name: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in name.to_lowercase().nfc() {
        if character.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(character);
        } else {
            gap = true;
        }
    }
    slug
}

/// `slug` as a URL's host writes it back, cut to what one label of it
/// holds in its ASCII form.
fn as_host(slug: &str) -> String {
    let mut slug = slug.to_owned();
    if slug.is_ascii() {
        slug.truncate(LABEL_MOST);
        return slug.trim_end_matches('-').to_owned();
    }
    while !slug.is_empty() {
        if let Ok(ascii) = idna::domain_to_ascii(&slug)
            && !ascii.is_empty()
            && ascii.len() <= LABEL_MOST
        {
            let (back, read) = idna::domain_to_unicode(&ascii);
            if read.is_ok() && !back.is_empty() {
                slug = back;
            }
            break;
        }
        slug.pop();
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    slug
}

/// A host's letters back from its ASCII form (`xn--…` labels, as a browser
/// writes a link to a project named outside ASCII); any other is as it is.
pub fn unicode_host(host: &str) -> String {
    if !host.to_ascii_lowercase().contains("xn--") {
        return host.to_owned();
    }
    match idna::domain_to_unicode(host) {
        (unicode, Ok(())) if !unicode.is_empty() => unicode,
        _ => host.to_owned(),
    }
}

/// The link to `revision` of the project in `directory`, at `seconds` at `fps`.
pub fn make(directory: &Path, revision: u64, seconds: Option<f64>, fps: f64) -> Option<String> {
    let name = directory.file_name()?.to_string_lossy();
    let slug = slug(&name);
    if slug.is_empty() {
        return None;
    }
    let mut link = format!("kerf://{slug}/rev{revision}");
    if let Some(seconds) = seconds {
        link.push('@');
        link.push_str(&crate::model::timecode(seconds, fps));
    }
    Some(link)
}

/// The folder among `candidates` whose slug is the link's; an error when
/// none is, or when two are.
pub fn resolve(link: &Link, candidates: &[PathBuf]) -> Result<PathBuf, String> {
    let wanted = slug(&link.project);
    let mut found: Vec<&PathBuf> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .file_name()
                .is_some_and(|name| slug(&name.to_string_lossy()) == wanted)
        })
        .collect();
    found.dedup();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "No project here is called “{}”. Open it once, and its links will work.",
            link.project
        )),
        _ => Err(format!(
            "More than one project here is called “{}”.",
            link.project
        )),
    }
}

/// Whether the project `resolve` found can be opened: its folder is still
/// there and holds its project.kerf. The error says so to the person.
pub fn openable(project: &Path) -> Result<(), String> {
    let name = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.display().to_string());
    if project.join("project.kerf").is_file() {
        Ok(())
    } else {
        Err(format!(
            "The project “{name}” is no longer at {}.",
            project.display()
        ))
    }
}

/// The seconds of the link's timecode at `fps`.
pub fn seconds(link: &Link, fps: f64) -> Option<f64> {
    let timecode = link.timecode.as_deref()?;
    let parts: Vec<f64> = timecode
        .split([':', ';'])
        .filter_map(|part| part.parse().ok())
        .collect();
    let [hours, minutes, secs, frames] = parts.as_slice() else {
        return None;
    };
    let fps = if fps > 0.0 { fps } else { 25.0 };
    Some(hours * 3600.0 + minutes * 60.0 + secs + frames / fps.round().max(1.0))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hex = |byte: u8| (byte as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[index + 1]) * 16 + hex(bytes[index + 2]));
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------- the link inbox

/// Where this person's editors hand links to one another, off macOS (where
/// the system hands a link to the running editor itself). The first editor
/// listens there ([`listen`]); one started for links alone gives them to it
/// and ends ([`hand_off`]), so the link opens in the editor already open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InboxAddress {
    /// A Unix domain socket in the person's own runtime folder (Linux):
    /// only their processes reach it.
    Socket(PathBuf),
    /// A port on this computer, with a token only the person's own files
    /// hold: the file at this path, in their settings folder, says both
    /// (Windows, which has no domain sockets std can use).
    Port(PathBuf),
}

/// The inbox for the id `id` (kept in the person's settings,
/// [`crate::prefs::Prefs::inbox_id`]): a socket in `runtime` (Linux's
/// `XDG_RUNTIME_DIR`), else, given `settings` (the settings folder), a
/// port named in a file there. None without either, or without an id:
/// not the shared temporary folder, where another user could be first.
pub fn inbox_address(
    id: &str,
    runtime: Option<&Path>,
    settings: Option<&Path>,
) -> Option<InboxAddress> {
    let id = id.trim();
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    let name = format!("industrial-kerf-links-{id}");
    if let Some(runtime) = runtime.filter(|runtime| runtime.is_dir()) {
        return Some(InboxAddress::Socket(runtime.join(name)));
    }
    settings.map(|settings| InboxAddress::Port(settings.join(name)))
}

/// The inbox on this computer for `id`: a socket in `XDG_RUNTIME_DIR` on
/// Linux and the other Unixes, a port on Windows. None on macOS.
pub fn inbox_here(id: &str) -> Option<InboxAddress> {
    if cfg!(target_os = "macos") {
        return None;
    }
    let settings =
        crate::prefs::Prefs::path().and_then(|path| path.parent().map(Path::to_path_buf));
    if cfg!(unix) {
        inbox_address(id, Some(&runtime_folder()?), None)
    } else {
        inbox_address(id, None, settings.as_deref())
    }
}

/// The person's own runtime folder: `XDG_RUNTIME_DIR`, else `/run/user/<uid>`,
/// else one made for them in the temporary folder; each only when it is
/// theirs and no one else's, so no one else can hand their editor links
/// through its inbox. None otherwise.
#[cfg(unix)]
fn runtime_folder() -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|runtime| !runtime.is_empty())
    {
        return Some(PathBuf::from(runtime));
    }
    let uid = std::fs::metadata("/proc/self")
        .ok()
        .or_else(|| std::fs::metadata(crate::prefs::home()?).ok())?
        .uid();
    let made = std::env::temp_dir().join(format!("industrial-kerf-runtime-{uid}"));
    if !made.exists()
        && let Err(error) = std::fs::DirBuilder::new().mode(0o700).create(&made)
    {
        log::warn!(
            "{}: the runtime folder could not be made: {error}",
            made.display()
        );
    }
    [PathBuf::from(format!("/run/user/{uid}")), made]
        .into_iter()
        .find(|folder| {
            std::fs::symlink_metadata(folder)
                .is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0)
        })
}

#[cfg(not(unix))]
fn runtime_folder() -> Option<PathBuf> {
    None
}

/// The most a message to the inbox may be.
const INBOX_MOST: usize = 64 * 1024;
/// What the inbox answers once it has the links.
const INBOX_TAKEN: &[u8] = b"ok\n";

/// What is said to the inbox for `links`: each link on a line, then an
/// empty line. None when none of them is a link on one line.
pub fn inbox_message(links: &[String]) -> Option<Vec<u8>> {
    let mut message = String::new();
    for link in links {
        let link = link.trim();
        if is_link(link) && !link.contains(['\n', '\r']) {
            message.push_str(link);
            message.push('\n');
        }
    }
    if message.is_empty() {
        return None;
    }
    message.push('\n');
    Some(message.into_bytes())
}

/// The links in what the inbox received, once it is whole (it ends in an
/// empty line); None until then. What is no link is dropped.
pub fn inbox_links(received: &[u8]) -> Option<Vec<String>> {
    let end = received.windows(2).position(|pair| pair == b"\n\n")?;
    Some(
        String::from_utf8_lossy(&received[..end])
            .lines()
            .map(str::trim)
            .filter(|line| is_link(line))
            .map(str::to_owned)
            .collect(),
    )
}

/// Gives `links` to the editor listening at `address`. True once it said
/// it has them; false when none answered within `timeout`, and the caller
/// opens them itself. Blocks: for a process with nothing else to do yet.
pub fn hand_off(address: &InboxAddress, links: &[String], timeout: Duration) -> bool {
    let Some(message) = inbox_message(links) else {
        return false;
    };
    let answered = match address {
        #[cfg(unix)]
        InboxAddress::Socket(path) => std::os::unix::net::UnixStream::connect(path)
            .ok()
            .and_then(|stream| say(stream, &message, timeout)),
        #[cfg(not(unix))]
        InboxAddress::Socket(_) => None,
        InboxAddress::Port(path) => read_port(path).and_then(|(port, token)| {
            let stream =
                std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout)
                    .ok()?;
            let mut said = format!("{token}\n").into_bytes();
            said.extend_from_slice(&message);
            say(stream, &said, timeout)
        }),
    };
    answered == Some(true)
}

/// Writes `message` on `stream` and reads the answer: Some(true) for "ok".
fn say<S: Read + Write + Timeouts>(
    mut stream: S,
    message: &[u8],
    timeout: Duration,
) -> Option<bool> {
    stream.timeouts(timeout);
    stream.write_all(message).ok()?;
    stream.flush().ok()?;
    let mut answer = Vec::new();
    let mut reader = std::io::BufReader::new(stream);
    reader.read_until(b'\n', &mut answer).ok()?;
    Some(answer == INBOX_TAKEN)
}

/// A stream whose reads and writes give up after a while.
trait Timeouts {
    fn timeouts(&mut self, timeout: Duration);
}

#[cfg(unix)]
impl Timeouts for std::os::unix::net::UnixStream {
    fn timeouts(&mut self, timeout: Duration) {
        // Refused only for a zero timeout, which no caller gives.
        let _ = self.set_read_timeout(Some(timeout));
        let _ = self.set_write_timeout(Some(timeout));
    }
}

impl Timeouts for std::net::TcpStream {
    fn timeouts(&mut self, timeout: Duration) {
        // Refused only for a zero timeout, which no caller gives.
        let _ = self.set_read_timeout(Some(timeout));
        let _ = self.set_write_timeout(Some(timeout));
    }
}

/// The port and token a port inbox's file names.
fn read_port(path: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    let (port, token) = text.trim().split_once(' ')?;
    Some((port.parse().ok()?, token.to_owned()))
}

/// The inbox this editor listens in, while it is kept: dropped, it is
/// taken away, so the next editor listens there.
pub struct Inbox {
    address: InboxAddress,
}

impl Drop for Inbox {
    fn drop(&mut self) {
        match &self.address {
            InboxAddress::Socket(path) | InboxAddress::Port(path) => {
                // One left behind is taken over by the next editor to listen.
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Listens at `address` for links handed over, giving each message's to
/// `take` (on a thread of the inbox's own). None when another editor
/// already answers there (it keeps the links) or the address cannot be
/// taken; what an editor that crashed left there is taken over.
pub fn listen(
    address: &InboxAddress,
    take: impl Fn(Vec<String>) + Send + Sync + 'static,
) -> Option<Inbox> {
    let take = std::sync::Arc::new(take);
    match address {
        #[cfg(unix)]
        InboxAddress::Socket(path) => {
            use std::os::unix::fs::PermissionsExt;
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return None;
            }
            // What an editor that crashed left, if anything: one that stays
            // has the bind below fail.
            let _ = std::fs::remove_file(path);
            let listener = std::os::unix::net::UnixListener::bind(path).ok()?;
            if let Err(error) =
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            {
                log::warn!(
                    "{}: the link inbox could not be kept private: {error}",
                    path.display()
                );
            }
            std::thread::Builder::new()
                .name("link-inbox".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let take = take.clone();
                        std::thread::spawn(move || {
                            if let Some(links) = receive(stream, None) {
                                take(links);
                            }
                        });
                    }
                })
                .ok()?;
        }
        #[cfg(not(unix))]
        InboxAddress::Socket(_) => return None,
        InboxAddress::Port(path) => {
            if let Some((port, _)) = read_port(path) {
                let other = std::net::TcpStream::connect_timeout(
                    &([127, 0, 0, 1], port).into(),
                    Duration::from_millis(250),
                );
                if other.is_ok() {
                    return None;
                }
            }
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
            let port = listener.local_addr().ok()?.port();
            let token = crate::prefs::random_id();
            if let Some(folder) = path.parent() {
                // One that could not be made fails the write below.
                let _ = std::fs::create_dir_all(folder);
            }
            std::fs::write(path, format!("{port} {token}\n")).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(error) =
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                {
                    log::warn!(
                        "{}: the link inbox could not be kept private: {error}",
                        path.display()
                    );
                }
            }
            std::thread::Builder::new()
                .name("link-inbox".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let (take, token) = (take.clone(), token.clone());
                        std::thread::spawn(move || {
                            if let Some(links) = receive(stream, Some(&token)) {
                                take(links);
                            }
                        });
                    }
                })
                .ok()?;
        }
    }
    Some(Inbox {
        address: address.clone(),
    })
}

/// Reads one message from `stream` (after `token`'s line, for a port),
/// answers "ok" and gives its links: answered first, so the editor that
/// handed them over is not left waiting while a link is followed.
fn receive<S: Read + Write + Timeouts>(mut stream: S, token: Option<&str>) -> Option<Vec<String>> {
    stream.timeouts(Duration::from_secs(2));
    let mut received = Vec::new();
    let mut chunk = [0u8; 4096];
    let links = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        received.extend_from_slice(&chunk[..read]);
        if received.len() > INBOX_MOST {
            return None;
        }
        let body = match token {
            Some(token) => {
                let Some(end) = received.iter().position(|byte| *byte == b'\n') else {
                    continue;
                };
                if received[..end] != *token.as_bytes() {
                    return None;
                }
                &received[end + 1..]
            }
            None => &received[..],
        };
        if let Some(links) = inbox_links(body) {
            break links;
        }
    };
    // The editor that handed them over may have stopped waiting; they are
    // followed all the same.
    let _ = stream.write_all(INBOX_TAKEN);
    let _ = stream.flush();
    Some(links)
}
