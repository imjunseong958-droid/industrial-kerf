//! How a project's sequences are planned for drawing ([`Builder`]): each track's clips with their
//! spans and transitions, each clip's source, effects, blend, and how large it is ever shown,
//! nested sequences once each, and what is refused as unsupported; with the number for all about a
//! clip that does not change over time that its frames are kept by.

use super::*;

/// Nested sequences deeper than this are refused, as the graph refuses
/// them.
pub(super) const MAX_NESTING: usize = 16;

/// Plans a program's sequences, the program's first and then those nested in it, gathering the
/// media they decode and the effects they run.
pub(super) struct Builder<'a> {
    pub(super) project: &'a Project,
    pub(super) catalog: &'a Catalog,
    /// Planned sequences by index; `None` while one is being planned.
    pub(super) sequences: Vec<Option<SequencePlan>>,
    /// The index of each nested sequence planned.
    pub(super) built: HashMap<Id, usize>,
    pub(super) clips: Vec<ClipSource>,
    pub(super) nesting: Vec<Id>,
    /// The effects of every clip planned, for the LUT files they name.
    pub(super) effects: Vec<Effect>,
    /// The program is an HDR sequence, so footage keeps what SDR could not show.
    pub(super) hdr: bool,
}

/// A clip's span on its track, from when to when it shows with its transitions' handles, how it
/// enters, and whether a transition takes it out.
type Span = (Time, Time, Option<Entrance>, bool);

/// A FilmCraft transition on a track: the places on the track of the clips it leaves and enters,
/// what it is with its parameters, and from when to when.
type Mix = (usize, usize, FxDef, Params, Time, Time);

/// What a clip draws from, the source's size in its own pixels, whether it is fitted to the
/// canvas, and whether its picture may be clear in places.
type Sourced = (Source, (u32, u32), bool, bool);

impl<'a> Builder<'a> {
    pub(super) fn sequence(&mut self, sequence: &'a Sequence) -> Result<SequencePlan> {
        let (width, height) = canvas(sequence);
        let mut tracks = Vec::new();
        let mut captions = Vec::new();
        let mut sounds = Vec::new();
        for track in &sequence.tracks {
            match track.kind {
                TrackKind::Audio => sounds.push(self.sound(track)),
                TrackKind::Video if track.enabled => tracks.push(self.track(sequence, track)?),
                // Effects read hidden tracks too; one that cannot be drawn
                // reads as empty, unless a sequence it holds could not be
                // planned, which leaves that sequence no plan to draw by.
                TrackKind::Video => {
                    let planned = self.sequences.len();
                    match self.track(sequence, track) {
                        Ok(plan) => tracks.push(plan),
                        Err(error) if self.sequences[planned..].iter().any(Option::is_none) => {
                            return Err(error)
                        }
                        Err(_) => tracks.push(TrackPlan {
                            id: track.id.clone(),
                            shows: false,
                            shown: Vec::new(),
                            dips: Vec::new(),
                            pairs: Vec::new(),
                        }),
                    }
                }
                TrackKind::Subtitle if track.enabled => push_captions(track, &mut captions),
                _ => {}
            }
        }
        let rigs = rigs(sequence, &tracks);
        Ok(SequencePlan {
            rigs,
            name: sequence.name.clone(),
            width,
            height,
            rate: sequence.rate,
            start: sequence.start,
            drop_frame: sequence.drop_frame,
            duration: sequence.duration(),
            track_ids: tracks.iter().map(|track| track.id.to_string()).collect(),
            tracks,
            sound_ids: sounds.iter().map(|sound| sound.id.clone()).collect(),
            sounds,
            captions,
        })
    }

    /// A sound track's enabled media clips that have sound, for the effects that draw it.
    fn sound(&self, track: &Track) -> SoundPlan {
        SoundPlan {
            id: track.id.to_string(),
            clips: track
                .clips
                .iter()
                .filter(|clip| clip.enabled)
                .filter_map(|clip| {
                    let Content::Media {
                        media,
                        source_in,
                        speed,
                    } = &clip.content
                    else {
                        return None;
                    };
                    let media = self.project.media(media.as_str())?;
                    media.audio.as_ref()?;
                    Some(SoundClip {
                        path: media.path.clone(),
                        start: clip.start,
                        duration: clip.duration,
                        source_in: *source_in,
                        speed: speed.clone(),
                    })
                })
                .collect(),
        }
    }

