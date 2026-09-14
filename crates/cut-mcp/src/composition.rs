//! Read-only, bounded inspection of a declarative HTML sidecar. This is a
//! conservative source scanner, not an HTML/JS engine or a second project model.
//! Only the entry's bytes are read; dependencies are checked for availability.

use super::McpError;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_HTML_BYTES: usize = 256 * 1024;
const MAX_ENTRY_BYTES: usize = 1024;
const MAX_TAGS: usize = 4096;
const MAX_ATTRIBUTES: usize = 64;
const MAX_VALUE_BYTES: usize = 4096;
const MAX_ITEMS: usize = 128;
// Staying below the router's inline threshold also prevents result-cache writes.
const MAX_OUTPUT_BYTES: usize = super::results::MAX_INLINE_BYTES;

pub(super) fn input_schema() -> Value {
    json!({"type":"object","properties":{"entry":{
        "type":"string","minLength":1,"maxLength":MAX_ENTRY_BYTES,"default":"index.html",
        "description":"Project-relative UTF-8 HTML file, at most 256 KiB. No absolute paths, .. components, backslashes, URL syntax, symlinks or special files. The limit is measured in UTF-8 bytes."
    }},"additionalProperties":false})
}

pub(super) fn check(directory: &Path, arguments: &Value) -> Result<Value, McpError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "composition.check expects an object"))?;
    if object.keys().any(|key| key != "entry") {
        return Err(McpError::new(
            "INVALID_ARGUMENTS",
            "only entry is supported",
        ));
    }
    let entry = match object.get("entry") {
        None => "index.html",
        Some(Value::String(entry)) => entry,
        _ => return Err(McpError::new("INVALID_ARGUMENTS", "entry must be a string")),
    };
    let path = relative_path(Path::new(""), entry, false)
        .map_err(|message| McpError::new("INVALID_ARGUMENTS", message))?;
    let root = File::open(directory).map_err(unavailable)?;
    let file = open_local(&root, &path).map_err(unavailable)?;
    if file.metadata().map_err(unavailable)?.len() > MAX_HTML_BYTES as u64 {
        return Err(McpError::new("COMPOSITION_LIMIT", "entry exceeds 256 KiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_HTML_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unavailable)?;
    if bytes.len() > MAX_HTML_BYTES {
        return Err(McpError::new("COMPOSITION_LIMIT", "entry exceeds 256 KiB"));
    }
    let html = std::str::from_utf8(&bytes)
        .map_err(|_| McpError::new("COMPOSITION_ENCODING", "entry must be UTF-8 HTML"))?;
    let mut inspection = Inspection::new(&root, path.parent().unwrap_or(Path::new("")));
    inspection.scan(html);
    inspection.finish(entry, bytes.len())
}

fn unavailable(error: std::io::Error) -> McpError {
    McpError::new(
        "COMPOSITION_UNAVAILABLE",
        format!("local regular file unavailable: {error}"),
    )
}

/// Normalize references lexically before any I/O. Entry paths reject all parent
/// components; dependency paths may use parents only inside the project root.
fn relative_path(base: &Path, value: &str, allow_parent: bool) -> Result<PathBuf, &'static str> {
    if value.is_empty()
        || value.len() > MAX_ENTRY_BYTES
        || value.chars().any(char::is_control)
        || value.contains(['\\', ':', '?', '#', '%'])
        || Path::new(value).is_absolute()
    {
        return Err("path must be a bounded project-relative path without URL syntax");
    }
    let mut path = base.to_path_buf();
    for component in Path::new(value).components() {
        match component {
            Component::Normal(name) => path.push(name),
            Component::CurDir => (),
            Component::ParentDir if allow_parent && path.pop() => (),
            _ => {
                return Err(
                    "path must not escape the project or contain parent components in entry",
                )
            }
        }
    }
    if path.as_os_str().is_empty() || path.as_os_str().len() > MAX_ENTRY_BYTES {
        return Err("path must name a file within the project");
    }
    Ok(path)
}

#[cfg(unix)]
fn open_local(root: &File, path: &Path) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    // Traverse using directory descriptors: replacing a component with a symlink
    // between inspection and open cannot redirect the read outside the project.
    let mut file = root.try_clone()?;
    let mut parts = path.components().peekable();
    while let Some(component) = parts.next() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::other("non-normal path component"));
        };
        let name =
            CString::new(name.as_bytes()).map_err(|_| std::io::Error::other("NUL in path"))?;
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        if parts.peek().is_some() {
            flags |= libc::O_DIRECTORY;
        }
        // SAFETY: the parent descriptor and C string stay alive for openat; no
        // creation flags are used. O_NONBLOCK avoids waiting on a FIFO/device.
        let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor, transferred once.
        file = unsafe { File::from_raw_fd(fd) };
    }
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_local(_root: &File, _path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "composition.check requires Unix descriptor-relative file traversal",
    ))
}

struct Tag {
    name: String,
    attrs: BTreeMap<String, String>,
    offset: usize,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    fn location(&self) -> Value {
        json!({"tag":self.name,"id":self.attr("id").map(short),"offset":self.offset})
    }
}

struct Inspection<'a> {
    root: &'a File,
    base: &'a Path,
    metadata: Value,
    roots: usize,
    tags: usize,
    lists: [Vec<Value>; 4],
    counts: [usize; 4],
    errors: usize,
    warnings: usize,
    deterministic: bool,
    scan_complete: bool,
}

