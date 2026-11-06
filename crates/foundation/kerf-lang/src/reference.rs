//! The language reference given to agents.
//!
//! The grammar is fixed text kept beside the parser; every property, effect,
//! transition, and generator comes from the catalog, so the reference never
//! drifts from what the kernel accepts.

use crate::text::{number, print_value};
use kerf_model::{Catalog, Domain, Engine, FxDef, FxKind, MarkerColor, ParamDef, ParamType};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const GRAMMAR: &str = r#"# Kerf edit script

A project is a `.kerf` document. Change it with a script: one operation per
line, applied in order and atomically. Later lines see what earlier lines did.

## Times
- Positions are sequence timecode (`01:00:04:12`, drop-frame `01:00:04;12`)
  or offsets from the sequence start (`96f`, `4.5s`), or anchors: `end` (the
  end of the sequence), `c3.start`, `c3.end`, a marker's id (`beat1`, or
  `beat1.end` past its duration), or `c3@<media timecode>`, where clip c3
  plays that media position. Each takes an optional amount, such as
  `at=c3.end`, `to=c3.start-6f`, `at=beat1+2f`, or `at=c3@00:12:04:10`.
  Prefer anchors to computing timecode: a time seen in a clip's media is
  `c3@` that time.
- Source positions (`src=`, `out=`) are media timecode or offsets from the
  media's first frame, or a quoted phrase a transcript heard (`src="so we
  closed"`): an in point where it starts, an out point where it ends.
- Durations: `00:00:02:12`, `60f`, or `2.5s`. Amounts: `+12f`, `-6f`, `-0.5s`.
  Seconds round to the nearest frame.
- Keyframes use clip-relative times: `opacity={0f:0 12f:1:ease}`.
  Interpolation: linear (default), hold, ease-in, ease-out, ease, or a curve
  `x1,y1,x2,y2` that may overshoot: `{0f:1 12f:1.2:0.2,0.8,0.4,1}`.
- `parent=c1` on a clip moves it with `c1`, which `gen=null@1` is for.
- `retime=flow` on footage draws new frames between its own for slow motion; `retime=blend` fades.
- `rect=`, `ellipse=`, and `points=` on a mask animate: `rect={0f:0,0,0.2,0.2 24f:0.5,0,0.7,0.2:ease}`,
  `points={0f:0,0;0.5,0;0.5,0.5 24f:0.1,0;0.6,0;0.6,0.5}`, corners one for one.
- `angle=V2` on a clip of a nested sequence shows or plays that track alone; `angle=""` undoes it. `look` with `sync` lines cameras up by sound.
- `speed={0f:1 24f:1 36f:1/4}` changes speed over a clip, keyframes as above but with no overshoot.

## Versions
Effects, transitions, and generators have versions, listed below as
`id@version`. Documents keep the version an edit used (`fx c1 add
blur.gaussian@1`); leave it out to take the current one.