    fn track(&mut self, sequence: &'a Sequence, track: &'a Track) -> Result<TrackPlan> {
        let rate = sequence.rate;
        let mut spans: Vec<Span> = track
            .clips
            .iter()
            .map(|clip| (clip.start, clip.end(), None, false))
            .collect();
        let mut dips = Vec::new();
        // FilmCraft's transitions, by the track's clips they join.
        let mut mixes = Vec::new();
        for transition in &track.transitions {
            let (Some(from), Some(to)) = (
                track.clip_index(transition.from.as_str()),
                track.clip_index(transition.to.as_str()),
            ) else {
                continue;
            };
            let cut = track.clips[from].end();
            let (before, after) = transition.split_at_cut(rate);
            let (start, end) = (cut - before, cut + after);
            let reveal = match transition.fx.as_str() {
                "dip" => {
                    let middle = start + Time::from_flicks((end - start).flicks() / 2);
                    spans[from].1 = middle;
                    spans[to].0 = middle;
                    dips.push(self.dip(transition, start, end)?);
                    continue;
                }
                "dissolve" | "crossfade" => Reveal::Dissolve,
                kerf_model::look::STACK => Reveal::Over,
                "wipe" => self.wipe(transition)?,
                other if kerf_fx::transitions::filmcraft_id(other).is_some() => {
                    let def = self.catalog.fx(other).ok_or_else(|| {
                        ComposeError::Invalid(format!("unknown transition `{other}`"))
                    })?;
                    // Both clips show through the transition, which mixes them.
                    spans[from].1 = end;
                    spans[from].3 = true;
                    spans[to].0 = start;
                    mixes.push((from, to, def.clone(), transition.params.clone(), start, end));
                    continue;
                }
                other => {
                    return Err(unsupported(&transition.id, format!("transition `{other}`")));
                }
            };
            spans[from].1 = end;
            spans[from].3 = true;
            spans[to].0 = start;
            spans[to].2 = Some((reveal, start, end));
        }
        let (shown, places) = self.shown(sequence, track, spans, &mixes)?;
        let pairs = pairs(mixes, &places);
        Ok(TrackPlan {
            id: track.id.clone(),
            shows: track.enabled,
            shown,
            dips,
            pairs,
        })
    }

    /// A dip's color between the two clips of its cut, from `start` to `end`.
    fn dip(&self, transition: &kerf_model::Transition, start: Time, end: Time) -> Result<Dip> {
        Ok(Dip {
            color: color(
                self.catalog,
                &transition.id,
                &transition.params,
                "dip",
                "color",
            )?,
            from: start,
            to: end,
        })
    }

    /// A wipe's front, at the angle and with the softness its transition has.
    fn wipe(&self, transition: &kerf_model::Transition) -> Result<Reveal> {
        Ok(Reveal::Wipe {
            angle: static_number(
                self.catalog,
                &transition.id,
                &transition.params,
                "wipe",
                "angle",
            )?,
            softness: static_number(
                self.catalog,
                &transition.id,
                &transition.params,
                "wipe",
                "softness",
            )?,
        })
    }