const FINDINGS: usize = 0;
const ELEMENTS: usize = 1;
const ASSETS: usize = 2;
const NESTED: usize = 3;
const LIST_NAMES: [&str; 4] = ["findings", "elements", "assets", "nested"];

impl<'a> Inspection<'a> {
    fn new(root: &'a File, base: &'a Path) -> Self {
        Self {
            root,
            base,
            metadata: Value::Null,
            roots: 0,
            tags: 0,
            lists: Default::default(),
            counts: [0; 4],
            errors: 0,
            warnings: 0,
            deterministic: true,
            scan_complete: true,
        }
    }

    fn push(&mut self, list: usize, item: Value) {
        self.counts[list] += 1;
        if self.lists[list].len() < MAX_ITEMS {
            self.lists[list].push(item);
        }
    }

    fn finding(
        &mut self,
        code: &str,
        message: &str,
        offset: usize,
        error: bool,
        determinism: bool,
    ) {
        if error {
            self.errors += 1;
        } else {
            self.warnings += 1;
        }
        self.deterministic &= !determinism;
        self.push(
            FINDINGS,
            json!({"code":code,"severity":if error {"error"} else {"warning"},
            "message":message,"offset":offset,"affectsDeterminism":determinism}),
        );
    }

    fn incomplete(&mut self, offset: usize, message: &str) {
        self.scan_complete = false;
        self.finding("HTML_SCAN_INCOMPLETE", message, offset, true, true);
    }

