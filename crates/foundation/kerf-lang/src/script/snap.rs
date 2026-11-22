//! Edges moved by what a clip's media sounds like and shows (`snap=pause`
//! or `snap=cut`): to the nearest pause or cut, on a frame, past no word,
//! with what is said of each move.

use super::speech::beside;
use super::Script;
use crate::captions::{is_a_sound_note, Word};
use crate::facts::{clear, Barred, Place, Snap, Snaps, Speech};
use crate::text::{quote, Clock};
use kerf_model::{Clip, Id, Media, Rate, Time, Track, TrackKind};
use std::fmt::Write as _;
use std::rc::Rc;

impl Script<'_> {
    /// `edge`, a time on the sequence where `clip` plays, moved to the
    /// nearest place `snap` allows in the clip's media that `keeps`, on a
    /// frame of the sequence: the frame nearest a cut, or the frame nearest
    /// the place in a pause that the clip still plays inside that pause.
    /// `what` names the edge for what is said of the move.
    pub(super) fn snap_edge(
        &self,
        clip: &Clip,
        edge: Time,
        (snap, within): (Snap, Time),
        rate: Rate,
        keeps: impl Fn(Time) -> bool,
        what: &str,
    ) -> Result<Time, String> {
        let media = self
            .media_of(clip)
            .ok_or_else(|| format!("clip `{}` plays no media to snap to", clip.id))?;
        let source = clip
            .source_at(edge)
            .expect("media_of found media for it, so it plays a source");
        let speed = clip.speed();
        let offset = edge - clip.start;
        let reach = speed.played(offset + within) - speed.played(offset);
        let played = |seconds: f64| {
            clip.played_at(media_time(seconds))
                .expect("media_of found media for it, so it plays a source")
        };
        let (found, speech) = self.snaps(media, source, snap, reach)?;
        let (at, place) = found
            .places
            .iter()
            .find_map(|place| {
                let at = match snap {
                    // An index's times are rounded to microseconds.
                    Snap::Cut => rate.round(
                        clip.played_at(media.rate.round(media_time(place.at)))
                            .expect("media_of found media for it, so it plays a source"),
                    ),
                    Snap::Pause => {
                        // A clip in reverse plays the end of a pause first.
                        let (from, to) = (played(place.room.0), played(place.room.1));
                        frame_within(rate, played(place.at), (from.min(to), from.max(to)))?
                    }
                };
                keeps(at).then_some((at, place))
            })
            .ok_or_else(|| nothing_near(media, source, snap, reach, &found.barred))?;
        let clock = Clock::of_sequence(
            &self.project.sequences[self
                .project
                .locate(clip.id.as_str())
                .expect("callers pass a clip Script::clip found in this project")
                .sequence],
        );
        self.say_snapped(what, &clock, (edge, at), media, (snap, place), &speech);
        Ok(at)
    }

    /// `at`, a time on `media`, moved to the nearest place `snap` allows
    /// within `reach`, on a frame of the media: the frame a cut starts, or
    /// the frame nearest the place in a pause that is still inside it.
    /// `what` names the edge for what is said of the move.
    pub(super) fn snap_source(
        &self,
        media: &Media,
        at: Time,
        snap: Snap,
        reach: Time,
        what: &str,
    ) -> Result<Time, String> {
        let (found, speech) = self.snaps(media, at, snap, reach)?;
        let (to, place) = found
            .places
            .iter()
            .find_map(|place| {
                let to = match snap {
                    Snap::Cut => media.rate.round(media_time(place.at)),
                    Snap::Pause => frame_within(
                        media.rate,
                        media_time(place.at),
                        (media_time(place.room.0), media_time(place.room.1)),
                    )?,
                };
                Some((to, place))
            })
            .ok_or_else(|| nothing_near(media, at, snap, reach, &found.barred))?;
        let clock = Clock::of_media(media);
        self.say_snapped(what, &clock, (at, to), media, (snap, place), &speech);
        Ok(to)
    }

    /// The places `snap` allows within `reach` of `at` on `media`, nearest
    /// first, in seconds of the media, with the quiets beside `at` that
    /// would not do, and what the media's transcripts heard where a pause
    /// is asked for: an edge is taken past no word of them.
    pub(super) fn snaps(
        &self,
        media: &Media,
        at: Time,
        snap: Snap,
        reach: Time,
    ) -> Result<(Snaps, Rc<Speech>), String> {
        let breaks = self
            .breaks
            .borrow_mut()
            .entry(media.id.clone())
            .or_insert_with(|| self.facts.breaks(media).map(Rc::new))
            .clone()
            .ok_or_else(|| {
                format!(
                    "`{}` is not indexed yet; look at it with media first",
                    media.id
                )
            })?;
        let speech = match snap {
            Snap::Pause => self.heard(media),
            // A cut of the picture is where it is, whatever is said.
            Snap::Cut => Rc::default(),
        };
        let found = breaks.snaps(snap, at.seconds(), reach.seconds(), &speech);
        Ok((found, speech))
    }

    /// Keep for [`Applied::snapped`](super::Applied::snapped) where `snap`
    /// put the edge `what`, which was asked for at `from` and is now at `to`
    /// on `clock`, in `place` of `media`, when it moved: a move is silent in
    /// the document, and one that left a word in or out is not what was
    /// asked for.
    fn say_snapped(
        &self,
        what: &str,
        clock: &Clock,
        moved: (Time, Time),
        media: &Media,
        (snap, place): (Snap, &Place),
        speech: &Speech,
    ) {
        let onto = match snap {
            Snap::Cut => "onto",
            Snap::Pause => "into",
        };
        let place = place_said(media, (snap, place), speech);
        self.say_went(what, clock, moved, &format!("{onto} {place}"));
    }

    /// Keep for [`Applied::snapped`](super::Applied::snapped) that `what`,
    /// asked for at `from`, went to `to` on `clock`, `into` what, when it
    /// moved.
    fn say_went(&self, what: &str, clock: &Clock, (from, to): (Time, Time), into: &str) {
        let frames = clock.rate.frames_round(to - from);
        if frames == 0 {
            return;
        }
        self.snapped.borrow_mut().push(format!(
            "{what} went {} frame{} {}, to {}: {into}",
            frames.abs(),
            if frames.abs() == 1 { "" } else { "s" },
            if frames < 0 { "earlier" } else { "later" },
            clock.print_position(to),
        ));
    }

    /// `at`, where a blade was asked for on sequence `s`, moved to the
    /// nearest place `snap` allows where it still cuts what it cut there and
    /// nothing else, on one frame for every track: a blade is one cut.
    ///
    /// A pause is a place every sound the blade cuts is quiet in at once.
    /// Those are the clips it cuts on sound tracks, or where it cuts none
    /// there, the media of the picture clips it cuts, as a split of one goes
    /// by. Each takes the blade past no word of its own, as it would a split
    /// of it, so sounds with no quiet in common refuse the blade, and what
    /// each would take, and what of another is in the way there, is said. A
    /// cut is the nearest of any picture the blade cuts.
    pub(super) fn snap_blade(
        &self,
        s: usize,
        at: Time,
        tracks: Option<&[Id]>,
        (snap, within): (Snap, Time),
    ) -> Result<Time, String> {
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let rate = sequence.rate;
        // The tracks it cuts, as the kernel takes them: one that is missing
        // or locked is the kernel's to refuse.
        let cut_tracks: Vec<&Track> = match tracks {
            None => sequence
                .tracks
                .iter()
                .filter(|track| !track.locked)
                .collect(),
            Some(ids) => {
                let named: Vec<&Track> = ids
                    .iter()
                    .filter_map(|id| sequence.track(id.as_str()))
                    .filter(|track| !track.locked)
                    .collect();
                if named.len() < ids.len() {
                    return Ok(at);
                }
                named
            }
        };
        // It stays inside each clip it was asked to cut and goes into no
        // other, so that it cuts the same clips wherever it lands.
        let crosses = |clip: &Clip| clip.start < at && at < clip.end();
        let frame = rate.frame_duration();
        let mut cut: Vec<(&Track, &Clip)> = Vec::new();
        let mut low: Option<(Time, &Clip)> = None;
        let mut high: Option<(Time, &Clip)> = None;
        for &track in &cut_tracks {
            for clip in &track.clips {
                let (from, to) = if crosses(clip) {
                    cut.push((track, clip));
                    (Some(clip.start + frame), Some(clip.end() - frame))
                } else if clip.end() <= at {
                    (Some(clip.end()), None)
                } else {
                    (None, Some(clip.start))
                };
                if let Some(from) = from.filter(|from| low.is_none_or(|(low, _)| *from > low)) {
                    low = Some((from, clip));
                }
                if let Some(to) = to.filter(|to| high.is_none_or(|(high, _)| *to < high)) {
                    high = Some((to, clip));
                }
            }
        }
        let bladed = self.goes_by(&cut, at, (snap, within))?;
        let Some((low, high)) = low.zip(high).filter(|_| !bladed.is_empty()) else {
            return Err(format!(
                "nothing the blade cuts at {} has {} to snap to",
                clock.print_position(at),
                match snap {
                    Snap::Pause => "a sound",
                    Snap::Cut => "a picture",
                }
            ));
        };
        let refuses = |clip: &Clip| {
            let would = if clip.start < at && at < clip.end() {
                format!("not cut `{}`", clip.id)
            } else {
                format!("cut `{}` as well", clip.id)
            };
            format!(
                "where it would {would}: blade there with at=, or leave a track out with tracks="
            )
        };
        let span = Span {
            at,
            low: (low.0, refuses(low.1)),
            high: (high.0, refuses(high.1)),
            name: "blade",
            apart: "cut them one at a time with tracks=",
        };
        let (to, into) = self.snap_place(&bladed, &span, &clock, snap, within)?;
        self.say_went("the blade", &clock, (at, to), &into);
        Ok(to)
    }

    /// `delta`, how far a roll of the cut after `clip` on sequence `s` was asked to move it,
    /// moved to where `snap` allows in every clip the roll cuts at once: the clip and the one
    /// after it on its track, and, unless `alone`, on each track where a clip linked to it ends
    /// with it. Each is taken as it would play across the two, the one before on past where it
    /// ends and the one after from before where it starts, and the roll goes through them as a
    /// blade would: into a quiet of every sound it cuts together, past no word of any, or onto
    /// the nearest cut of their pictures. Each clip keeps a frame. A roll the kernel would
    /// refuse is left for it to.
    pub(super) fn snap_roll(
        &self,
        s: usize,
        clip: &Clip,
        delta: Time,
        alone: bool,
        (snap, within): (Snap, Time),
    ) -> Result<Time, String> {
        let sequence = &self.project.sequences[s];
        let clock = Clock::of_sequence(sequence);
        let frame = sequence.rate.frame_duration();
        let cut = clip.end();
        let at = cut + delta;
        let mut pairs: Vec<(&Track, &Clip, &Clip)> = Vec::new();
        for track in &sequence.tracks {
            for (index, outgoing) in track.clips.iter().enumerate() {
                let rolled = outgoing.id == clip.id
                    || (!alone && clip.link.is_some() && outgoing.link == clip.link);
                if !rolled || outgoing.end() != cut {
                    continue;
                }
                if let Some(incoming) = track.clips.get(index + 1).filter(|next| next.start == cut)
                {
                    pairs.push((track, outgoing, incoming));
                }
            }
        }
        if pairs.is_empty()
            || pairs
                .iter()
                .any(|(_, outgoing, incoming)| at <= outgoing.start || at >= incoming.end())
        {
            return Ok(delta);
        }
        // Each pair as it would play across the whole of the two.
        let across: Vec<(&Track, Clip, Clip)> = pairs
            .iter()
            .map(|(track, outgoing, incoming)| {
                let mut on = (*outgoing).clone();
                on.move_tail(incoming.end() - outgoing.end());
                let mut from = (*incoming).clone();
                from.move_head(outgoing.start - incoming.start);
                (*track, on, from)
            })
            .collect();
        let cut_clips: Vec<(&Track, &Clip)> = across
            .iter()
            .flat_map(|(track, on, from)| [(*track, on), (*track, from)])
            .collect();
        let rolled = self.goes_by(&cut_clips, at, (snap, within))?;
        if rolled.is_empty() {
            return Err(format!(
                "nothing the roll cuts at {} has {} to snap to",
                clock.print_position(at),
                match snap {
                    Snap::Pause => "a sound",
                    Snap::Cut => "a picture",
                }
            ));
        }
        let keeps = |clip: &Clip| format!("where `{}` would keep no frame", clip.id);
        let (low, first) = pairs
            .iter()
            .map(|(_, outgoing, _)| (outgoing.start + frame, *outgoing))
            .max_by_key(|(low, _)| *low)
            .expect("no pairs returned early above");
        let (high, last) = pairs
            .iter()
            .map(|(_, _, incoming)| (incoming.end() - frame, *incoming))
            .min_by_key(|(high, _)| *high)
            .expect("no pairs returned early above");
        let span = Span {
            at,
            low: (low, keeps(first)),
            high: (high, keeps(last)),
            name: "roll",
            apart: if alone {
                "roll it another amount, or without snap="
            } else {
                "roll one track alone, or without snap="
            },
        };
        let (to, into) = self.snap_place(&rolled, &span, &clock, snap, within)?;
        if to == cut && at != cut {
            // Back where it is, which changes nothing: say so rather than nothing.
            return Err(format!(
                "the nearest place for the roll within {:.2}s of {} is the cut where it is, at {}, \
                 {into}: roll further, give a longer within=, or roll without snap=",
                within.seconds(),
                clock.print_position(at),
                clock.print_position(cut),
            ));
        }
        self.say_went("the roll", &clock, (at, to), &into);
        Ok(to - cut)
    }

    /// The clips of `cut` that `snap` goes by at `at`, each with where `at` is in its media and
    /// the places beside it within `within`: for a pause the sounds, or where it cuts none the
    /// sound of the pictures' media, as a split of one goes by; for a cut the pictures. Clips that
    /// play one media together, as a picture and its sound do, are one.
    fn goes_by<'c>(
        &'c self,
        cut: &[(&Track, &'c Clip)],
        at: Time,
        (snap, within): (Snap, Time),
    ) -> Result<Vec<Bladed<'c>>, String> {
        let with = |has: fn(&Media) -> bool| -> Vec<(&'c Clip, &'c Media)> {
            let media = |clip| self.media_of(clip).filter(|media| has(media));
            cut.iter()
                .filter_map(|(_, clip)| Some((*clip, media(clip)?)))
                .collect()
        };
        let goes_by: Vec<(&Clip, &Media)> = match snap {
            Snap::Pause => {
                let mut sounds = Vec::new();
                for (track, clip) in cut {
                    if track.kind != TrackKind::Audio {
                        continue;
                    }
                    // What sounds and has no index cannot be said to be quiet.
                    let media = self.media_of(clip).ok_or_else(|| {
                        format!(
                            "clip `{}` sounds and plays no media to snap to: leave its track \
                             out with tracks=",
                            clip.id
                        )
                    })?;
                    sounds.push((*clip, media));
                }
                if sounds.is_empty() {
                    sounds = with(|media| media.audio.is_some());
                }
                sounds
            }
            Snap::Cut => with(|media| media.video.is_some()),
        };
        let mut bladed: Vec<Bladed<'_>> = Vec::new();
        for (clip, media) in goes_by {
            let source = clip
                .source_at(at)
                .expect("goes_by keeps only clips that play media");
            let speed = clip.speed();
            // Clips that play one media together, as a picture and its
            // sound do, are one to go by.
            if bladed.iter().any(|other| {
                other.media.id == media.id && other.source == source && other.clip.speed() == speed
            }) {
                continue;
            }
            let offset = at - clip.start;
            let reach = speed.played(offset + within) - speed.played(offset);
            let (found, speech) = self.snaps(media, source, snap, reach)?;
            bladed.push(Bladed {
                clip,
                media,
                source,
                reach,
                found,
                speech,
            });
        }
        Ok(bladed)
    }

    /// Where something that cuts the clips of `bladed` at once goes, within `span`: into a quiet
    /// of each of them together, or onto the nearest cut of any picture of them, as a frame of
    /// `clock`, with what is said of where it went.
    fn snap_place(
        &self,
        bladed: &[Bladed<'_>],
        span: &Span,
        clock: &Clock,
        snap: Snap,
        within: Time,
    ) -> Result<(Time, String), String> {
        let (rate, at) = (clock.rate, span.at);
        Ok(match snap {
            Snap::Pause => quiet_together(bladed, span, clock, within)?,
            Snap::Cut => {
                let cuts = bladed.iter().flat_map(|one| {
                    one.found.places.iter().map(move |place| {
                        // An index's times are rounded to microseconds.
                        let cut = one.media.rate.round(media_time(place.at));
                        let to = one.clip.played_at(cut);
                        let to = rate.round(to.expect("goes_by keeps only clips that play media"));
                        (to, one, place)
                    })
                });
                let (to, one, place) = cuts
                    .filter(|(to, ..)| span.holds(*to))
                    .min_by_key(|(to, ..)| (*to - at).abs())
                    .ok_or_else(|| {
                        let none: Vec<String> =
                            bladed.iter().map(|one| one.nowhere(snap)).collect();
                        none.join("; ")
                    })?;
                let place = place_said(one.media, (snap, place), &one.speech);
                (to, format!("onto {place}"))
            }
        })
    }
}

