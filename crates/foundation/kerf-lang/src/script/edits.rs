//! The edits of the timeline: placing clips (`insert`, `overwrite`), taking
//! ranges out (`lift`, `extract`), trimming, rolling, slipping, sliding,
//! splitting, blading, moving, and changing a clip's speed.

use super::positions::{source_clock, Positions};
use super::speech::Said;
use super::{required, Parsed, Script};
use crate::doc::content;
use crate::facts::{Snap, BEAT};
use crate::lex::{Arg, Atom};
use crate::text::{parse_param, parse_speed, Clock, Fields};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Clip, Content, Domain, Edge, NewClip, Op, Params, Rate, Time};

impl Script<'_> {
    pub(super) fn place(&mut self, mut fields: Fields<'_>, ripple: bool) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let (mut content, source) = content(&mut fields, &self.project, self.catalog, clock.rate)?;
        let snap = take_snap(&mut fields, &clock)?;
        let wrote = |key: &str| line.args.iter().any(|arg| arg.key.as_deref() == Some(key));
        let media = match (&content, snap) {
            (_, None) => None,
            (Content::Media { .. }, Some(_)) if !wrote("src") && !wrote("out") => {
                return Err(line.error(1, "snap= moves src= and out=; give one"))
            }
            (Content::Media { media, .. }, Some(_)) => self.project.media(media.as_str()),
            (_, Some(_)) => return Err(line.error(1, "snap= needs media=")),
        };
        if let (
            Some(media),
            Some((to, within)),
            Content::Media {
                source_in, speed, ..
            },
            true,
        ) = (media, snap, &mut content, wrote("src"))
        {
            *source_in = self
                .snap_source(media, *source_in, to, speed.played(within), "src=")
                .map_err(|m| line.error(1, m))?;
        }
        let positions = Positions::of(&self.project, s);
        let at = required(&mut fields, "at", |t| positions.position(t))?;
        let duration = match (
            fields.take_with("dur", |t| clock.duration(t))?,
            fields.take("out"),
        ) {
            (Some(_), Some(_)) => return Err(line.error(1, "give dur= or out=, not both")),
            (Some(duration), None) => duration,
            (None, Some(out)) => {
                let source =
                    source.ok_or_else(|| fields.error(out, "out= needs media= or seq="))?;
                let mut out_time = source
                    .position(fields.text(out)?)
                    .map_err(|m| fields.error(out, m))?;
                if let (Some(media), Some((to, within)), Content::Media { speed, .. }) =
                    (media, snap, &content)
                {
                    out_time = self
                        .snap_source(media, out_time, to, speed.played(within), "out=")
                        .map_err(|m| fields.error(out, m))?;
                }
                let (Content::Media {
                    source_in, speed, ..
                }
                | Content::Sequence {
                    source_in, speed, ..
                }) = &content
                else {
                    unreachable!("a source clock implies source content")
                };
                let span = out_time - *source_in;
                if !span.is_positive() {
                    return Err(fields.error(out, "out= must come after src="));
                }
                let frames = clock.rate.frames_round(speed.time_for(span)).max(1);
                clock.rate.time_of(frames)
            }
            (None, None) => {
                return Err(line.error(1, "missing `dur=` (or `out=` for a source out point)"))
            }
        };
        let tracks = fields.take_ids("on")?.ok_or_else(|| {
            line.error(
                1,
                "missing `on=` with the target track, such as on=V1 or on=V1,A1",
            )
        })?;
        let names = fields.take_ids("as")?.unwrap_or_default();
        if names.len() > tracks.len() {
            return Err(line.error(1, "as= names more clips than on= has tracks"));
        }
        let name = fields.take_text("name")?.map(str::to_owned);
        let domains: Vec<Option<Domain>> = tracks
            .iter()
            .map(|track| sequence.track(track.as_str()).and_then(|t| t.kind.domain()))
            .collect();
        let mut props: Vec<Params> = vec![Params::new(); tracks.len()];
        for (key, arg) in fields.rest() {
            if let Content::Generator { fx, params, .. } = &mut content
                && let Some(def) = self.catalog.fx(fx.as_str()).and_then(|d| d.find(key))
            {
                params.insert(
                    key.to_owned(),
                    parse_param(&arg.value, def, clock.rate).map_err(|m| fields.error(arg, m))?,
                );
                continue;
            }
            let mut accepted = false;
            for (target, domain) in props.iter_mut().zip(&domains) {
                if let Some(def) = domain.and_then(|d| self.catalog.prop(d, key)) {
                    target.insert(
                        key.to_owned(),
                        parse_param(&arg.value, def, clock.rate)
                            .map_err(|m| fields.error(arg, m))?,
                    );
                    accepted = true;
                }
            }
            if accepted {
                continue;
            }
            // A track that is not there takes nothing, and that is what is
            // wrong with the line: placing the clip says so, with the tracks
            // there are and the line that adds this one.
            if tracks
                .iter()
                .any(|track| sequence.track(track.as_str()).is_none())
            {
                continue;
            }
            // What `set` writes on a clip that is there, not a property a
            // track takes.
            if matches!(key, "parent" | "enabled" | "caption" | "retime" | "angle") {
                let clip = names.first().map_or("<clip>", |id| id.as_str());
                return Err(fields.error(
                    arg,
                    format!(
                        "`{key}=` is set on a clip once it is placed, not where it is placed: \
                         after this line, `set {clip} {key}={}`",
                        fields.text(arg).unwrap_or_default()
                    ),
                ));
            }
            return Err(fields.error(arg, format!("no target track takes `{key}`")));
        }
        fields.finish(1)?;
        let targets = tracks
            .into_iter()
            .zip(props)
            .enumerate()
            .map(|(i, (track, props))| {
                (
                    track,
                    NewClip {
                        id: names.get(i).cloned(),
                        content: content.clone(),
                        name: name.clone(),
                        props,
                    },
                )
            })
            .collect();
        let sequence = sequence.id.clone();
        Ok(vec![if ripple {
            Op::Insert {
                sequence,
                at,
                duration,
                targets,
            }
        } else {
            Op::Overwrite {
                sequence,
                at,
                duration,
                targets,
            }
        }])
    }

    pub(super) fn trim(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let edge = match fields.word(2, "in or out")? {
            "in" => Edge::In,
            "out" => Edge::Out,
            other => {
                return Err(line.error(
                    fields.positional()[2].column,
                    format!("trim {other}? use in or out"),
                ))
            }
        };
        let (found, clock) = self.clip(&clip, line)?;
        let edge_time = if edge == Edge::In {
            found.start
        } else {
            found.end()
        };
        let rest: Vec<&Arg> = fields.positional()[3..].to_vec();
        let ripple = rest
            .iter()
            .any(|arg| arg.value == Atom::Word("ripple".into()));
        let amounts: Vec<&&Arg> = rest
            .iter()
            .filter(|arg| arg.value != Atom::Word("ripple".into()))
            .collect();
        let to = match (
            fields.take_with("to", |t| clock.position(t))?,
            fields.take("src"),
        ) {
            (Some(_), Some(_)) => return Err(line.error(1, "give to= or src=, not both")),
            (Some(to), None) => Some(to),
            (None, Some(src)) => {
                let text = fields.text(src)?;
                Some(
                    self.source_edge(found, clock.rate, text)
                        .map_err(|m| fields.error(src, m))?,
                )
            }
            (None, None) => None,
        };
        let delta = match (to, amounts.as_slice()) {
            (Some(to), []) => to - edge_time,
            (None, [amount]) => {
                let text = fields.text(amount)?;
                clock.delta(text).map_err(|m| fields.error(amount, m))?
            }
            (Some(_), _) => return Err(line.error(1, "give an amount, to=, or src=, not more")),
            (None, []) => {
                return Err(line.error(
                    1,
                    "trim needs an amount like -6f, to=<time>, or src=<source time>",
                ))
            }
            (None, [_, extra, ..]) => return Err(fields.error(extra, "unexpected argument")),
        };
        let delta = match take_snap(&mut fields, &clock)? {
            Some(snap) => {
                // The trimmed clip must keep a length.
                let keeps = |at: Time| match edge {
                    Edge::In => at < found.end(),
                    Edge::Out => at > found.start,
                };
                let what = format!(
                    "the {} of `{clip}`",
                    if edge == Edge::In { "in" } else { "out" }
                );
                self.snap_edge(found, edge_time + delta, snap, clock.rate, keeps, &what)
                    .map_err(|m| line.error(1, m))?
                    - edge_time
            }
            None => delta,
        };
        let count = fields.positional().len();
        fields.finish(count)?;
        Ok(vec![Op::Trim {
            clip,
            edge,
            delta,
            ripple,
        }])
    }

    /// Where on the sequence `clip` plays the source position `text`, on
    /// the nearest frame: where a trim to that source position puts the
    /// edge.
    fn source_edge(&self, clip: &Clip, rate: Rate, text: &str) -> Result<Time, String> {
        let source = source_clock(&self.project, clip)?
            .ok_or_else(|| format!("clip `{}` has no source; use to=", clip.id))?;
        if clip.speed().is_reverse() {
            return Err(format!("clip `{}` plays in reverse; use to=", clip.id));
        }
        let played = clip.played_at(source.position(text)?).expect(
            "source_clock is Some only for media and nested-sequence clips, which play a source",
        );
        Ok(rate.round(played))
    }

    /// `lift` and `extract`: a range of the sequence between two positions,
    /// or, with a clip named, what the clip says (`said=`), the pause after
    /// what it says (`pause=`), or two positions moved by its sound
    /// (`snap=`).
    pub(super) fn take_out(&mut self, mut fields: Fields<'_>, close: bool) -> Parsed {
        let line = fields.line();
        let named = fields.positional().len() > 1;
        let (s, from, to) = if named {
            let clip = fields.id(1, "a clip id")?;
            let (found, positions) = self.clip(&clip, line)?;
            let clock = &positions.clock;
            let (said, pause) = (fields.take("said"), fields.take("pause"));
            let from = fields.take_with("from", |t| positions.position(t))?;
            let to = fields.take_with("to", |t| positions.position(t))?;
            let leave = fields.take_with("leave", |t| clock.duration(t))?;
            let range =
                match (said, pause, from, to) {
                    (Some(_), Some(_), ..) => {
                        return Err(line.error(1, "give said= or pause=, not both"))
                    }
                    (Some(arg), None, None, None) | (None, Some(arg), None, None) => {
                        if pause.is_some() && !close {
                            return Err(fields.error(arg, "pause= closes a pause up: use extract"));
                        }
                        let within = fields
                            .take_with("within", |text| clock.duration(text))?
                            .unwrap_or_else(|| Time::from_flicks(FLICKS_PER_SECOND / 2));
                        let cut = Said {
                            phrase: fields.text(arg)?,
                            pause: pause.is_some(),
                            leave: leave.map_or(BEAT, Time::seconds),
                            within,
                        };
                        self.said(found, clock, &cut)
                            .map_err(|m| fields.error(arg, m))?
                    }
                    (None, None, Some(from), Some(to)) => {
                        if leave.is_some() {
                            return Err(line.error(1, "leave= goes with said= or pause="));
                        }
                        let snap = take_snap(&mut fields, clock)?.ok_or_else(|| {
                            line.error(
                                1,
                                format!(
                                "naming `{clip}` puts the edges by its sound: add snap=pause or \
                                 snap=cut, or leave the clip out"
                            ),
                            )
                        })?;
                        for (key, at) in [("from", from), ("to", to)] {
                            if at < found.start || at > found.end() {
                                return Err(line.error(
                                    1,
                                    format!(
                                        "{key}= is not in `{clip}`, which plays from {} to {}",
                                        clock.print_position(found.start),
                                        clock.print_position(found.end())
                                    ),
                                ));
                            }
                        }
                        let rate = clock.rate;
                        let starts = |at: Time| found.start <= at && at < found.end();
                        let ends = |at: Time| found.start < at && at <= found.end();
                        (
                            self.snap_edge(found, from, snap, rate, starts, "from=")
                                .map_err(|m| line.error(1, m))?,
                            self.snap_edge(found, to, snap, rate, ends, "to=")
                                .map_err(|m| line.error(1, m))?,
                        )
                    }
                    _ => return Err(line.error(
                        1,
                        "with a clip, give said=\"…\", pause=\"…\", or from= and to= with snap=",
                    )),
                };
            let at = self.project.locate(clip.as_str());
            (
                at.expect("Script::clip found it above, and nothing since changed the project")
                    .sequence,
                range.0,
                range.1,
            )
        } else {
            for key in ["snap", "said", "pause", "leave"] {
                if let Some(arg) = fields.take(key) {
                    return Err(fields.error(
                        arg,
                        format!(
                            "{key}= goes by a clip's sound and words: name the clip, as in \
                             `extract c1 said=\"…\"` or `extract c1 from=… to=… snap=pause`"
                        ),
                    ));
                }
            }
            let s = self.current(line)?;
            let clock = Positions::of(&self.project, s);
            let from = required(&mut fields, "from", |t| clock.position(t))?;
            let to = required(&mut fields, "to", |t| clock.position(t))?;
            (s, from, to)
        };
        let tracks = fields.take_ids("tracks")?;
        fields.finish(if named { 2 } else { 1 })?;
        let sequence = self.project.sequences[s].id.clone();
        Ok(vec![if close {
            Op::Extract {
                sequence,
                from,
                to,
                tracks,
            }
        } else {
            Op::Lift {
                sequence,
                from,
                to,
                tracks,
            }
        }])
    }

    /// `roll`, `slip`, or `slide <clip>` by an amount, or a roll or slide `to=` a time: the
    /// cut after the clip, the part of its source it plays, or where it sits between the clips
    /// either side. A roll may go `alone`, and may snap.
    pub(super) fn roll_slip_slide(&mut self, mut fields: Fields<'_>, command: &str) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        // A roll of the clip's own track alone, its linked clips' cut staying: a J or L cut.
        let alone_word = Atom::Word("alone".into());
        let alone = command == "roll"
            && fields.positional()[2..]
                .iter()
                .any(|arg| arg.value == alone_word);
        let rest: Vec<&Arg> = fields.positional()[2..]
            .iter()
            .copied()
            .filter(|arg| !(alone && arg.value == alone_word))
            .collect();
        let to = fields.take_with("to", |t| clock.position(t))?;
        if let (Some(_), Some(extra)) = (to, rest.first()) {
            return Err(fields.error(extra, "give an amount or to=, not both"));
        }
        let delta = match (to, rest.as_slice()) {
            (Some(to), _) if command == "roll" => to - found.end(),
            (Some(to), _) if command == "slide" => to - found.start,
            (Some(_), _) => return Err(line.error(1, "slip takes an amount like +12f, not to=")),
            (None, [amount, ..]) => {
                let text = fields.text(amount)?;
                clock.delta(text).map_err(|m| fields.error(amount, m))?
            }
            (None, []) => clock
                .delta(fields.word(2, "an amount like +12f or -0.5s")?)
                .map_err(|m| line.error(fields.positional()[2].column, m))?,
        };
        let count = fields.positional().len();
        if let Some(extra) = rest.get(1) {
            return Err(fields.error(extra, "unexpected argument"));
        }
        // A slip or a slide moves no cut between two clips: snap= is left to refuse.
        let snap = match command {
            "roll" => take_snap(&mut fields, &clock)?,
            _ => None,
        };
        let delta = match snap {
            Some(snap) => {
                let s = self
                    .project
                    .locate(clip.as_str())
                    .expect("Script::clip found it above, and nothing since changed the project")
                    .sequence;
                self.snap_roll(s, found, delta, alone, snap)
                    .map_err(|m| line.error(1, m))?
            }
            None => delta,
        };
        fields.finish(count)?;
        Ok(vec![match command {
            "roll" => Op::Roll { clip, delta, alone },
            "slip" => Op::Slip { clip, delta },
            _ => Op::Slide { clip, delta },
        }])
    }

    /// `split <clip> at=…`: the clip cut in two there, the second part named by `as=`.
    pub(super) fn split(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        let mut at = required(&mut fields, "at", |t| clock.position(t))?;
        let tail = fields.take_id("as")?;
        if let Some(snap) = take_snap(&mut fields, &clock)? {
            // A split must fall inside the clip.
            let what = format!("the split of `{clip}`");
            at = self
                .snap_edge(
                    found,
                    at,
                    snap,
                    clock.rate,
                    |at| found.start < at && at < found.end(),
                    &what,
                )
                .map_err(|m| line.error(1, m))?;
        }
        fields.finish(2)?;
        Ok(vec![Op::Split { clip, at, tail }])
    }

    /// `blade at=…`: the clips of the sequence there cut, on every track or on those
    /// `tracks=` names.
    pub(super) fn blade(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let s = self.current(line)?;
        let clock = Positions::of(&self.project, s);
        let mut at = required(&mut fields, "at", |t| clock.position(t))?;
        let tracks = fields.take_ids("tracks")?;
        if let Some(snap) = take_snap(&mut fields, &clock)? {
            at = self
                .snap_blade(s, at, tracks.as_deref(), snap)
                .map_err(|m| line.error(1, m))?;
        }
        fields.finish(1)?;
        Ok(vec![Op::Blade {
            sequence: self.project.sequences[s].id.clone(),
            at,
            tracks,
        }])
    }

    /// `move <clip>` onto a track (`on=`), to a time (`at=`), or by an amount (`by=`).
    pub(super) fn move_clip(&mut self, mut fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let (found, clock) = self.clip(&clip, line)?;
        let track = fields.take_id("on")?;
        let at = fields.take_with("at", |t| clock.position(t))?;
        let by = fields.take_with("by", |t| clock.delta(t))?;
        let delta = match (at, by) {
            (Some(_), Some(_)) => return Err(line.error(1, "give at= or by=, not both")),
            (Some(at), None) => at - found.start,
            (None, Some(by)) => by,
            (None, None) if track.is_some() => Time::ZERO,
            (None, None) => {
                return Err(line.error(1, "move needs on=<track>, at=<time>, or by=<amount>"))
            }
        };
        fields.finish(2)?;
        Ok(vec![Op::Move { clip, track, delta }])
    }

    /// `speed <clip> <speed>`: how fast the clip plays, a number or a ramp.
    pub(super) fn speed(&mut self, fields: Fields<'_>) -> Parsed {
        let line = fields.line();
        let clip = fields.id(1, "a clip id")?;
        let arg = fields.positional().get(2).copied().ok_or_else(|| {
            line.error(1, "expected a speed such as 2, 0.5, -1, or {0f:1 12f:1/2}")
        })?;
        let rate = self
            .project
            .locate(clip.as_str())
            .map_or(Rate::FPS_24, |at| self.project.sequences[at.sequence].rate);
        let speed = parse_speed(&arg.value, rate).map_err(|m| fields.error(arg, m))?;
        fields.finish(3)?;
        Ok(vec![Op::Speed { clip, speed }])
    }
}

/// `snap=pause|cut` and `within=<amount>`, half a second unless given: what
/// an edge snaps to and how far it may move.
fn take_snap(fields: &mut Fields<'_>, clock: &Clock) -> Parsed<Option<(Snap, Time)>> {
    let line = fields.line();
    let snap = fields.take_with("snap", |text| match text {
        "pause" => Ok(Snap::Pause),
        "cut" => Ok(Snap::Cut),
        other => Err(format!("snap={other}? use pause or cut")),
    })?;
    let within = fields.take_with("within", |text| clock.duration(text))?;
    match (snap, within) {
        (Some(snap), within) => Ok(Some((
            snap,
            within.unwrap_or_else(|| Time::from_flicks(FLICKS_PER_SECOND / 2)),
        ))),
        (None, Some(_)) => Err(line.error(1, "within= goes with snap=")),
        (None, None) => Ok(None),
    }
}