    /// The clips of `track` it shows, each over its span of `spans` as the graph clamps it, and
    /// each one's place among them, by its place on the track. `mixes` are the track's FilmCraft
    /// transitions.
    fn shown(
        &mut self,
        sequence: &'a Sequence,
        track: &'a Track,
        spans: Vec<Span>,
        mixes: &[Mix],
    ) -> Result<(Vec<Shown>, HashMap<usize, usize>)> {
        let mut shown = Vec::new();
        // Each clip's place in `shown`, by its place on the track.
        let mut places = HashMap::new();
        for (index, (clip, (from, to, reveal, fades_out))) in
            track.clips.iter().zip(spans).enumerate()
        {
            if !clip.enabled || matches!(clip.content, Content::Caption { .. }) {
                continue;
            }
            let adjustment =
                matches!(&clip.content, Content::Generator { fx, .. } if fx == "adjustment");
            let mixed = mixes.iter().any(|mix| mix.0 == index || mix.1 == index);
            if adjustment && (reveal.is_some() || fades_out || mixed) {
                return Err(unsupported(&clip.id, "a transition on an adjustment layer"));
            }
            let (from, to) = if adjustment {
                (clip.start, clip.end())
            } else {
                match self.span(clip, from, to) {
                    Some(span) => span,
                    None => continue,
                }
            };
            let Some(plan) = self.clip(sequence, clip, from, to, reveal.is_none() && !mixed)?
            else {
                continue;
            };
            places.insert(index, shown.len());
            shown.push(Shown {
                plan,
                from,
                to,
                reveal,
            });
        }
        Ok((shown, places))
    }