    fn scan(&mut self, html: &str) {
        let lower = html.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(relative) = html[cursor..].find('<') {
            let start = cursor + relative;
            if html[start..].starts_with("<!--") {
                let Some(end) = html[start + 4..].find("-->") else {
                    self.incomplete(start, "Unterminated HTML comment");
                    break;
                };
                cursor = start + 4 + end + 3;
                continue;
            }
            if lower[start..].starts_with("<!doctype html>") {
                cursor = start + "<!doctype html>".len();
                continue;
            }
            if html[start..].starts_with("</") {
                let Some(end) = html[start..].find('>') else {
                    self.incomplete(start, "Unterminated closing tag");
                    break;
                };
                cursor = start + end + 1;
                continue;
            }
            if !html
                .as_bytes()
                .get(start + 1)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.incomplete(
                    start,
                    "Unsupported markup; only ordinary tags, comments and HTML doctype are scanned",
                );
                break;
            }
            if self.tags == MAX_TAGS {
                self.incomplete(start, "Entry exceeds the 4096-tag scan limit");
                break;
            }
            let (tag, end) = match parse_tag(html, start) {
                Ok(tag) => tag,
                Err(message) => {
                    self.incomplete(start, message);
                    break;
                }
            };
            self.tags += 1;
            cursor = end;
            self.inspect_tag(&tag);
            if matches!(
                tag.name.as_str(),
                "script" | "style" | "textarea" | "title" | "iframe" | "xmp"
            ) {
                let Some(end) = raw_text_end(&lower, cursor, &tag.name) else {
                    self.incomplete(start, "Unterminated raw-text element");
                    break;
                };
                if tag.name == "script" {
                    let kind = tag.attr("type").unwrap_or("").trim().to_ascii_lowercase();
                    if matches!(
                        kind.as_str(),
                        "" | "module" | "text/javascript" | "application/javascript"
                    ) {
                        self.inspect_code(&html[cursor..end], start);
                    } else if !matches!(kind.as_str(), "application/json" | "application/ld+json") {
                        self.finding(
                            "UNINSPECTED_SCRIPT_TYPE",
                            "Script type is outside the static JavaScript scan",
                            start,
                            false,
                            true,
                        );
                    }
                } else if tag.name == "style" {
                    self.inspect_css(&html[cursor..end], &tag);
                }
                cursor = end;
            }
        }
        if self.roots == 0 {
            self.finding(
                "MISSING_COMPOSITION_ROOT",
                "Expected one data-composition-id, data-composition, or id=composition root",
                0,
                true,
                false,
            );
        }
    }

    fn inspect_tag(&mut self, tag: &Tag) {
        let nested = tag
            .attr("data-composition-src")
            .or_else(|| tag.attr("data-composition-source"));
        let is_root = nested.is_none()
            && (tag.attr("data-composition-id").is_some()
                || tag.attr("data-composition").is_some()
                || tag.attr("id") == Some("composition"));
        if is_root {
            self.roots += 1;
            if self.roots > 1 {
                self.finding(
                    "MULTIPLE_COMPOSITION_ROOTS",
                    "Entry must declare exactly one composition root",
                    tag.offset,
                    true,
                    false,
                );
            } else {
                let id = tag.attr("data-composition-id").or_else(|| tag.attr("id"));
                let id = id.filter(|id| !id.trim().is_empty() && id.len() <= 256);
                if id.is_none() {
                    self.finding(
                        "INVALID_METADATA",
                        "Composition id is required and must be at most 256 bytes",
                        tag.offset,
                        true,
                        false,
                    );
                }
                let mut metadata = json!({"id":id});
                for name in ["width", "height", "fps", "duration"] {
                    let value = tag.attr(&format!("data-{name}")).or_else(|| tag.attr(name));
                    metadata[name] = json!(self.number(
                        value,
                        name,
                        tag.offset,
                        false,
                        matches!(name, "width" | "height")
                    ));
                }
                self.metadata = metadata;
            }
        }
        if tag.attr("data-start").is_some()
            || tag.attr("data-track-index").is_some()
            || (!is_root && tag.attr("data-duration").is_some())
        {
            let start = self.number(
                tag.attr("data-start"),
                "data-start",
                tag.offset,
                true,
                false,
            );
            let duration = self.number(
                tag.attr("data-duration"),
                "data-duration",
                tag.offset,
                false,
                false,
            );
            let track = self
                .number(
                    tag.attr("data-track-index"),
                    "data-track-index",
                    tag.offset,
                    true,
                    true,
                )
                .map(|value| value as u32);
            self.push(ELEMENTS, json!({"element":tag.location(),"start":start,"duration":duration,"trackIndex":track}));
        }
        if let Some(value) = nested {
            self.reference(tag, "data-composition-src", value, true, true);
        }
        if matches!(
            tag.name.as_str(),
            "base" | "template" | "noscript" | "noembed" | "noframes" | "plaintext"
        ) {
            self.finding(
                "UNSUPPORTED_HTML_CONTEXT",
                "Base URLs and conditional or inert HTML contexts are outside this scanner",
                tag.offset,
                true,
                true,
            );
        }
        for (name, value) in &tag.attrs {
            if matches!(name.as_str(), "src" | "href" | "xlink:href" | "poster")
                || (tag.name == "object" && name == "data")
            {
                let executable =
                    matches!(tag.name.as_str(), "script" | "iframe" | "object" | "embed")
                        || (tag.name == "link"
                            && tag.attr("rel").is_some_and(|rel| {
                                rel.split_ascii_whitespace().any(|part| {
                                    part.eq_ignore_ascii_case("stylesheet")
                                        || part.eq_ignore_ascii_case("modulepreload")
                                })
                            }));
                self.reference(tag, name, value, false, executable);
            } else if name == "style" {
                self.inspect_css(value, tag);
            } else if name.starts_with("on") {
                self.inspect_code(value, tag.offset);
            } else if matches!(
                name.as_str(),
                "srcdoc" | "srcset" | "imagesrcset" | "ping" | "manifest"
            ) {
                self.finding(
                    "UNINSPECTED_ATTRIBUTE",
                    "Embedded HTML or multi-resource attribute is outside this scan",
                    tag.offset,
                    false,
                    true,
                );
            }
        }
        if tag.name == "meta"
            && tag
                .attr("http-equiv")
                .is_some_and(|v| v.eq_ignore_ascii_case("refresh"))
        {
            self.finding(
                "RENDER_TIME_NAVIGATION",
                "Meta refresh can navigate or access the network during rendering",
                tag.offset,
                true,
                true,
            );
        }
    }

    fn number(
        &mut self,
        value: Option<&str>,
        field: &str,
        offset: usize,
        zero: bool,
        integer: bool,
    ) -> Option<f64> {
        let number = value
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| {
                value.is_finite()
                    && (if zero { *value >= 0.0 } else { *value > 0.0 })
                    && (!integer || (value.fract() == 0.0 && *value <= u32::MAX as f64))
            });
        if number.is_none() {
            self.finding(
                "INVALID_METADATA",
                &format!(
                    "{field} is required and must be a finite {}{} number",
                    if zero { "non-negative" } else { "positive" },
                    if integer { " integer" } else { "" }
                ),
                offset,
                true,
                false,
            );
        }
        number
    }

    fn reference(
        &mut self,
        tag: &Tag,
        attribute: &str,
        value: &str,
        nested: bool,
        executable: bool,
    ) {
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        let (kind, path, available) = if value.starts_with('#') && !nested && !executable {
            ("fragment", None, None)
        } else if lower.starts_with("http:")
            || lower.starts_with("https:")
            || value.starts_with("//")
        {
            self.finding(
                "EXTERNAL_REFERENCE",
                "External HTTP(S) and protocol-relative resources are forbidden",
                tag.offset,
                true,
                true,
            );
            ("external", None, None)
        } else {
            let local = value.split(['?', '#']).next().unwrap_or("");
            let resolved =
                percent_decode(local).and_then(|decoded| relative_path(self.base, &decoded, true));
            match resolved {
                Ok(path) => {
                    let available = open_local(self.root, &path).is_ok();
                    if !available {
                        self.finding(
                            "LOCAL_REFERENCE_UNAVAILABLE",
                            "Local reference must exist as a regular file without symlinks",
                            tag.offset,
                            true,
                            executable,
                        );
                    }
                    if executable && available {
                        self.finding("DEPENDENCY_NOT_INSPECTED", "Referenced composition, script, stylesheet or document is not read; its determinism is unknown", tag.offset, false, true);
                    }
                    (
                        "local",
                        Some(path.to_string_lossy().into_owned()),
                        Some(available),
                    )
                }
                Err(_) => {
                    self.finding("UNSAFE_REFERENCE", "Reference has an unsupported scheme, encoding, path escape or excessive length", tag.offset, true, true);
                    ("unsafe", None, None)
                }
            }
        };
        self.push(
            if nested { NESTED } else { ASSETS },
            json!({"element":tag.location(),
            "attribute":attribute,"reference":short(value),"referenceTruncated":value.len()>256,
            "kind":kind,"path":path,"available":available,"inspected":false}),
        );
    }

    fn inspect_code(&mut self, code: &str, offset: usize) {
        // Conservative lexical detection. Comments/spacing cannot hide common
        // calls, including bracket property access. Strings can cause findings;
        // computed aliases/obfuscation are not a supported JavaScript analysis.
        let compact: String = without_comments(code, true)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for (code, message, patterns) in [
            (
                "WALL_CLOCK",
                "Wall-clock time makes frame evaluation nondeterministic",
                &[
                    "Date.now",
                    "Date['now']",
                    "Date[\"now\"]",
                    "newDate(",
                    "Date()",
                    "performance.now",
                    "performance['now']",
                    "performance[\"now\"]",
                ][..],
            ),
            (
                "RANDOMNESS",
                "Unseeded randomness makes frame evaluation nondeterministic",
                &[
                    "Math.random",
                    "Math['random']",
                    "Math[\"random\"]",
                    "crypto.getRandomValues",
                    "crypto.randomUUID",
                ][..],
            ),
            (
                "UNCONTROLLED_TIMER",
                "Animation frames and timers run independently of composition time",
                &["requestAnimationFrame", "setInterval", "setTimeout"][..],
            ),
            (
                "RENDER_TIME_NETWORK",
                "Render-time networking and external URL literals are forbidden",
                &[
                    "fetch",
                    "XMLHttpRequest",
                    "WebSocket",
                    "EventSource",
                    "sendBeacon",
                    "import(",
                    "importScripts",
                    "Worker(",
                    "http://",
                    "https://",
                ][..],
            ),
            (
                "DYNAMIC_CODE",
                "Dynamically generated code cannot be checked statically",
                &["eval(", "Function("][..],
            ),
        ] {
            if patterns.iter().any(|pattern| compact.contains(*pattern)) {
                self.finding(code, message, offset, true, true);
            }
        }
        if compact.contains("import") || compact.contains("export") {
            self.finding(
                "UNINSPECTED_MODULE",
                "JavaScript module dependencies are not resolved or read",
                offset,
                false,
                true,
            );
        }
    }

    fn inspect_css(&mut self, css: &str, tag: &Tag) {
        if css.contains('\\') {
            self.finding(
                "UNSUPPORTED_CSS_ESCAPE",
                "Escaped CSS tokens cannot be resolved by this scanner",
                tag.offset,
                true,
                true,
            );
        }
        let css = without_comments(css, false);
        let lower = css.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(relative) = lower[cursor..].find("url") {
            let start = cursor + relative + 3;
            cursor = start;
            let rest = css[start..].trim_start();
            let Some(rest) = rest.strip_prefix('(') else {
                continue;
            };
            let rest = rest.trim_start();
            let (value, remaining) = if rest.starts_with(['\'', '"']) {
                let quote = rest.as_bytes()[0] as char;
                let Some(end) = rest[1..].find(quote) else {
                    self.incomplete(tag.offset, "Unterminated CSS URL string");
                    break;
                };
                (&rest[1..1 + end], &rest[2 + end..])
            } else {
                let Some(end) = rest.find(')') else {
                    self.incomplete(tag.offset, "Unterminated CSS URL");
                    break;
                };
                (rest[..end].trim(), &rest[end..])
            };
            let remaining = remaining.trim_start();
            if !remaining.starts_with(')') {
                self.incomplete(tag.offset, "Malformed CSS URL");
                break;
            }
            self.reference(tag, "style:url", value, false, false);
            cursor = css.len() - remaining.len() + 1;
        }
        let mut cursor = 0;
        while let Some(relative) = lower[cursor..].find("@import") {
            cursor += relative + "@import".len();
            self.finding(
                "DEPENDENCY_NOT_INSPECTED",
                "Imported stylesheet content is not read",
                tag.offset,
                false,
                true,
            );
            let rest = css[cursor..].trim_start();
            if rest.starts_with(['\'', '"']) {
                let quote = rest.as_bytes()[0] as char;
                if let Some(end) = rest[1..].find(quote) {
                    self.reference(tag, "style:import", &rest[1..1 + end], false, false);
                } else {
                    self.incomplete(tag.offset, "Unterminated CSS import string");
                    break;
                }
            }
        }
    }

    fn finish(self, entry: &str, bytes: usize) -> Result<Value, McpError> {
        let mut report = json!({"entry":entry,"scope":"entry-only-static",
            "valid":self.errors==0,"deterministic":self.deterministic && self.scan_complete,
            "scanComplete":self.scan_complete,"metadata":self.metadata,"bytesRead":bytes,"outputBytes":0,
            "truncated":false,"counts":{"tags":self.tags,"roots":self.roots,"errors":self.errors,"warnings":self.warnings},
            "returned":{},"limits":{"htmlBytes":MAX_HTML_BYTES,"entryBytes":MAX_ENTRY_BYTES,
                "tags":MAX_TAGS,"attributesPerTag":MAX_ATTRIBUTES,"attributeBytes":MAX_VALUE_BYTES,
                "itemsPerList":MAX_ITEMS,"outputBytes":MAX_OUTPUT_BYTES},
            "limitations":"Limited HTML tokenizer and conservative pattern scan, not browser validation or a security sandbox. Offsets are UTF-8 byte offsets of containing tags. Dependency bytes and computed JavaScript references are not inspected. Truncated lists retain prefix items; counts and validity include all scanned items."});
        for (index, items) in self.lists.into_iter().enumerate() {
            report["counts"][LIST_NAMES[index]] = json!(self.counts[index]);
            report[LIST_NAMES[index]] = json!(items);
        }
        loop {
            for (index, name) in LIST_NAMES.iter().enumerate() {
                let returned = report[*name].as_array().map_or(0, Vec::len);
                report["returned"][*name] = json!(returned);
                if returned < self.counts[index] {
                    report["truncated"] = json!(true);
                }
            }
            let size = report.to_string().len();
            if size <= MAX_OUTPUT_BYTES {
                if report["outputBytes"] == json!(size) {
                    return Ok(report);
                }
                report["outputBytes"] = json!(size);
                continue;
            }
            let largest = LIST_NAMES
                .iter()
                .max_by_key(|name| report[**name].to_string().len())
                .unwrap();
            if report[*largest].as_array_mut().and_then(Vec::pop).is_none() {
                return Err(McpError::new(
                    "COMPOSITION_LIMIT",
                    "summary exceeds the inline response budget",
                ));
            }
        }
    }
}

