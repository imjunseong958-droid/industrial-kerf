//! A tool call as one line of a turn's work log, the way a coding agent shows
//! "Read main.rs" and then "Edited main.rs": said as it starts, and said
//! again, in its place, once it ends.

use kerf_agent::{AgentTargetSummary, ToolResultSummary};

/// The prefix Claude Code gives the tools of the `kerf` server.
const MCP_PREFIX: &str = "mcp__kerf__";
/// The most targets a line names before it says how many more.
const NAMED: usize = 3;
/// The longest reason a failed line gives, in characters.
const REASON_CHARS: usize = 80;

/// The tool's own name, without the MCP server's prefix.
pub(crate) fn tool_name(name: &str) -> &str {
    name.strip_prefix(MCP_PREFIX).unwrap_or(name)
}

/// What the call is about: its clips, media, tracks and times, a few of them.
fn about(targets: &AgentTargetSummary) -> String {
    let all: Vec<&str> = targets
        .clips
        .iter()
        .chain(&targets.sources)
        .chain(&targets.tracks)
        .chain(&targets.times)
        .map(String::as_str)
        .collect();
    match all.len() {
        0 => String::new(),
        n if n <= NAMED => all.join(", "),
        n => format!("{} and {} more", all[..NAMED].join(", "), n - NAMED),
    }
}

/// `verb` and what it is about, or `alone` where the call names nothing.
fn said(verb: &str, about: &str, alone: &str) -> String {
    if about.is_empty() {
        alone.to_owned()
    } else {
        format!("{verb} {about}")
    }
}

/// The line while the call runs.
pub(crate) fn running(name: &str, targets: &AgentTargetSummary) -> String {
    let about = about(targets);
    match tool_name(name) {
        "look" => said("Looking at", &about, "Looking at the project"),
        "edit" => said("Editing", &about, "Editing the cut"),
        "render" => "Rendering".to_owned(),
        "make" => "Preparing a making".to_owned(),
        "suggest" => "Suggesting next steps".to_owned(),
        "session.read" => "Reading earlier work".to_owned(),
        other => said(other, &about, other),
    }
}

/// The line once the call has ended. `targets` are those it started with.
pub(crate) fn finished(
    name: &str,
    targets: &AgentTargetSummary,
    ok: bool,
    summary: &ToolResultSummary,
) -> String {
    let about = about(targets);
    let tool = tool_name(name);
    if !ok {
        let verb = match tool {
            "look" => "Looking",
            "edit" => "The edit",
            "render" => "The render",
            "make" => "The making",
            other => other,
        };
        let reason = summary
            .error
            .as_deref()
            .map(|error| {
                let short: String = error.chars().take(REASON_CHARS).collect();
                if short.len() < error.len() {
                    format!(" · {}…", short.trim_end())
                } else {
                    format!(" · {short}")
                }
            })
            .unwrap_or_default();
        return format!("{verb} failed{reason}");
    }
    let status = summary.status.as_deref().unwrap_or_default();
    let media = summary.media.as_deref().unwrap_or("a media");
    match tool {
        "look" => said("Looked at", &about, "Looked at the project"),
        "edit" => match status {
            "awaiting_review" => said("Proposed a change to", &about, "Proposed a change"),
            "accepted" => said("Changed", &about, "Changed the cut"),
            _ => said("Edited", &about, "Edited the cut"),
        },
        "render" => {
            let file = summary
                .artifact_paths
                .first()
                .and_then(|path| std::path::Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned());
            let done = matches!(status, "completed" | "done" | "succeeded");
            match (done, file) {
                (true, Some(file)) => format!("Rendered {file}"),
                (true, None) => "Rendered".to_owned(),
                (false, Some(file)) => format!("Rendering {file}"),
                (false, None) => "Started a render".to_owned(),
            }
        }
        "make" => match status {
            "asked" => format!("Asked to make {media}"),
            "making" => format!("Making {media}"),
            "made" => match summary.take {
                Some(take) => format!("Made take {take} of {media}"),
                None => format!("Made {media}"),
            },
            "stopped" => format!("Stopped making {media}"),
            _ => format!("Checked the making of {media}"),
        },
        "suggest" => "Suggested next steps".to_owned(),
        "session.read" => "Read earlier work".to_owned(),
        other => said(other, &about, other),
    }
}