    /// The part of a shown clip its source covers, as the graph clamps it.
    fn span(&self, clip: &Clip, mut lo: Time, mut hi: Time) -> Option<(Time, Time)> {
        let length = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()).map(|m| m.duration),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map(Sequence::duration),
            Content::Generator { .. } | Content::Caption { .. } => None,
        };
        if let Some(length) = length {
            let ends = (clip.played_at(Time::ZERO)?, clip.played_at(length)?);
            let (earliest, latest) = if clip.speed().is_reverse() {
                (ends.1, ends.0)
            } else {
                ends
            };
            lo = lo.max(earliest);
            hi = hi.min(latest);
        }
        (lo < hi).then_some((lo, hi))
    }

    /// A clip's plan, or nothing for a clip without a picture. `entering`
    /// is false for a clip a transition reveals.
    fn clip(
        &mut self,
        sequence: &'a Sequence,
        clip: &'a Clip,
        lo: Time,
        hi: Time,
        entering: bool,
    ) -> Result<Option<ClipPlan>> {
        let canvas = canvas(sequence);
        let mut name = clip.name.clone().unwrap_or_else(|| clip.id.to_string());
        let (source, size, fit, alpha) = match &clip.content {
            Content::Media {
                media,
                source_in,
                speed,
            } => {
                let Some(sourced) = self.media_source(clip, media, *source_in, speed, &mut name)?
                else {
                    return Ok(None);
                };
                sourced
            }
            Content::Sequence {
                sequence: nested,
                source_in,
                speed,
            } => self.nested_source(clip, nested, *source_in, speed, &mut name)?,
            Content::Generator { fx, params, .. } => {
                self.generator_source(clip, fx.as_str(), params, canvas)?
            }
            Content::Caption { .. } => return Ok(None),
        };
        let animated = clip.parent.is_some()
            || !matches!(param(clip, "path"), Param::Static(Value::Text(text)) if text.is_empty())
            || GEOMETRY.iter().any(|name| param(clip, name).varies());
        // A mesh is in depth whether it says so or not.
        let three_d = matches!(param(clip, "three_d"), Param::Static(Value::Bool(true)))
            || matches!(source, Source::Mesh);
        if matches!(source, Source::Mesh) {
            three_d::mesh::check(clip).map_err(|why| {
                ComposeError::Invalid(format!("clip `{}`: its mesh's file {why}", clip.id))
            })?;
        }
        let effects = self.picture_effects(clip)?;
        let blend = blend_of(clip)?;
        self.effects.extend(effects.iter().cloned());
        let media = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()),
            _ => None,
        };
        let digest = clip_digest(clip, self.catalog, media);
        let reads_sequence_time = effects
            .iter()
            .any(|effect| effect.fx.as_str().starts_with("text."));
        let path = motion_path(clip, canvas);
        let mut plan = ClipPlan {
            clip: clip.clone(),
            name,
            source,
            size,
            fit,
            letterboxed: false,
            alpha,
            plain: false,
            animated,
            three_d,
            filmed: clip.filmed(),
            shown: 1.0,
            blend,
            effects,
            path,
            digest,
            reads_sequence_time,
        };
        let largest = largest_shown(clip, &plan, sequence.rate, (lo, hi), canvas, animated)?;
        if matches!(plan.source, Source::Media { .. }) {
            // A parent can scale it past anything its own properties say.
            let moved = clip.parent.is_some() || plan.path.is_some() || three_d;
            plan.shown = if moved { 1.0 } else { largest.clamp(1e-6, 1.0) };
            plan.letterboxed = !alpha && !moved && letterboxed(&plan, canvas);
            let deep = media
                .and_then(|media| media.video)
                .is_some_and(|video| video.is_deep());
            plan.plain = !alpha
                && !deep
                && clip.retime == kerf_model::Retime::Frames
                && !moved
                && self.nesting.len() == 1
                && entering
                && size == canvas
                && untouched(&plan);
        }
        Ok(Some(plan))
    }

    /// What a media clip draws from: its media's picture, decoded as the clip plays it. Nothing
    /// for media without one. `name` becomes the file's, for a clip without a name of its own.
    fn media_source(
        &mut self,
        clip: &Clip,
        media: &Id,
        source_in: Time,
        speed: &Speed,
        name: &mut String,
    ) -> Result<Option<Sourced>> {
        let media = self.project.media(media.as_str()).ok_or_else(|| {
            ComposeError::Invalid(format!("clip `{}` uses missing media", clip.id))
        })?;
        let Some(video) = media.video else {
            return Ok(None);
        };
        if clip.name.is_none()
            && let Some(file) = std::path::Path::new(&media.path).file_name()
        {
            *name = file.to_string_lossy().into_owned();
        }
        self.clips.push(ClipSource {
            path: media.path.clone(),
            rate: media.rate,
            frames: frame_count(media.rate, media.duration),
            width: video.width,
            height: video.height,
            alpha: video.alpha,
            deep: video
                .is_deep()
                .then(|| crate::footage::input_transform(video.color, self.hdr)),
            retime: clip.retime,
            source_in,
            speed: speed.clone(),
            duration: clip.duration,
        });
        Ok(Some((
            Source::Media {
                picture: self.clips.len() - 1,
            },
            (video.width, video.height),
            true,
            video.alpha,
        )))
    }

    /// What a clip holding sequence `nested` draws from: that sequence, planned once however many
    /// clips hold it, and refused nested too deep or in itself. `name` becomes the sequence's, for
    /// a clip without a name of its own.
    fn nested_source(
        &mut self,
        clip: &Clip,
        nested: &Id,
        source_in: Time,
        speed: &Speed,
        name: &mut String,
    ) -> Result<Sourced> {
        if self.nesting.len() >= MAX_NESTING || self.nesting.contains(nested) {
            return Err(unsupported(&clip.id, "sequences nested this deep"));
        }
        let inner = self.project.sequence(nested.as_str()).ok_or_else(|| {
            ComposeError::Invalid(format!(
                "clip `{}` uses missing sequence `{nested}`",
                clip.id
            ))
        })?;
        if clip.name.is_none() {
            *name = inner.name.clone();
        }
        let index = match self.built.get(nested) {
            Some(index) => *index,
            None => {
                self.nesting.push(nested.clone());
                let index = self.sequences.len();
                self.sequences.push(None);
                let plan = self.sequence(inner);
                self.nesting.pop();
                self.sequences[index] = Some(plan?);
                self.built.insert(nested.clone(), index);
                index
            }
        };
        Ok((
            Source::Nested {
                sequence: index,
                source_in,
                speed: speed.clone(),
                duration: clip.duration,
                angle: clip.angle.clone(),
            },
            kerf_core::program::canvas(inner),
            true,
            true,
        ))
    }

    /// What generator `fx` draws from, the size of the canvas: a solid, a title, an adjustment
    /// layer, a mesh, nothing for a null, a camera, a light, or a signal, and else what a crate
    /// draws (`crate::domains`).
    fn generator_source(
        &self,
        clip: &Clip,
        fx: &str,
        params: &kerf_model::Params,
        canvas: (u32, u32),
    ) -> Result<Sourced> {
        Ok(match fx {
            "solid" => (
                Source::Solid(color(self.catalog, &clip.id, params, "solid", "color")?),
                canvas,
                false,
                true,
            ),
            "title" => (
                Source::Title(self.title(clip, params)?),
                canvas,
                false,
                true,
            ),
            "adjustment" => (Source::Adjustment, canvas, false, true),
            "mesh" => (Source::Mesh, canvas, false, true),
            "null" | "camera" | "light" | kerf_model::SIGNAL => {
                (Source::Nothing, canvas, false, true)
            }
            other => match domains::source(other) {
                Some(source) => (Source::Drawn(source), canvas, false, true),
                None => return Err(unsupported(&clip.id, format!("generator `{other}`"))),
            },
        })
    }

    /// A clip's enabled effects on its picture: each one a crate runs (`crate::domains`), but a
    /// shape's outline, which is part of the shape, and none on its sound.
    fn picture_effects(&self, clip: &Clip) -> Result<Vec<Effect>> {
        let mut effects = Vec::new();
        for effect in clip.effects.iter().filter(|effect| effect.enabled) {
            let def = self.catalog.fx(effect.fx.as_str()).ok_or_else(|| {
                ComposeError::Invalid(format!("clip `{}` uses unknown effect", clip.id))
            })?;
            if def.domain != Domain::Video {
                continue;
            }
            match effect.fx.as_str() {
                // Part of the shape, which reads it where it draws.
                kerf_model::SHAPE_OUTLINE => {}
                id if domains::effect(id).is_some() => effects.push(effect.clone()),
                other => {
                    return Err(unsupported(
                        &clip.id,
                        format!("effect `{other}` on a picture"),
                    ))
                }
            }
        }
        Ok(effects)
    }

    /// A title's text and look, which do not animate.
    fn title(&self, clip: &Clip, params: &kerf_model::Params) -> Result<TitlePlan> {
        let def = self.catalog.fx("title").expect(
            "Kerf draws with the bundled catalog, looks perhaps added, which defines `title`",
        );
        let get = |name: &str| -> Result<Value> {
            match params.get(name) {
                None => Ok(def
                    .find(name)
                    .expect("the bundled `title` defines every parameter read here")
                    .default
                    .clone()),
                Some(Param::Static(value)) => Ok(value.clone()),
                Some(Param::Animated(_) | Param::Expr(_)) => {
                    Err(unsupported(&clip.id, format!("animated title `{name}`")))
                }
            }
        };
        Ok(TitlePlan {
            text: match get("text")? {
                Value::Text(text) => text,
                _ => String::new(),
            },
            font: match get("font")? {
                Value::Text(font) if !font.trim().is_empty() => font,
                _ => "sans-serif".to_owned(),
            },
            style: kerf_model::title_face(
                match &get("style")? {
                    Value::Text(style) => style,
                    _ => "",
                },
                get("weight")?.as_number().unwrap_or(0.0),
            ),
            size: get("size")?.as_number().unwrap_or(0.08),
            color: match get("color")? {
                Value::Color(color) => color,
                _ => Rgba::WHITE,
            },
            outline: match (get("outline")?, get("outline_width")?.as_number()) {
                (Value::Color(edge), Some(width)) if edge.a > 0 && width > 0.0 => {
                    Some((edge, width))
                }
                _ => None,
            },
            justify: match get("align")? {
                Value::Enum(align) if align == "left" => Justify::Left,
                Value::Enum(align) if align == "right" => Justify::Right,
                _ => Justify::Center,
            },
            axes: kerf_model::TITLE_AXES
                .iter()
                .filter_map(|tag| Some((*tag, params.get(std::str::from_utf8(tag).ok()?)?.clone())))
                .collect(),
            // The document is refused where they do not read; a title built past it draws without.
            features: match get("features")? {
                Value::Text(features) => kerf_model::parse_features(&features).unwrap_or_default(),
                _ => Vec::new(),
            },
            reveal: title_reveal(def, params),
        })
    }
}