## Operations
```
use <sequence>                          choose the sequence when there are several
insert <content> at=<pos> dur=<dur>|out=<src> on=<track>[,<track>] [as=<id>[,<id>]] [name="…"] [<prop>=<value> …]
                                        open time on every unlocked track; several tracks get linked clips
overwrite <same arguments as insert>    replace what occupies the range on the target tracks
    <content> is media=<id> [src=<src>] [speed=<speed>] | seq=<id> [src=<src>] | gen=<generator> | caption="…"
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
speed <clip> <speed>                    2, 0.5, -1 for reverse, or {0f:1 24f:1/4} to change over the clip; keeps the source range
fx <clip> add <effect> [at=<n>] [<param>=<value> …]
fx <clip> set <n|effect> [enabled=…] [reset=…] [<param>=<value> …]
fx <clip> remove <n|effect>  ·  fx <clip> move <n|effect> to=<n>
mask <clip> add <name> rect=L,T,R,B|ellipse=X,Y,RX,RY|points="x,y x,y …" [closed=false] [mode=…] [inverted=true] [feather=<px>] [opacity=<0-1>] [expansion=<px>] [at=<n>]
                                        a picture clip's mask, in fractions of its picture (0,0 top left)
mask <clip> set <name> [outline] [mode=…] [inverted=…] [reset=…] [<prop>=<value> …]  ·  mask <clip> remove <name>
transition add from=<clip> to=<clip> fx=<transition> dur=<dur> [align=center|start|end] [as=<id>] [<param>=<value> …]
transition set <id> [dur=…] [align=…] [<param>=<value> …]  ·  transition remove <id>
marker add at=<pos> [dur=<dur>] [name="…"] [color=<color>] [note="…"] [as=<id>]
marker set <id> …  ·  marker remove <id>
captions <clip> on=<subtitle track> [words=<n>]
                                        caption what the clip says, timed to its speech, from a
                                        transcript of its media made earlier; it runs as the caption lines
link <clip> <clip> … [as=<group>]  ·  unlink <clip> …
track add <id> video|audio|subtitle [before=<track>|after=<track>] [name="…"] [locked=true]
track set <id> [name=…] [enabled=…] [locked=…] [muted=…]  ·  track remove <id>
media add <id> path="…" rate=<fps> dur=<dur> [tc=<tc>] [video=WxH [alpha=true]] [audio=<n>ch@<hz>] [reel="…"]
media replace <id> <same arguments as media add>
                                        swap the file behind a media id; clips keep their timing and
                                        must still fit (length, streams, transition handles)
media relink <id> path="…"  ·  media remove <id>
sequence add <id> rate=<fps> size=WxH [name="…"] [start=<tc>] [color=rec2100-pq|rec2100-hlg]  ·  sequence set|remove <id> …
```

`snap=pause|cut [within=<amount>]` on trim, split, insert, and overwrite
moves the edge into the nearest pause of the media's sound, or onto a cut of
its picture, by its index; within half a second unless given.

Masks combine in order by `mode` (add, subtract, intersect, lighten,
darken, difference); `mode=none` cuts nothing and leaves a path that
effects such as `generate.audiowaveform path=<name>` read. Linked clips (a picture
and its sound) move, trim, split, slip, and delete together. Ripple edits act on every unlocked track so sync holds; lock a
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
        for fx in catalog
            .all()
            .filter(|fx| fx.kind == kind && fx.engine == Engine::Graph)
        {
            out.push_str(&fx_reference(fx));
        }
    }
    for (kind, text) in [
        (FxKind::Transition, KERF_TRANSITIONS),
        (FxKind::Effect, KERF_EFFECTS),
    ] {
        writeln!(out, "\n{text}").unwrap();
        let mut categories: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for fx in catalog
            .all()
            .filter(|fx| fx.engine == Engine::Kerf && fx.kind == kind)
        {
            categories
                .entry(fx.category.as_str())
                .or_default()
                .push(format!("`{}@{}`", fx.id, fx.version));
        }
        for (category, ids) in categories {
            writeln!(out, "- {category}: {}", ids.join(", ")).unwrap();
        }
    }
    writeln!(out, "\n{LOOKS}").unwrap();
    let mut categories: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for fx in catalog.all().filter(|fx| fx.engine == Engine::Look) {
        categories
            .entry(fx.category.as_str())
            .or_default()
            .push(format!("`{}@{}`", fx.id, fx.version));
    }
    for (category, ids) in categories {
        writeln!(out, "- {category}: {}", ids.join(", ")).unwrap();
    }
    writeln!(out, "\n{RASTER}").unwrap();
    let mut categories: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for fx in catalog.all().filter(|fx| fx.engine == Engine::Raster) {
        categories
            .entry(fx.category.as_str())
            .or_default()
            .push(fx.id.as_str());
    }
    for (category, ids) in categories {
        let ids: Vec<String> = ids.iter().map(|id| format!("`{id}`")).collect();
        writeln!(out, "- {category}: {}", ids.join(", ")).unwrap();
    }
    out
}

const KERF_TRANSITIONS: &str = "## Transitions from FilmCraft
A video editor's transitions, drawn from both clips across the cut: pushes,
slides, whips, zooms, flashes, light leaks, glitches, irises, and more.
Written like any transition (`transition add from=c1 to=c2 fx=push@1
dur=12f direction=from-east`); `look` with `effect: \"<id>\"` lists one's
parameters. A direction names the side the incoming clip enters from, and a
center is `center_x`/`center_y` in fractions of the frame.";