fn short(value: &str) -> &str {
    let mut end = value.len().min(256);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn parse_tag(html: &str, offset: usize) -> Result<(Tag, usize), &'static str> {
    let bytes = html.as_bytes();
    let mut cursor = offset + 1;
    let start = cursor;
    while cursor < bytes.len()
        && (bytes[cursor].is_ascii_alphanumeric() || b"-_:".contains(&bytes[cursor]))
    {
        cursor += 1;
    }
    if cursor - start > 64 {
        return Err("Tag name exceeds 64 bytes");
    }
    let name = html[start..cursor].to_ascii_lowercase();
    let mut attrs = BTreeMap::new();
    loop {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == bytes.len() {
            return Err("Unterminated opening tag");
        }
        if bytes[cursor] == b'>' {
            return Ok((
                Tag {
                    name,
                    attrs,
                    offset,
                },
                cursor + 1,
            ));
        }
        if html[cursor..].starts_with("/>") {
            return Ok((
                Tag {
                    name,
                    attrs,
                    offset,
                },
                cursor + 2,
            ));
        }
        if attrs.len() == MAX_ATTRIBUTES {
            return Err("Tag exceeds 64 attributes");
        }
        let start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || b"-_:".contains(&bytes[cursor]))
        {
            cursor += 1;
        }
        if cursor == start || cursor - start > 64 {
            return Err("Unsupported or oversized attribute name");
        }
        let key = html[start..cursor].to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = String::new();
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let start;
            let end;
            if bytes.get(cursor).is_some_and(|b| matches!(b, b'\'' | b'"')) {
                let quote = bytes[cursor];
                cursor += 1;
                start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                if cursor == bytes.len() {
                    return Err("Unterminated attribute string");
                }
                end = cursor;
                cursor += 1;
            } else {
                start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'>'
                {
                    if b"<'\"=`".contains(&bytes[cursor]) {
                        return Err("Malformed unquoted attribute");
                    }
                    cursor += 1;
                }
                end = cursor;
                if start == end {
                    return Err("Missing attribute value");
                }
            }
            if end - start > MAX_VALUE_BYTES {
                return Err("Attribute exceeds 4096 bytes");
            }
            value = decode_entities(&html[start..end])?;
        }
        if attrs.insert(key, value).is_some() {
            return Err("Duplicate attributes are ambiguous");
        }
    }
}