/// The captions of subtitle track `track` that say something, onto `captions`.
fn push_captions(track: &Track, captions: &mut Vec<CaptionPlan>) {
    for clip in track.clips.iter().filter(|clip| clip.enabled) {
        if let Content::Caption { text } = &clip.content
            && !text.trim().is_empty()
        {
            captions.push(CaptionPlan {
                text: text.clone(),
                from: clip.start,
                to: clip.end(),
            });
        }
    }
}

/// The clips of `tracks` that other clips of `sequence` move with, by their ids.
fn rigs(sequence: &Sequence, tracks: &[TrackPlan]) -> HashMap<Id, Rig> {
    let parents: std::collections::HashSet<&Id> = sequence
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .filter_map(|clip| clip.parent.as_ref())
        .collect();
    tracks
        .iter()
        .flat_map(|track: &TrackPlan| &track.shown)
        .map(|shown| &shown.plan)
        .filter(|plan| parents.contains(&plan.clip.id))
        .map(|plan| {
            (
                plan.clip.id.clone(),
                Rig {
                    clip: plan.clip.clone(),
                    size: plan.size,
                    fit: plan.fit,
                    path: plan.path.clone(),
                    in_depth: plan.three_d || is_null(&plan.clip),
                    digest: plan.digest,
                },
            )
        })
        .collect()
}

