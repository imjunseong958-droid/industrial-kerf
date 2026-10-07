//! The language reference given to agents.
//!
//! The grammar is fixed text kept beside the parser; every property, effect,
//! transition, and generator comes from the catalog, so the reference never
//! drifts from what the kernel accepts.

use crate::text::{number, print_value};
use cut_model::{Catalog, Domain, FxKind, MarkerColor, ParamDef, ParamType};
use std::fmt::Write as _;

const GRAMMAR: &str = r#"# Cut edit script

A project is a `.cut` document. Change it with a script: one operation per
line, applied in order and atomically. Later lines see what earlier lines did.

## Times
- Positions are sequence timecode (`01:00:04:12`, drop-frame `01:00:04;12`)
  or offsets from the sequence start (`96f`, `4.5s`), or anchors: `end` (the
  end of the sequence), `c3.start`, or `c3.end`, each with an optional
  amount, such as `at=end`, `at=c3.end`, or `to=c3.start-6f`. Prefer anchors
  to computing timecode.
- Source positions (`src=`, `out=`) are media timecode or offsets from the
  media's first frame.
- Durations: `00:00:02:12`, `60f`, or `2.5s`. Amounts: `+12f`, `-6f`, `-0.5s`.
  Seconds round to the nearest frame.
- Keyframes use clip-relative times: `opacity={0f:0 12f:1:ease}`.
  Interpolation: linear (default), hold, ease-in, ease-out, ease.

## Operations
```
use <sequence>                          choose the sequence when there are several
insert <content> at=<pos> dur=<dur>|out=<src> on=<track>[,<track>] [as=<id>[,<id>]] [name="…"] [<prop>=<value> …]
                                        open time on every unlocked track; several tracks get linked clips
overwrite <same arguments as insert>    replace what occupies the range on the target tracks
    <content> is media=<id> [src=<src>] [speed=<ratio>] | seq=<id> [src=<src>] | gen=<generator> | caption="…"
delete <clip> …                         remove clips (and linked partners), leaving gaps
ripple-delete <clip> …                  remove and close the gap on every unlocked track
lift from=<pos> to=<pos> [tracks=…]     clear a range, leaving a gap
extract from=<pos> to=<pos> [tracks=…]  cut a range out and close it
trim <clip> in|out <amount>|to=<pos>|src=<src> [ripple]
                                        src= puts the edge on that source position, such as a word's time
roll <clip> <amount>|to=<pos>           move the cut after the clip
slip <clip> <amount>                    play different source frames in the same place
slide <clip> <amount>|to=<pos>          move a clip between its neighbors
split <clip> at=<pos> [as=<id>]
blade at=<pos> [tracks=…]               split every clip crossing a point
move <clip> [on=<track>] [at=<pos>|by=<amount>]
set <clip> <prop>=<value> … [name="…"] [enabled=false] [caption="…"] [reset=<prop>,…]
speed <clip> <ratio>                    2, 0.5, or -1 for reverse; keeps the source range
fx <clip> add <effect> [at=<n>] [<param>=<value> …]
fx <clip> set <n|effect> [enabled=…] [reset=…] [<param>=<value> …]
fx <clip> remove <n|effect>  ·  fx <clip> move <n|effect> to=<n>
transition add from=<clip> to=<clip> fx=<transition> dur=<dur> [align=center|start|end] [as=<id>] [<param>=<value> …]
transition set <id> [dur=…] [align=…] [<param>=<value> …]  ·  transition remove <id>
marker add at=<pos> [dur=<dur>] [name="…"] [color=<color>] [note="…"] [as=<id>]
marker set <id> …  ·  marker remove <id>
link <clip> <clip> … [as=<group>]  ·  unlink <clip> …
track add <id> video|audio|subtitle [before=<track>|after=<track>] [name="…"] [locked=true]
track set <id> [name=…] [enabled=…] [locked=…] [muted=…]  ·  track remove <id>
media add <id> path="…" rate=<fps> dur=<dur> [tc=<tc>] [video=WxH] [audio=<n>ch@<hz>]
media relink <id> path="…"  ·  media remove <id>
sequence add <id> rate=<fps> size=WxH [name="…"] [start=<tc>]  ·  sequence set|remove <id> …
```

Linked clips (a picture and its sound) move, trim, split, slip, and delete
together. Ripple edits act on every unlocked track so sync holds; lock a
track to leave it out. Edits that would cut through another track's clip are
refused with the clip named.
"#;

pub fn reference(catalog: &Catalog) -> String {
    let mut out = String::from(GRAMMAR);
    let colors: Vec<&str> = MarkerColor::ALL.iter().map(|c| c.name()).collect();
    writeln!(out, "\nMarker colors: {}.", colors.join(", ")).unwrap();
    for domain in [Domain::Video, Domain::Audio] {
        writeln!(out, "\n## {} clip properties", capitalized(domain.name())).unwrap();
        for param in catalog.props(domain) {
            writeln!(out, "- {}", describe(param)).unwrap();
        }
    }
    for (kind, title) in [
        (FxKind::Effect, "Effects"),
        (FxKind::Transition, "Transitions"),
        (FxKind::Generator, "Generators"),
    ] {
        writeln!(out, "\n## {title}").unwrap();
        for fx in catalog.all().filter(|fx| fx.kind == kind) {
            writeln!(out, "- `{}` ({}): {}", fx.id, fx.domain.name(), fx.doc).unwrap();
            for param in &fx.params {
                writeln!(out, "  - {}", describe(param)).unwrap();
            }
        }
    }
    out
}

fn describe(param: &ParamDef) -> String {
    let mut text = format!("`{}={}`", param.name, print_value(&param.default));
    let mut facts = Vec::new();
    match &param.ty {
        ParamType::Enum(options) => facts.push(options.join("|")),
        ParamType::Number => {
            if let (Some(min), Some(max)) = (param.min, param.max) {
                facts.push(format!("{}..{}", number(min), number(max)));
            }
        }
        other => facts.push(other.name().to_owned()),
    }
    if let Some(unit) = &param.unit {
        facts.push(unit.clone());
    }
    if param.animatable {
        facts.push("animatable".into());
    }
    write!(text, " ({})", facts.join(", ")).unwrap();
    if !param.doc.is_empty() {
        write!(text, " {}", param.doc).unwrap();
    }
    text
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_every_catalog_entry() {
        let catalog = Catalog::builtin();
        let text = reference(catalog);
        for fx in catalog.all() {
            assert!(text.contains(&format!("`{}`", fx.id)), "{}", fx.id);
        }
        assert!(text.contains("`opacity=1` (0..1, animatable) Layer opacity."));
        assert!(text.contains("`blend=normal` (normal|add|multiply"));
        assert!(text.contains("`gain=0` (-96..24, dB, animatable)"));
    }
}
