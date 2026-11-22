//! What the chip over the picture says of what is being made: pure, so
//! that tests can hold it to its words.

/// How a making's line says it stands (`asked`, `waiting … 2 ahead`,
/// `running 12s`, `rendering 40%`, `made 31s`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Asked,
    /// Its turn on the server not yet come, with how many are ahead.
    Waiting {
        ahead: Option<u32>,
    },
    Running,
    Rendering,
    Made,
    Other,
}

impl Phase {
    /// Waiting its turn: queued, not yet making.
    pub fn queued(self) -> bool {
        matches!(self, Phase::Asked | Phase::Waiting { .. })
    }
}

/// The first word of `line`, taken whole. The line is whatever the
/// making's program printed, in any script: it is cut only where a
/// character ends.
fn starts_with_word(line: &str, word: &str) -> bool {
    let (Some(head), Some(rest)) = (line.get(..word.len()), line.get(word.len()..)) else {
        return false;
    };
    head.eq_ignore_ascii_case(word)
        && !rest
            .chars()
            .next()
            .is_some_and(|next| next.is_alphanumeric() || next == '_')
}

/// How the making's `line` says it stands.
pub fn phase(line: &str) -> Phase {
    let said = line.trim();
    if starts_with_word(said, "asked") {
        return Phase::Asked;
    }
    if starts_with_word(said, "waiting") {
        // The number just before "ahead".
        let words: Vec<&str> = said.split_whitespace().collect();
        let bare = |word: &str| word.trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
        let ahead = words
            .windows(2)
            .find(|pair| bare(pair[1]).eq_ignore_ascii_case("ahead"))
            .and_then(|pair| bare(pair[0]).parse().ok());
        return Phase::Waiting { ahead };
    }
    for (word, phase) in [
        ("running", Phase::Running),
        ("rendering", Phase::Rendering),
        ("made", Phase::Made),
    ] {
        if starts_with_word(said, word) {
            return phase;
        }
    }
    Phase::Other
}

/// Seconds as a clock: `1:05`, or `1:02:05` past the hour.
pub fn elapsed(seconds: u64) -> String {
    let (hours, minutes, rest) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// A file's name without its extension.
pub fn stem(file: &str) -> &str {
    match file.rfind('.') {
        Some(at) if at > 0 => &file[..at],
        _ => file,
    }
}

/// The chip for the makings, each its media's name, its line and the
/// seconds since it began, in the window's order: the one making ahead
/// of those waiting their turn, "queued, N ahead" while it waits, its
/// clock while it makes, and " +N" for the others. None with none.
pub fn makings(makings: &[(String, String, u64)]) -> Option<String> {
    let shown = makings
        .iter()
        .find(|(_, line, _)| !phase(line).queued())
        .or_else(|| makings.first())?;
    let (name, line, seconds) = shown;
    let mut chip = format!("Making {name} · {}", stands(line, *seconds));
    if makings.len() > 1 {
        chip.push_str(&format!(" +{}", makings.len() - 1));
    }
    Some(chip)
}

/// How a making stands, by its `line` and the `seconds` since it
/// began: "queued, 2 ahead" or "queued" while it waits its turn, else
/// its clock.
fn stands(line: &str, seconds: u64) -> String {
    match phase(line) {
        Phase::Waiting { ahead: Some(ahead) } if ahead > 0 => format!("queued, {ahead} ahead"),
        other if other.queued() => "queued".to_owned(),
        _ => elapsed(seconds),
    }
}

/// What the chip says of the makings while they run, followed in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Following {
    /// What the one shown last said, as `store::making_status` puts it
    /// ("Making m4: <line>", "Making take 2 of m4 from its steps…"):
    /// cut short on the chip where it has no room.
    pub said: String,
    /// After it, never cut: how it stands ("0:41", "queued, 2 ahead")
    /// and " +N" for the others.
    pub stands: String,
    /// Its tip: each making's words whole and how it stands, a line
    /// each, the one shown first.
    pub tip: String,
}

/// The chip for the makings, each its media's name, its take, its line
/// and the seconds since it began, in the window's order: the one
/// making ahead of those waiting their turn, as `makings` chooses.
/// None with none.
pub fn following(makings: &[(String, crate::store::Making)]) -> Option<Following> {
    following_at(makings, std::time::Instant::now())
}

/// The chip for the makings as it stands at `now`: `following`, with
/// each clock counted from when its making began to `now` (none for
/// one that begins after it).
pub fn following_at(
    makings: &[(String, crate::store::Making)],
    now: std::time::Instant,
) -> Option<Following> {
    let seconds =
        |making: &crate::store::Making| now.saturating_duration_since(making.since).as_secs();
    let first = makings
        .iter()
        .position(|(_, making)| !phase(&making.line).queued())
        .unwrap_or(0);
    let (name, shown) = makings.get(first)?;
    let mut stands = stands(&shown.line, seconds(shown));
    if makings.len() > 1 {
        stands.push_str(&format!(" +{}", makings.len() - 1));
    }
    let line = |(name, making): &(String, crate::store::Making)| {
        format!(
            "{} · {}",
            crate::store::making_status(name, making),
            self::stands(&making.line, seconds(making))
        )
    };
    let tip = std::iter::once(&makings[first])
        .chain(
            makings
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != first)
                .map(|(_, making)| making),
        )
        .map(line)
        .collect::<Vec<_>>()
        .join("\n");
    Some(Following {
        said: crate::store::making_status(name, shown),
        stands,
        tip,
    })
}