/// A track's FilmCraft transitions `mixes`, by the places in what it shows of the clips they mix,
/// as `places` has them: one whose clips do not show mixes nothing, and one with one of them alone
/// mixes it with nothing.
fn pairs(mixes: Vec<Mix>, places: &HashMap<usize, usize>) -> Vec<Pair> {
    mixes
        .into_iter()
        .filter_map(|(from, to, def, params, start, end)| {
            let (from, to, from_nothing) = match (places.get(&from), places.get(&to)) {
                (Some(from), Some(to)) => (*from, *to, false),
                // One side alone mixes with nothing.
                (Some(only), None) => (*only, *only, false),
                (None, Some(only)) => (*only, *only, true),
                // Neither shows: nothing to draw.
                (None, None) => return None,
            };
            Some(Pair {
                from,
                to,
                from_nothing,
                def,
                params,
                start,
                end,
            })
        })
        .collect()
}

/// A clip's blend mode, which does not animate.
fn blend_of(clip: &Clip) -> Result<Blend> {
    let blend = match param(clip, "blend") {
        Param::Static(Value::Enum(blend) | Value::Text(blend)) => blend
            .parse::<Blend>()
            .map_err(|_| unsupported(&clip.id, format!("blend `{blend}`")))?,
        Param::Static(_) => Blend::Normal,
        Param::Animated(_) | Param::Expr(_) => {
            return Err(unsupported(&clip.id, "animated `blend`"))
        }
    };
    Ok(blend)
}

/// The most of its size clip `clip`, planned as `plan`, is shown at on a canvas of `canvas` across
/// the frames at `rate` between `lo` and `hi`: at each when it animates, and at the first when it
/// does not. Refused where its crop leaves nothing of it.
fn largest_shown(
    clip: &Clip,
    plan: &ClipPlan,
    rate: Rate,
    (lo, hi): (Time, Time),
    canvas: (u32, u32),
    animated: bool,
) -> Result<f64> {
    let frames = Frames::new(rate, clip, lo, hi);
    let cw = f64::from(canvas.0);
    let ch = f64::from(canvas.1);
    let mut largest: f64 = 0.0;
    for time in frames.times() {
        let [l, r, t, b] = ["crop_l", "crop_r", "crop_t", "crop_b"]
            .map(|name| number(&param(clip, name).value_at(time)));
        if l + r >= 1.0 || t + b >= 1.0 {
            return Err(ComposeError::Invalid(format!(
                "clip `{}` is cropped away entirely",
                clip.id
            )));
        }
        let (_, scale) = place(plan, (cw, ch), time);
        largest = largest.max(scale.x.abs()).max(scale.y.abs());
        if !animated {
            break;
        }
    }
    Ok(largest)
}