const KERF_EFFECTS: &str = "## Effects from FilmCraft
A video editor's effects, run in stack order on a clip's picture after its
masks: Lumetri grades and color correction, blurs, keys and track mattes,
glows and light leaks, distortions, moves that animate over the clip,
burn-ins, and 360° effects. Written like any effect (`fx c1 add
color.lumetri@2 exposure=0.5`); `look` with `effect: \"<id>\"` lists one's
parameters. A point is `<name>_x`/`<name>_y` across the picture (0 to 1),
an offset is in pixels, a curve is text of `x,y` points from 0 to 1, a LUT
is a quoted `.cube` or `.3dl` path, and another track is its id
(`matte=V2`), read even while it is hidden. Sizes are the clip's source
pixels. An id at version 2 was an EffectCraft effect that FilmCraft's
replaced; version 1 of it no longer reads.";

const LOOKS: &str = "## Looks
Playful effects written in the Kerf language from the effects above, behind
a few parameters (`docs/looks.md`). A clip takes one like any effect (`fx c1
add look.shake@1 intensity=0.7 hit=12f`), a title the Titles ones, and a cut
the Transitions ones like any transition. A time parameter is a time in the
clip (`12f`, `0.5s`). The document keeps the one line; `fx c1 flatten 1`
replaces the look at that place with what it expands to, for hand tuning.
A project's own looks are `.kerf/looks/*.kerflook` files.";

const RASTER: &str = "## Effects kept from EffectCraft
The EffectCraft effects Kerf keeps, which FilmCraft has nothing like:
restoration (grain, dust, rolling shutter, lens, upscaling), key and matte
clean-up, curves and finer color, a few blurs and distortions, and the
audio visualizers. They run in stack order beside FilmCraft's effects. A
file parameter is a quoted path, relative to the project; a sound parameter
names a sound track whose sound the effect draws (`audioLayer=A1`); a mask
parameter names one of the clip's masks (`path=k1`). `look` with
`effects: \"<words>\"` finds entries by the look or moment you want, each
with when to use it, and `look` with `effect: \"<id>\"` lists one entry's
parameters. An id EffectCraft had that Kerf no longer does is an error
that names what to use instead.";

/// One catalog definition as the reference lists it: its id, domain, and
/// doc, then a line per parameter.
pub fn fx_reference(fx: &FxDef) -> String {
    let mut out = format!(
        "- `{}@{}` ({}): {}\n",
        fx.id,
        fx.version,
        fx.domain.name(),
        fx.doc
    );
    for param in &fx.params {
        writeln!(out, "  - {}", describe(param)).unwrap();
    }
    out
}

fn describe(param: &ParamDef) -> String {
    let mut text = format!("`{}={}`", param.name, print_value(&param.default));
    let mut facts = Vec::new();
    match &param.ty {
        ParamType::Enum(options) => facts.push(options.join("|")),
        ParamType::Track => facts.push("a picture track of the sequence, such as V2".into()),
        ParamType::Sound => facts.push("a sound track of the sequence, such as A1".into()),
        ParamType::Mask => facts.push("one of the clip's masks, by name".into()),
        ParamType::File(extensions) => facts.push(format!(
            "a file ({}), relative to the project",
            extensions
                .iter()
                .map(|extension| format!(".{extension}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        ParamType::Number => {
            if let (Some(min), Some(max)) = (param.min, param.max) {
                facts.push(format!("{}..{}", number(min), number(max)));
            }
        }
        ParamType::Time => {
            facts.push("a time in the clip, such as 12f or 0.5s, or a marker's id".into())
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
            let listed = match fx.engine {
                Engine::Graph | Engine::Kerf | Engine::Look => {
                    format!("`{}@{}`", fx.id, fx.version)
                }
                Engine::Raster => format!("`{}`", fx.id),
            };
            assert!(text.contains(&listed), "{}", fx.id);
        }
        assert!(text.contains("`opacity=1` (0..1, animatable) Layer opacity."));
        assert!(text.contains("`blend=normal` (normal|add|multiply"));
        assert!(text.contains("`gain=0` (-96..24, dB, animatable)"));
        // Kept effects are listed by id; their parameters come from `look`.
        assert!(text.contains("- Blur & Sharpen: `blur.bilateral`"));
        assert!(!text.contains("Blend With Original"));
        assert!(text.len() < 18 * 1024, "{}", text.len());
    }
}