fn raw_text_end(lower: &str, mut cursor: usize, name: &str) -> Option<usize> {
    let closing = format!("</{name}");
    while let Some(relative) = lower[cursor..].find(&closing) {
        let start = cursor + relative;
        cursor = start + closing.len();
        if lower
            .as_bytes()
            .get(cursor)
            .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
        {
            return Some(start);
        }
    }
    None
}

fn decode_entities(value: &str) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        if let Some(numeric) = rest.strip_prefix('#') {
            let (digits, radix, prefix) = match numeric.strip_prefix(['x', 'X']) {
                Some(hex) => (hex, 16, 2),
                None => (numeric, 10, 1),
            };
            let count = digits
                .bytes()
                .take_while(|b| {
                    if radix == 16 {
                        b.is_ascii_hexdigit()
                    } else {
                        b.is_ascii_digit()
                    }
                })
                .count();
            let ch = u32::from_str_radix(&digits[..count], radix)
                .ok()
                .and_then(char::from_u32)
                .filter(|c| *c != '\0')
                .ok_or("Invalid numeric character reference")?;
            output.push(ch);
            rest = &rest[prefix + count..];
            rest = rest.strip_prefix(';').unwrap_or(rest);
        } else if let Some(end) = rest
            .find(';')
            .filter(|end| *end <= 12 && rest[..*end].bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            let decoded = match &rest[..end] {
                "amp" | "AMP" => '&',
                "quot" | "QUOT" => '"',
                "apos" => '\'',
                "lt" | "LT" => '<',
                "gt" | "GT" => '>',
                "colon" => ':',
                "sol" => '/',
                "bsol" => '\\',
                "period" => '.',
                "Tab" => '\t',
                "NewLine" => '\n',
                _ => return Err("Unsupported named character reference in attribute"),
            };
            output.push(decoded);
            rest = &rest[end + 1..];
        } else {
            output.push('&');
        }
    }
    output.push_str(rest);
    Ok(output)
}