/// A title's reveal, as `params` set it over the defaults of `def`, the title's: none where they
/// do not set one.
fn title_reveal(def: &FxDef, params: &kerf_model::Params) -> Option<Box<Reveals>> {
    let raw = |name: &str| {
        params.get(name).cloned().unwrap_or_else(|| {
            Param::Static(
                def.find(name)
                    .expect("the bundled `title` defines every parameter read here")
                    .default
                    .clone(),
            )
        })
    };
    // Left at rest, it draws as plain text does.
    params.contains_key("reveal").then(|| Box::new(Reveals {
        reveal: raw("reveal"),
        by_word: matches!(params.get("reveal_by"), Some(Param::Static(Value::Enum(by))) if by == "word"),
        soft: raw("reveal_soft"),
        from: [raw("from_x"), raw("from_y")],
        scale: raw("from_scale"),
        rotation: raw("from_rotation"),
        opacity: raw("from_opacity"),
    }))
}

/// A number for everything about `clip` that does not change over time, wherever it starts, and
/// the file it shows. `None` when it draws on something the document does not hold.
pub(super) fn clip_digest(clip: &Clip, catalog: &Catalog, media: Option<&Media>) -> Option<u64> {
    // A light wrapped from another track depends on what that track draws, hidden or not.
    if let Content::Generator { fx, params, .. } = &clip.content {
        let wraps =
            |param: &Param| !matches!(param, Param::Static(Value::Text(track)) if track.is_empty());
        if fx.as_str() == "light" && params.get("image").is_some_and(wraps) {
            return None;
        }
    }
    for effect in &clip.effects {
        let def = catalog.fx(effect.fx.as_str())?;
        let outside = def.params.iter().any(|param| {
            matches!(
                param.ty,
                ParamType::Track | ParamType::Sound | ParamType::File(_)
            ) && !effect_text_is_empty(effect, &param.name)
        });
        if outside {
            return None;
        }
    }
    let mut hash = DefaultHasher::new();
    let mut unplaced = clip.clone();
    unplaced.start = Time::ZERO;
    format!("{unplaced:?}").hash(&mut hash);
    // A file a generator reads, a mesh's, replaced under the same name is another picture.
    if let Content::Generator { fx, params, .. } = &clip.content {
        let def = catalog.fx(fx.as_str())?;
        for param in &def.params {
            if !matches!(param.ty, ParamType::File(_)) {
                continue;
            }
            if let Some(Param::Static(Value::Text(path))) = params.get(&param.name)
                && !path.is_empty()
            {
                let file = std::fs::metadata(path).ok()?;
                file.len().hash(&mut hash);
                file.modified().ok()?.hash(&mut hash);
            }
        }
    }
    if let Some(media) = media {
        format!("{media:?}").hash(&mut hash);
        // A file replaced under the same name is another picture.
        let file = std::fs::metadata(&media.path).ok()?;
        file.len().hash(&mut hash);
        file.modified().ok()?.hash(&mut hash);
    }
    Some(hash.finish())
}

/// Whether effect parameter `name` holds no text, as a track or file not chosen does.
pub(super) fn effect_text_is_empty(effect: &Effect, name: &str) -> bool {
    match effect.params.get(name) {
        Some(Param::Static(Value::Text(text))) => text.is_empty(),
        Some(_) => false,
        None => true,
    }
}

/// Whether a clip is a null: it draws nothing, and is there for other clips to move with.
pub(super) fn is_null(clip: &Clip) -> bool {
    matches!(&clip.content, Content::Generator { fx, .. } if fx.as_str() == "null")
}