/// A clip a blade cuts and goes by: where the blade was asked for in its
/// media, and the places beside that.
struct Bladed<'a> {
    clip: &'a Clip,
    media: &'a Media,
    source: Time,
    reach: Time,
    found: Snaps,
    speech: Rc<Speech>,
}

impl Bladed<'_> {
    /// Where the clip plays `seconds` of its media.
    fn played(&self, seconds: f64) -> Time {
        self.clip
            .played_at(media_time(seconds))
            .expect("goes_by keeps only clips that play media")
    }

    /// Where the clip plays a stretch of its media, the earlier end first:
    /// a clip in reverse plays the end of a pause first.
    fn plays(&self, (from, to): (f64, f64)) -> (Time, Time) {
        let (from, to) = (self.played(from), self.played(to));
        (from.min(to), from.max(to))
    }

    fn nowhere(&self, snap: Snap) -> String {
        nothing_near(
            self.media,
            self.source,
            snap,
            self.reach,
            &self.found.barred,
        )
    }

    /// What of this clip is in the way of a blade at `to` on its sequence,
    /// where that is no place for it: the word a quiet there is on the
    /// other side of, or the word its transcript has it saying there.
    fn in_the_way(&self, to: Time) -> Option<String> {
        let takes = self.found.places.iter().any(|place| {
            let room = self.plays(place.room);
            room.0 <= to && to <= room.1
        });
        if takes {
            return None;
        }
        let playing = self.clip.source_at(to)?.seconds();
        let across = self.found.barred.iter().find_map(|barred| match barred {
            Barred::Across { quiet, word, .. } if quiet.0 <= playing && playing <= quiet.1 => {
                Some(word)
            }
            _ => None,
        });
        let heard = self.speech.heard();
        let says = heard.iter().find(|word| {
            !is_a_sound_note(&word.text) && word.start <= playing && playing <= word.end
        });
        let id = &self.clip.id;
        Some(match (across, says) {
            (Some(word), _) => format!(
                ", which is on the other side of {} in `{id}`",
                quote(word.text.trim())
            ),
            (None, Some(word)) => format!(", while `{id}` says {}", quote(word.text.trim())),
            (None, None) => format!(", which is no place for `{id}`"),
        })
    }
}