fn percent_decode(value: &str) -> Result<String, &'static str> {
    let mut decoded = Vec::new();
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let digits = bytes
                .get(cursor + 1..cursor + 3)
                .ok_or("Invalid URL escape")?;
            let high = (digits[0] as char)
                .to_digit(16)
                .ok_or("Invalid URL escape")?;
            let low = (digits[1] as char)
                .to_digit(16)
                .ok_or("Invalid URL escape")?;
            decoded.push((high * 16 + low) as u8);
            cursor += 3;
        } else {
            decoded.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "URL path is not UTF-8")
}

fn without_comments(value: &str, javascript: bool) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    let mut quote = None;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(delimiter) = quote {
            output.push(byte);
            cursor += 1;
            if byte == b'\\' && cursor < bytes.len() {
                output.push(bytes[cursor]);
                cursor += 1;
            } else if byte == delimiter {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            output.push(byte);
            cursor += 1;
        } else if bytes[cursor..].starts_with(b"/*") {
            cursor += 2;
            while cursor < bytes.len() && !bytes[cursor..].starts_with(b"*/") {
                cursor += 1;
            }
            cursor = (cursor + 2).min(bytes.len());
        } else if javascript && bytes[cursor..].starts_with(b"//") {
            while cursor < bytes.len() && !matches!(bytes[cursor], b'\r' | b'\n') {
                cursor += 1;
            }
        } else {
            output.push(byte);
            cursor += 1;
        }
    }
    // Only whole ASCII comment delimiters and comment bodies were removed.
    String::from_utf8(output).expect("comment removal preserves UTF-8")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const ROOT: &str = r#"<main data-composition-id="intro" data-width="1920" data-height="1080" data-fps="30" data-duration="5"></main>"#;

    fn inspect(html: &str) -> Value {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("index.html"), html).unwrap();
        check(dir.path(), &json!({})).unwrap()
    }

    fn has_finding(report: &Value, code: &str, severity: &str) -> bool {
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == code && finding["severity"] == severity)
    }

    #[test]
    fn composition_valid_fixture_extracts_metadata_timing_assets_and_nested() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("assets")).unwrap();
        fs::create_dir(dir.path().join("scenes")).unwrap();
        for name in ["paper.svg", "photo.svg", "mask.svg"] {
            fs::write(dir.path().join("assets").join(name), b"<svg/>").unwrap();
        }
        // Dependency contents are deliberately not valid HTML. Only availability
        // is checked, and an explicit warning prevents a determinism claim.
        fs::write(
            dir.path().join("scenes/outro.html"),
            b"fetch('https://example.invalid')",
        )
        .unwrap();
        let html = include_str!("../tests/fixtures/composition.html");
        fs::write(dir.path().join("index.html"), html).unwrap();
        let report = check(dir.path(), &json!({})).unwrap();
        assert_eq!(report["valid"], true, "{report}");
        assert_eq!(report["deterministic"], false);
        assert_eq!(report["scanComplete"], true);
        assert_eq!(
            report["metadata"],
            json!({"id":"intro","width":1920.0,"height":1080.0,"fps":30.0,"duration":5.0})
        );
        assert_eq!(report["bytesRead"], html.len());
        assert_eq!(report["counts"]["elements"], 3);
        assert_eq!(report["elements"][0]["element"]["id"], "title");
        assert_eq!(report["elements"][0]["start"], 0.0);
        assert_eq!(report["elements"][0]["duration"], 2.5);
        assert_eq!(report["elements"][0]["trackIndex"], 0);
        assert_eq!(report["counts"]["assets"], 3);
        assert!(report["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|asset| asset["available"] == true && asset["kind"] == "local"));
        assert!(report["assets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|asset| asset["path"] == "assets/photo.svg"
                && asset["reference"] == "assets/photo.svg?version=1&size=small"));
        assert_eq!(report["nested"][0]["path"], "scenes/outro.html");
        assert_eq!(report["nested"][0]["inspected"], false);
        assert!(has_finding(&report, "DEPENDENCY_NOT_INSPECTED", "warning"));
        assert!(!has_finding(&report, "RENDER_TIME_NETWORK", "error"));
        assert_eq!(
            check(dir.path(), &json!({"entry":"index.html"})).unwrap(),
            report
        );
        assert!(!dir.path().join(".cut").exists());
    }

    #[test]
    fn composition_self_contained_entry_has_no_findings() {
        let report = inspect(ROOT);
        assert_eq!(report["valid"], true);
        assert_eq!(report["deterministic"], true);
        assert_eq!(report["truncated"], false);
        assert_eq!(
            report["counts"]["elements"], 0,
            "root duration alone is not a timed element"
        );
        assert_eq!(report["findings"], json!([]));
        assert_eq!(report["outputBytes"], report.to_string().len());
        let alternative = inspect(
            r#"<main data-composition id='intro' width=1920 height=1080 fps=30 duration=5></main>"#,
        );
        assert_eq!(alternative["metadata"], report["metadata"]);
        assert_eq!(alternative["valid"], true);
    }

    #[test]
    fn composition_entry_rejects_traversal_absolute_paths_and_extra_arguments() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("index.html"), ROOT).unwrap();
        for entry in [
            "",
            "..",
            "../index.html",
            "sub/../index.html",
            "/index.html",
            "C:\\index.html",
            "sub\\index.html",
            "https://example.invalid/a",
            "%2e%2e/index.html",
            "index.html?x=1",
            "index.html\0",
        ] {
            let error = check(dir.path(), &json!({"entry":entry})).unwrap_err();
            assert_eq!(error.code, "INVALID_ARGUMENTS", "{entry:?}");
        }
        for arguments in [
            json!(null),
            json!([]),
            json!({"entry":false}),
            json!({"entry":null}),
            json!({"entry":"x".repeat(MAX_ENTRY_BYTES + 1)}),
            json!({"entry":"index.html","execute":true}),
        ] {
            assert_eq!(
                check(dir.path(), &arguments).unwrap_err().code,
                "INVALID_ARGUMENTS"
            );
        }
        assert_eq!(
            check(dir.path(), &json!({"entry":dir.path().join("index.html")}))
                .unwrap_err()
                .code,
            "INVALID_ARGUMENTS"
        );
    }

    #[test]
    fn composition_entry_rejects_symlinks_directories_fifos_and_missing_files() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("index.html"), ROOT).unwrap();
        fs::create_dir(dir.path().join("directory")).unwrap();
        symlink(
            outside.path().join("index.html"),
            dir.path().join("link.html"),
        )
        .unwrap();
        symlink(outside.path(), dir.path().join("linked-directory")).unwrap();
        let fifo = std::ffi::CString::new(dir.path().join("fifo").as_os_str().as_bytes()).unwrap();
        // SAFETY: the C string is a path owned by this temporary test directory.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for entry in [
            "directory",
            "link.html",
            "linked-directory/index.html",
            "fifo",
            "missing.html",
        ] {
            assert_eq!(
                check(dir.path(), &json!({"entry":entry})).unwrap_err().code,
                "COMPOSITION_UNAVAILABLE",
                "{entry}"
            );
        }
    }

    #[test]
    fn composition_rejects_oversized_and_non_utf8_entries() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("index.html"),
            vec![b' '; MAX_HTML_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            check(dir.path(), &json!({})).unwrap_err().code,
            "COMPOSITION_LIMIT"
        );
        fs::write(dir.path().join("index.html"), [0xff, 0xfe]).unwrap();
        assert_eq!(
            check(dir.path(), &json!({})).unwrap_err().code,
            "COMPOSITION_ENCODING"
        );
    }

    #[test]
    fn composition_reports_missing_invalid_metadata_and_timing() {
        assert!(has_finding(
            &inspect("<html></html>"),
            "MISSING_COMPOSITION_ROOT",
            "error"
        ));
        let report = inspect(
            r#"<main data-composition data-width="NaN" data-height="0" data-fps="Infinity" data-duration="-2"><div data-start="-1" data-duration="0" data-track-index="1.5"></div></main>"#,
        );
        assert_eq!(report["valid"], false);
        assert_eq!(
            report["deterministic"], true,
            "declaration errors alone do not imply nondeterminism"
        );
        assert_eq!(report["counts"]["errors"], 8);
        assert_eq!(report["metadata"]["width"], Value::Null);
        assert_eq!(report["elements"][0]["trackIndex"], Value::Null);
        let incomplete = inspect(&format!("{ROOT}<div data-duration='1'></div>"));
        assert_eq!(incomplete["counts"]["errors"], 2);
        assert!(has_finding(
            &inspect(&format!("{ROOT}{ROOT}")),
            "MULTIPLE_COMPOSITION_ROOTS",
            "error"
        ));
    }

    #[test]
    fn composition_detects_forbidden_calls_in_scripts_and_event_attributes() {
        for (source, code) in [
            ("Date.now()", "WALL_CLOCK"),
            ("performance /*gap*/ . now ()", "WALL_CLOCK"),
            ("Math [ 'random' ] ()", "RANDOMNESS"),
            ("requestAnimationFrame(tick)", "UNCONTROLLED_TIMER"),
            ("window.setInterval(tick, 10)", "UNCONTROLLED_TIMER"),
            ("fetch('/a')", "RENDER_TIME_NETWORK"),
            ("new XMLHttpRequest()", "RENDER_TIME_NETWORK"),
            ("new WebSocket('/a')", "RENDER_TIME_NETWORK"),
            ("window['fetch']('/a')", "RENDER_TIME_NETWORK"),
            ("eval(code)", "DYNAMIC_CODE"),
            (
                "image.src = 'https://example.invalid/frame.png'",
                "RENDER_TIME_NETWORK",
            ),
        ] {
            let report = inspect(&format!("{ROOT}<script>{source}</script>"));
            assert!(has_finding(&report, code, "error"), "{source}: {report}");
            assert_eq!(report["valid"], false);
            assert_eq!(report["deterministic"], false);
        }
        let event = inspect(&format!("{ROOT}<img onload='Math&#46;random()'>"));
        assert!(has_finding(&event, "RANDOMNESS", "error"));
    }

    #[test]
    fn composition_ignores_comments_text_and_raw_text_markup() {
        let report = inspect(&format!(
            r#"{ROOT}
            <!-- <main data-composition-id='fake'><img src='https://example.invalid/x'> Date.now() -->
            <p title="quoted > angle">Date.now() Math.random()</p>
            <textarea><img src='https://example.invalid/fake'></textarea>
            <script>// Date.now()
                /* fetch('/fake') */ const markup = "<img src='fake.png'>";
            </script>"#
        ));
        assert_eq!(report["valid"], true, "{report}");
        assert_eq!(report["deterministic"], true);
        assert_eq!(report["counts"]["roots"], 1);
        assert_eq!(report["counts"]["assets"], 0);
    }

    #[test]
    fn composition_reports_external_encoded_and_escaping_references() {
        let report = inspect(&format!(
            r#"{ROOT}
            <img src="https&#58;//example.invalid/a"><link href='//example.invalid/a.css'>
            <div style="background: URL( 'http://example.invalid/a' )"></div>
            <style>@import "https://example.invalid/font.css";</style>
            <img src='../outside.png'><img src='%2e%2e/outside.png'>
            <img src='file:///etc/passwd'><img src='javascript:alert(1)'>
            <section data-composition-src='https://example.invalid/child.html'></section>"#
        ));
        assert_eq!(report["valid"], false);
        assert_eq!(report["deterministic"], false);
        assert_eq!(report["counts"]["assets"], 8);
        assert_eq!(report["counts"]["nested"], 1);
        assert!(has_finding(&report, "EXTERNAL_REFERENCE", "error"));
        assert!(has_finding(&report, "UNSAFE_REFERENCE", "error"));
    }

    #[test]
    fn composition_resolves_dependencies_from_entry_directory_without_following_links() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("scenes")).unwrap();
        fs::write(dir.path().join("my asset.svg"), b"<svg/>").unwrap();
        fs::write(dir.path().join("code.js"), b"fetch('/not-read')").unwrap();
        fs::write(outside.path().join("private.svg"), b"private").unwrap();
        symlink(outside.path(), dir.path().join("linked")).unwrap();
        let html = format!(
            r#"{ROOT}<img src='../my%20asset.svg?x=1#part'><script src='../code.js'></script><img src='../linked/private.svg'><a href='#intro'>Back</a>"#
        );
        fs::write(dir.path().join("scenes/intro.html"), html).unwrap();
        let report = check(dir.path(), &json!({"entry":"scenes/intro.html"})).unwrap();
        assert_eq!(report["assets"][0]["path"], "my asset.svg");
        assert_eq!(report["assets"][0]["available"], true);
        assert_eq!(report["assets"][2]["available"], false);
        assert_eq!(report["assets"][3]["kind"], "fragment");
        assert!(has_finding(&report, "LOCAL_REFERENCE_UNAVAILABLE", "error"));
        assert!(has_finding(&report, "DEPENDENCY_NOT_INSPECTED", "warning"));
        assert_eq!(report["deterministic"], false);
    }

    #[test]
    fn composition_scan_limits_and_unsupported_markup_fail_closed() {
        for tail in [
            "<i></i>".repeat(MAX_TAGS),
            format!("<p title='{}'>", "x".repeat(MAX_VALUE_BYTES + 1)),
            format!(
                "<p {}>",
                (0..=MAX_ATTRIBUTES)
                    .map(|i| format!("x{i}='1' "))
                    .collect::<String>()
            ),
            "<p title='unterminated>".into(),
            "<p title='&unknown;'>".into(),
            "<p src='a' src='b'>".into(),
            "<script>Math.random()".into(),
        ] {
            let report = inspect(&format!("{ROOT}{tail}"));
            assert_eq!(report["valid"], false, "{report}");
            assert_eq!(report["scanComplete"], false);
            assert_eq!(report["deterministic"], false);
            assert!(has_finding(&report, "HTML_SCAN_INCOMPLETE", "error"));
        }
    }

    #[test]
    fn composition_output_is_bounded_without_losing_aggregate_errors() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("asset.svg"), b"<svg/>").unwrap();
        let mut html = ROOT.to_owned();
        for i in 0..300 {
            html.push_str(&format!("<img id='{i}{}' src='asset.svg' data-start='0' data-duration='1' data-track-index='0'>", "한".repeat(80)));
        }
        // This error follows more elements than can be returned in any list.
        html.push_str("<script>Math.random()</script>");
        fs::write(dir.path().join("index.html"), html).unwrap();
        let report = check(dir.path(), &json!({})).unwrap();
        assert!(report.to_string().len() <= MAX_OUTPUT_BYTES);
        assert_eq!(report["outputBytes"], report.to_string().len());
        assert_eq!(report["counts"]["elements"], 300);
        assert_eq!(report["counts"]["assets"], 300);
        assert_eq!(report["counts"]["errors"], 1);
        assert_eq!(report["valid"], false);
        assert_eq!(report["deterministic"], false);
        assert_eq!(report["scanComplete"], true);
        assert_eq!(report["truncated"], true);
        assert!(report["returned"]["elements"].as_u64().unwrap() < 300);
        assert!(report["returned"]["assets"].as_u64().unwrap() < 300);
        assert!(!dir.path().join(".cut").exists());

        let findings = inspect(&format!(
            "{ROOT}{}",
            "<script>Date.now()</script>".repeat(MAX_ITEMS + 20)
        ));
        assert_eq!(findings["counts"]["errors"], MAX_ITEMS + 20);
        assert!(findings["returned"]["findings"].as_u64().unwrap() < (MAX_ITEMS + 20) as u64);
        assert_eq!(findings["valid"], false);
        assert_eq!(findings["truncated"], true);
    }
}