/// Whether the graph pads a clip's picture to the canvas with black:
/// media left where fitting puts it (flipped and cropped included), without
/// masks, that does not fill the canvas.
pub(super) fn letterboxed(plan: &ClipPlan, canvas: (u32, u32)) -> bool {
    let clip = &plan.clip;
    let static_number = |name: &str| match param(clip, name) {
        Param::Static(Value::Number(n)) => Some(n),
        _ => None,
    };
    if let [Some(l), Some(r), Some(t), Some(b)] =
        ["crop_l", "crop_r", "crop_t", "crop_b"].map(static_number)
    {
        let (sw, sh) = (f64::from(plan.size.0), f64::from(plan.size.1));
        let (fw, fh) = fitted(
            sw * (1.0 - l - r),
            sh * (1.0 - t - b),
            f64::from(canvas.0),
            f64::from(canvas.1),
        );
        if fw >= f64::from(canvas.0) && fh >= f64::from(canvas.1) {
            return false;
        }
    }
    let unit = |axis: &str| match (static_number("scale"), static_number(axis)) {
        (Some(scale), Some(factor)) => ((scale * factor).abs() - 1.0).abs() < 5e-7,
        _ => false,
    };
    clip.masks.is_empty()
        && ["x", "y", "rotation", "skew", "anchor_x", "anchor_y"]
            .iter()
            .all(|name| static_number(name) == Some(0.0))
        && unit("scale_x")
        && unit("scale_y")
}

/// Whether a clip is its picture unmoved and untouched.
pub(super) fn untouched(plan: &ClipPlan) -> bool {
    let clip = &plan.clip;
    let is = |name: &str, value: f64| matches!(param(clip, name), Param::Static(Value::Number(n)) if n == value);
    plan.blend == Blend::Normal
        && plan.effects.is_empty()
        && clip.masks.is_empty()
        && [
            "x", "y", "rotation", "skew", "anchor_x", "anchor_y", "crop_l", "crop_r", "crop_t",
            "crop_b",
        ]
        .iter()
        .all(|name| is(name, 0.0))
        && ["scale", "scale_x", "scale_y", "opacity"]
            .iter()
            .all(|name| is(name, 1.0))
}

pub(super) fn static_number(
    catalog: &Catalog,
    entity: &Id,
    params: &kerf_model::Params,
    fx: &str,
    name: &str,
) -> Result<f64> {
    match params.get(name) {
        None => Ok(catalog
            .fx(fx)
            .and_then(|def| def.find(name))
            .and_then(|def| def.default.as_number())
            .unwrap_or(0.0)),
        Some(Param::Static(Value::Number(n))) => Ok(*n),
        Some(_) => Err(unsupported(entity, format!("animated `{name}`"))),
    }
}

pub(super) fn color(
    catalog: &Catalog,
    entity: &Id,
    params: &kerf_model::Params,
    fx: &str,
    name: &str,
) -> Result<Rgba> {
    match params.get(name) {
        None => Ok(
            match catalog
                .fx(fx)
                .and_then(|def| def.find(name))
                .map(|def| &def.default)
            {
                Some(Value::Color(color)) => *color,
                _ => Rgba::BLACK,
            },
        ),
        Some(Param::Static(Value::Color(color))) => Ok(*color),
        Some(_) => Err(unsupported(entity, format!("animated `{name}`"))),
    }
}

pub(super) fn unsupported(entity: &Id, what: impl Into<String>) -> ComposeError {
    ComposeError::Unsupported {
        entity: entity.to_string(),
        what: what.into(),
    }
}

/// The frames a clip shows between `lo` and `hi`, as clip times.
pub(super) struct Frames {
    pub(super) start: Time,
    pub(super) rate: Rate,
    pub(super) first: i64,
    pub(super) last: i64,
}

impl Frames {
    pub(super) fn new(rate: Rate, clip: &Clip, lo: Time, hi: Time) -> Self {
        let frame = rate.frame_duration().flicks();
        let ceil = |time: Time| -(-time.flicks()).div_euclid(frame);
        Self {
            start: clip.start,
            rate,
            first: ceil(lo),
            last: (ceil(hi) - 1).max(ceil(lo)),
        }
    }

    pub(super) fn times(&self) -> impl Iterator<Item = Time> + '_ {
        (self.first..=self.last).map(|frame| self.rate.time_of(frame) - self.start)
    }
}