/// How far a blade asked for at `at` may go and still cut the clips it cuts
/// there and no other: from `low` to `high`, each with the clip that says
/// so, one the blade would come out of or one it would go into.
struct Span {
    at: Time,
    low: (Time, String),
    high: (Time, String),
    /// What it is, as what is said of it names it: `blade` or `roll`.
    name: &'static str,
    /// What to do where the clips it cuts have no quiet in common.
    apart: &'static str,
}

impl Span {
    fn holds(&self, to: Time) -> bool {
        self.low.0 <= to && to <= self.high.0
    }

    /// Why a place at `to`, which the span does not hold, is refused: for a blade, what it
    /// would cut there that it was not asked to, or would not that it was; for a roll, which
    /// clip would keep no frame.
    fn refuses(&self, to: Time) -> String {
        if to < self.low.0 {
            self.low.1.clone()
        } else {
            self.high.1.clone()
        }
    }
}

/// Where a blade goes that is to be in a quiet of every clip of `bladed` at
/// once, within `span`: the frame of `clock`, and what is said of where it
/// went. Each clip takes it as it would a split of itself; where the clips
/// are quiet together it keeps clear of the sound either side in each, and
/// a quiet too short together for that is divided as a short pause of one.
fn quiet_together(
    bladed: &[Bladed<'_>],
    span: &Span,
    clock: &Clock,
    within: Time,
) -> Result<(Time, String), String> {
    let (rate, at) = (clock.rate, span.at);
    let said =
        |one: &Bladed<'_>, place: &Place| place_said(one.media, (Snap::Pause, place), &one.speech);
    // What each would take by itself, as a split of it would.
    let mut alone: Vec<(Time, &Place)> = Vec::new();
    let mut nowhere: Vec<String> = Vec::new();
    for one in bladed {
        let mut frames = one.found.places.iter().filter_map(|place| {
            let to = frame_within(rate, one.played(place.at), one.plays(place.room))?;
            Some((to, place))
        });
        let first = frames.clone().next();
        match (frames.find(|(to, _)| span.holds(*to)), first) {
            (Some(took), _) => alone.push(took),
            (None, Some((to, place))) => nowhere.push(format!(
                "`{}` would take the {} to {}, into {}, {}",
                one.clip.id,
                span.name,
                clock.print_position(to),
                said(one, place),
                span.refuses(to)
            )),
            (None, None) => nowhere.push(format!(
                "in `{}`, {}",
                one.clip.id,
                one.nowhere(Snap::Pause)
            )),
        }
    }
    if !nowhere.is_empty() {
        return Err(nowhere.join("; "));
    }
    // The stretches a frame would do in for all of them: in a quiet of
    // each, the `nth` of its places, where those are quiet together.
    let ever = (Time::from_flicks(i64::MIN), Time::from_flicks(i64::MAX));
    let mut together: Vec<((Time, Time), Vec<usize>)> = vec![(ever, Vec::new())];
    for one in bladed {
        together = together
            .iter()
            .flat_map(|(all, taken)| {
                let places = one.found.places.iter().enumerate();
                places.filter_map(move |(nth, place)| {
                    let room = one.plays(place.room);
                    let both = (all.0.max(room.0), all.1.min(room.1));
                    let taken = taken.iter().copied().chain([nth]).collect();
                    (both.0 <= both.1).then_some((both, taken))
                })
            })
            .collect();
    }
    let mut frames: Vec<_> = together
        .iter()
        .filter_map(|(room, taken)| {
            let places: Vec<(&Bladed<'_>, &Place)> = bladed
                .iter()
                .zip(taken)
                .map(|(one, nth)| (one, &one.found.places[*nth]))
                .collect();
            let all = |stretch: fn(&Place) -> (f64, f64)| {
                let each = places.iter().map(|(one, place)| one.plays(stretch(place)));
                let from = each.clone().map(|(from, _)| from).max()?;
                Some((from, each.map(|(_, to)| to).min()?))
            };
            let quiet = all(|place| place.quiet)?;
            let (from, to) = all(|place| clear(place.quiet))?;
            let (from, to) = if from <= to {
                (from, to)
            } else {
                let (from, to) = clear((quiet.0.seconds(), quiet.1.seconds()));
                let time = |seconds: f64| Time::from_seconds(seconds).unwrap_or(at);
                (time(from), time(to))
            };
            let to = frame_within(rate, at.clamp(from, to), *room)?;
            Some((to, places, quiet, taken.iter().max().copied()))
        })
        .collect();
    // The nearer end of the sound each is in first, as for one of them;
    // else the nearer frame.
    frames.sort_by_key(|(to, _, _, furthest)| (*furthest, (*to - at).abs()));
    if let Some((to, places, quiet, _)) = frames.iter().find(|(to, ..)| span.holds(*to)) {
        let places: Vec<String> = places.iter().map(|(one, place)| said(one, place)).collect();
        let mut into = format!("into {}", places.join(", and "));
        if places.len() > 1 {
            let together = (quiet.1 - quiet.0).seconds();
            write!(into, ", quiet together for {together:.2}s")
                .expect("writing to a String cannot fail");
        }
        return Ok((*to, into));
    }
    if let Some((to, ..)) = frames.first() {
        return Err(format!(
            "the clips this {} cuts are quiet together at {}, {}",
            span.name,
            clock.print_position(*to),
            span.refuses(*to)
        ));
    }
    let each: Vec<String> = bladed
        .iter()
        .zip(&alone)
        .map(|(one, (to, place))| {
            let place = said(one, place);
            let way = bladed.iter().find_map(|other| other.in_the_way(*to));
            let way = way.unwrap_or_default();
            if *to == at {
                format!("`{}` is quiet there, in {place}{way}", one.clip.id)
            } else {
                format!(
                    "`{}` would take it to {}, into {place}{way}",
                    one.clip.id,
                    clock.print_position(*to)
                )
            }
        })
        .collect();
    Err(format!(
        "the clips this {} cuts have no quiet in common within {:.2}s of {}: {}: {}",
        span.name,
        within.seconds(),
        clock.print_position(at),
        each.join("; "),
        span.apart
    ))
}

/// A place of `media` that `snap` put an edge in, as it is said of the edge:
/// the cut, or the pause or gap and the words either side of it.
fn place_said(media: &Media, (snap, place): (Snap, &Place), speech: &Speech) -> String {
    let position =
        |seconds: f64| Clock::of_media(media).print_position(media.rate.round(media_time(seconds)));
    let (start, end) = place.quiet;
    match snap {
        Snap::Cut => format!("the cut of `{}` at {}", media.id, position(start)),
        Snap::Pause => {
            // The words either side of where it is now, where a
            // transcript heard them near.
            let mut heard = speech.heard();
            heard.retain(|word| !is_a_sound_note(&word.text));
            let (before, after) = beside(&heard, place.at, place.at);
            let before = before.filter(|word| word.end > start - WORDS_BESIDE_SECONDS);
            let after = after.filter(|word| word.start < end + WORDS_BESIDE_SECONDS);
            let said = |word: &Word| quote(word.text.trim());
            let (quiet, beside) = match (place.gap, before.map(said), after.map(said)) {
                (true, Some(before), Some(after)) => {
                    ("gap", format!(", where {before} and {after} run together"))
                }
                (true, ..) => ("gap", String::new()),
                (false, Some(before), Some(after)) => {
                    ("pause", format!(", between {before} and {after}"))
                }
                (false, Some(before), None) => ("pause", format!(", after {before}")),
                (false, None, Some(after)) => ("pause", format!(", before {after}")),
                (false, None, None) => ("pause", String::new()),
            };
            let cut = if place.on_cut {
                format!(", on the cut of its picture at {}", position(place.at))
            } else {
                String::new()
            };
            format!(
                "the {:.2}s {quiet} of `{}` at {}{beside}{cut}",
                end - start,
                media.id,
                position(start)
            )
        }
    }
}

/// Why nothing near `at` on `media` would do for `snap`.
pub(super) fn nothing_near(
    media: &Media,
    at: Time,
    snap: Snap,
    reach: Time,
    barred: &[Barred],
) -> String {
    let what = match snap {
        Snap::Pause => "pause",
        Snap::Cut => "cut",
    };
    let clock = Clock::of_media(media);
    let position = |seconds: f64| clock.print_position(media.rate.round(media_time(seconds)));
    let why: Vec<String> = barred
        .iter()
        .map(|barred| match barred {
            Barred::Across {
                quiet: at,
                gap,
                word,
            } => format!(
                "the {} at {} is on the other side of {}",
                if *gap { "gap" } else { "pause" },
                position(at.0),
                quote(word.text.trim())
            ),
            Barred::Unheard { quiet: at } => format!(
                "the sound parts for {:.2}s at {}, which no transcript covers to say is \
                 between two words: look at `{}` with transcript there first",
                at.1 - at.0,
                position(at.0),
                media.id
            ),
            Barred::Inside { quiet: at, word } => format!(
                "the sound parts for {:.2}s at {} inside {}, by its transcript",
                at.1 - at.0,
                position(at.0),
                quote(word.text.trim())
            ),
        })
        .collect();
    // Where the edge was asked for falls between the media's frames when the clip plays at
    // another rate or speed than the media's own; it is said on the frame it falls in.
    let mut said = format!(
        "no {what} of `{}` within {:.2}s of {} would do here",
        media.id,
        reach.seconds(),
        clock.print_position(media.rate.round(at))
    );
    if !why.is_empty() {
        said.push_str(": ");
        said.push_str(&why.join("; "));
    }
    said
}

/// `seconds` of a media as a time, none of it before its start.
pub(super) fn media_time(seconds: f64) -> Time {
    Time::from_seconds(seconds.max(0.0)).unwrap_or(Time::ZERO)
}

/// The frame of `rate` nearest `at` that is no earlier than `from` and no
/// later than `to`; `None` when no frame falls between them.
pub(super) fn frame_within(rate: Rate, at: Time, (from, to): (Time, Time)) -> Option<Time> {
    let first = rate.frames_floor(from) + i64::from(!rate.is_aligned(from));
    let last = rate.frames_floor(to);
    (first <= last).then(|| rate.time_of(rate.frames_round(at).clamp(first, last)))
}

/// How near a quiet a word has to be for it to be named as beside it, in
/// seconds.
const WORDS_BESIDE_SECONDS: f64 = 2.0;
