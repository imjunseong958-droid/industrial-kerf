//! Raster effects, baked before the graph runs.
//!
//! The graph cannot run raster effects (`Engine::Raster`). A render first
//! bakes each picture clip's raster effects into a file of the frames the
//! clip can show, and the graph plays that file in the clip's place. A
//! media clip's frames come from its media; a title, a solid, or a nested
//! sequence has no file, so the graph first renders it on its own, keeping
//! its transparency ([`Prerender`]), and the effects run on that.
//! [`plan`] says which files a render needs and rewrites the project to use
//! them. It reads and writes nothing, so a check compiles exactly what a
//! render would; `kerf-engine` makes the files.

use crate::program::canvas;
use kerf_model::{
    Catalog, Clip, Content, Effect, Engine, Id, IdGen, Media, Project, Rate, Ratio, Sequence, Time,
    Track, TrackKind, VideoInfo,
};
use std::collections::BTreeSet;

/// A sequence rendered on its own, with its transparency, for a clip that
/// has no file: a title or solid (in a sequence made for it), or a nested
/// sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Prerender {
    /// The project the sequence is rendered from.
    pub project: Project,
    pub sequence: Id,
    /// The interval of the sequence rendered.
    pub from: Time,
    pub to: Time,
    /// Where the frames go, as the caller placed them.
    pub output: String,
}

impl Prerender {
    /// Everything the frames depend on but the media's contents: the
    /// sequences rendered and the media they use.
    pub fn identity(&self) -> String {
        let sequences = reachable(&self.project, &self.sequence);
        format!(
            "{:?}|{}..{}|{:?}",
            sequences,
            self.from.flicks(),
            self.to.flicks(),
            self.media()
        )
    }

    /// The media the rendered sequences use.
    pub fn media(&self) -> Vec<&Media> {
        let used: BTreeSet<&str> = reachable(&self.project, &self.sequence)
            .iter()
            .flat_map(|sequence| &sequence.tracks)
            .flat_map(|track| &track.clips)
            .filter_map(|clip| match &clip.content {
                Content::Media { media, .. } => Some(media.as_str()),
                _ => None,
            })
            .collect();
        self.project
            .media
            .iter()
            .filter(|media| used.contains(media.id.as_str()))
            .collect()
    }
}

/// One clip's raster effects over the frames it can show.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeJob {
    /// The file the frames come from: the media's, or a prerender's.
    pub source: String,
    /// The prerender that makes `source`, by index, when it is one.
    pub prerender: Option<usize>,
    pub rate: Rate,
    /// The picture the effects measure against.
    pub width: u32,
    pub height: u32,
    /// The source interval baked, on the source's frames.
    pub from: Time,
    pub to: Time,
    /// The clip's source in, speed, and length, for its keyframes.
    pub source_in: Time,
    pub speed: Ratio,
    pub clip_duration: Time,
    /// The raster effects, in stack order.
    pub effects: Vec<Effect>,
    /// Keeps random effects the same from render to render.
    pub seed: u32,
    /// Where the baked frames go, as the caller placed them.
    pub output: String,
}

impl BakeJob {
    /// The clip time at which the clip shows source time `at`, which is
    /// where its keyframes are read. Handles fall before 0 or after the end.
    pub fn clip_time(&self, at: Time) -> Time {
        if self.speed.is_reverse() {
            let out = self.source_in + self.speed.source_span(self.clip_duration);
            self.speed.timeline_span(out - at)
        } else {
            self.speed.timeline_span(at - self.source_in)
        }
    }

    /// Everything the baked frames depend on but the source's contents.
    pub fn identity(&self) -> String {
        format!(
            "{}|{}|{}x{}|{}..{}|{}|{}|{}|{}|{:?}",
            self.source,
            self.rate,
            self.width,
            self.height,
            self.from.flicks(),
            self.to.flicks(),
            self.source_in.flicks(),
            self.speed,
            self.clip_duration.flicks(),
            self.seed,
            self.effects,
        )
    }
}

/// A file the caller places.
pub enum Job<'a> {
    Prerender(&'a Prerender),
    Bake(&'a BakeJob),
}

/// A project whose picture clips play their baked frames, and the files
/// that takes: prerenders first, then bakes.
#[derive(Clone, Debug)]
pub struct Planned {
    pub project: Project,
    pub prerenders: Vec<Prerender>,
    pub jobs: Vec<BakeJob>,
}

/// Plan what rendering `sequence` (the first one when `None`) of `project`
/// needs: every picture clip with an enabled raster effect, or leaving
/// through a raster transition, in that sequence or one nested in it plays
/// a new media in `place` instead, with its raster effects removed. Raster
/// effects on adjustment layers stay, and the graph reports them.
pub fn plan<E>(
    project: &Project,
    catalog: &Catalog,
    sequence: Option<&str>,
    mut place: impl FnMut(Job<'_>) -> Result<String, E>,
) -> Result<Planned, E> {
    let mut planned = Planned {
        project: project.clone(),
        prerenders: Vec::new(),
        jobs: Vec::new(),
    };
    let root = match sequence {
        Some(id) => project.sequence(id),
        None => project.sequences.first(),
    };
    let Some(root) = root else {
        return Ok(planned);
    };
    let reached: BTreeSet<Id> = reachable(project, &root.id)
        .iter()
        .map(|sequence| sequence.id.clone())
        .collect();
    let mut planner = Planner {
        project,
        catalog,
        ids: project.id_gen(),
        added: Vec::new(),
    };
    for sequence in planned.project.sequences.iter_mut() {
        if !reached.contains(&sequence.id) {
            continue;
        }
        let parent = sequence.clone();
        for track in sequence
            .tracks
            .iter_mut()
            .filter(|track| track.kind == TrackKind::Video)
        {
            planner.track(
                &parent,
                track,
                &mut planned.prerenders,
                &mut planned.jobs,
                &mut place,
            )?;
        }
    }
    planned.project.media.extend(planner.added);
    Ok(planned)
}

struct Planner<'a> {
    project: &'a Project,
    catalog: &'a Catalog,
    ids: IdGen,
    added: Vec<Media>,
}

/// A baked clip's new content: the media that plays its frames.
struct Baked {
    clip: Id,
    media: Id,
    source_in: Time,
    speed: Ratio,
}

impl Planner<'_> {
    fn raster(&self, effect: &Effect) -> bool {
        self.catalog
            .fx(effect.fx.as_str())
            .is_some_and(|fx| fx.engine == Engine::Raster)
    }

    fn track<E>(
        &mut self,
        sequence: &Sequence,
        track: &mut Track,
        prerenders: &mut Vec<Prerender>,
        jobs: &mut Vec<BakeJob>,
        place: &mut impl FnMut(Job<'_>) -> Result<String, E>,
    ) -> Result<(), E> {
        let mut baked = Vec::new();
        for clip in &track.clips {
            let effects: Vec<Effect> = clip
                .effects
                .iter()
                .filter(|effect| effect.enabled && self.raster(effect))
                .cloned()
                .collect();
            if effects.is_empty() {
                continue;
            }
            let (pre, post) = handles(track, clip);
            let Some(source) = self.source(sequence, clip, pre, post, prerenders, place)? else {
                continue;
            };
            let mut job = BakeJob {
                effects,
                seed: seed(clip.id.as_str()),
                clip_duration: clip.duration,
                output: String::new(),
                ..source
            };
            job.output = place(Job::Bake(&job))?;
            let media = self.ids.next("fx");
            // Baked frames keep whatever transparency the effects made.
            self.added.push(Media {
                id: media.clone(),
                path: job.output.clone(),
                rate: job.rate,
                drop_frame: false,
                start: Time::ZERO,
                duration: job.to - job.from,
                video: Some(VideoInfo {
                    width: job.width,
                    height: job.height,
                    alpha: true,
                }),
                audio: None,
                reel: None,
            });
            baked.push(Baked {
                clip: clip.id.clone(),
                media,
                source_in: job.source_in - job.from,
                speed: job.speed,
            });
            jobs.push(job);
        }
        for baked in baked {
            let clip = track
                .clips
                .iter_mut()
                .find(|c| c.id == baked.clip)
                .expect("baked clips stay on their track");
            clip.content = Content::Media {
                media: baked.media,
                source_in: baked.source_in,
                speed: baked.speed,
            };
            clip.effects.retain(|effect| {
                !self
                    .catalog
                    .fx(effect.fx.as_str())
                    .is_some_and(|fx| fx.engine == Engine::Raster)
            });
        }
        Ok(())
    }

    /// The frames `clip` shows, with `pre` and `post` of handles, as a
    /// bake job without its effects; `None` for a clip without a picture
    /// or one the graph reports.
    fn source<E>(
        &mut self,
        sequence: &Sequence,
        clip: &Clip,
        pre: Time,
        post: Time,
        prerenders: &mut Vec<Prerender>,
        place: &mut impl FnMut(Job<'_>) -> Result<String, E>,
    ) -> Result<Option<BakeJob>, E> {
        let job =
            |source: String, prerender, rate: Rate, size: (u32, u32), window: (Time, Time)| {
                BakeJob {
                    source,
                    prerender,
                    rate,
                    width: size.0,
                    height: size.1,
                    from: window.0,
                    to: window.1,
                    source_in: Time::ZERO,
                    speed: Ratio::ONE,
                    clip_duration: clip.duration,
                    effects: Vec::new(),
                    seed: 0,
                    output: String::new(),
                }
            };
        Ok(Some(match &clip.content {
            Content::Media {
                media,
                source_in,
                speed,
            } => {
                let Some(media) = self.project.media(media.as_str()) else {
                    return Ok(None);
                };
                let Some(video) = media.video else {
                    return Ok(None);
                };
                let window = frames(
                    media.rate,
                    source_window(clip, *source_in, *speed, pre, post),
                    media.duration,
                );
                BakeJob {
                    source_in: *source_in,
                    speed: *speed,
                    ..job(
                        media.path.clone(),
                        None,
                        media.rate,
                        (video.width, video.height),
                        window,
                    )
                }
            }
            Content::Sequence {
                sequence: nested,
                source_in,
                speed,
            } => {
                let Some(nested) = self.project.sequence(nested.as_str()) else {
                    return Ok(None);
                };
                let (from, to) = frames(
                    nested.rate,
                    source_window(clip, *source_in, *speed, pre, post),
                    nested.duration(),
                );
                let index = self.prerender(
                    self.project.clone(),
                    nested.id.clone(),
                    (from, to),
                    prerenders,
                    place,
                )?;
                BakeJob {
                    source_in: *source_in - from,
                    speed: *speed,
                    ..job(
                        prerenders[index].output.clone(),
                        Some(index),
                        nested.rate,
                        canvas(nested),
                        (Time::ZERO, to - from),
                    )
                }
            }
            Content::Generator {
                fx,
                version,
                params,
            } if fx.as_str() != "adjustment" => {
                // The generator alone over its span and handles, in a
                // sequence made for it, as large as the clip's.
                let length = pre + clip.duration + post;
                let alone = Clip::new(
                    Id::new("c1").expect("a valid id"),
                    Content::Generator {
                        fx: fx.clone(),
                        version: *version,
                        params: params
                            .iter()
                            .map(|(name, param)| (name.clone(), param.shifted(pre)))
                            .collect(),
                    },
                    Time::ZERO,
                    length,
                );
                let mut track = Track::new(Id::new("V1").expect("a valid id"), TrackKind::Video);
                track.clips.push(alone);
                let id = self.ids.next("prerender");
                let mut project = self.project.clone();
                project.sequences.push(Sequence {
                    id: id.clone(),
                    name: String::new(),
                    tracks: vec![track],
                    markers: Vec::new(),
                    start: Time::ZERO,
                    ..sequence.clone()
                });
                let index = self.prerender(project, id, (Time::ZERO, length), prerenders, place)?;
                BakeJob {
                    source_in: pre,
                    ..job(
                        prerenders[index].output.clone(),
                        Some(index),
                        sequence.rate,
                        canvas(sequence),
                        (Time::ZERO, length),
                    )
                }
            }
            Content::Generator { .. } | Content::Caption { .. } => return Ok(None),
        }))
    }

    fn prerender<E>(
        &mut self,
        project: Project,
        sequence: Id,
        (from, to): (Time, Time),
        prerenders: &mut Vec<Prerender>,
        place: &mut impl FnMut(Job<'_>) -> Result<String, E>,
    ) -> Result<usize, E> {
        let mut prerender = Prerender {
            project,
            sequence,
            from,
            to,
            output: String::new(),
        };
        prerender.output = place(Job::Prerender(&prerender))?;
        prerenders.push(prerender);
        Ok(prerenders.len() - 1)
    }
}

/// How far the clip's transitions reach before and after it.
fn handles(track: &Track, clip: &Clip) -> (Time, Time) {
    track
        .transitions
        .iter()
        .fold((Time::ZERO, Time::ZERO), |(pre, post), t| {
            (
                if t.to == clip.id {
                    pre.max(t.duration)
                } else {
                    pre
                },
                if t.from == clip.id {
                    post.max(t.duration)
                } else {
                    post
                },
            )
        })
}

/// The source interval a clip plays, extended by `pre` and `post` of its
/// sequence before and after it.
fn source_window(
    clip: &Clip,
    source_in: Time,
    speed: Ratio,
    pre: Time,
    post: Time,
) -> (Time, Time) {
    let (a, b) = (-pre, clip.duration + post);
    if speed.is_reverse() {
        let out = source_in + speed.source_span(clip.duration);
        (out - speed.source_span(b), out - speed.source_span(a))
    } else {
        (
            source_in + speed.source_span(a),
            source_in + speed.source_span(b),
        )
    }
}

/// `(a, b)` widened to whole frames of `rate` inside `[0, length]`.
fn frames(rate: Rate, (a, b): (Time, Time), length: Time) -> (Time, Time) {
    let from = rate.time_of(rate.frames_floor(a)).max(Time::ZERO);
    let to = rate
        .time_of(rate.frames_floor(b + rate.frame_duration() - Time::from_flicks(1)))
        .min(length);
    (from, to)
}

/// `root` and every sequence nested in it, each once.
fn reachable<'a>(project: &'a Project, root: &Id) -> Vec<&'a Sequence> {
    let mut found: Vec<&Sequence> = Vec::new();
    let mut next = vec![root.clone()];
    while let Some(id) = next.pop() {
        if found.iter().any(|sequence| sequence.id == id) {
            continue;
        }
        let Some(sequence) = project.sequence(id.as_str()) else {
            continue;
        };
        found.push(sequence);
        for clip in sequence.tracks.iter().flat_map(|track| &track.clips) {
            if let Content::Sequence { sequence, .. } = &clip.content {
                next.push(sequence.clone());
            }
        }
    }
    found
}

/// The seed of a clip's raster effects: FNV-1a of its id, so random effects
/// differ between clips and match between bakes and live playback.
pub fn seed(id: &str) -> u32 {
    id.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    fn project(text: &str) -> Project {
        kerf_lang::parse_document(text, Catalog::builtin())
            .expect("valid document")
            .project
    }

    /// Plan the first sequence, placing each file under a name for what
    /// makes it.
    fn planned(project: &Project) -> Planned {
        let mut count = 0;
        plan(project, Catalog::builtin(), None, |job| {
            count += 1;
            Ok::<_, Infallible>(match job {
                Job::Prerender(_) => format!("/cache/prerender-{count}.mkv"),
                Job::Bake(_) => format!("/cache/bake-{count}.mkv"),
            })
        })
        .unwrap()
    }

    const DOCUMENT: &str = "kerf 1
project name=\"fx\"
media m1 path=\"a.mov\" rate=24 dur=10s video=1920x1080 audio=2ch@48000
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 src=48f at=0f dur=48f
      fx color.huesaturation@1
      fx color.basic@1 exposure=0.5
    clip c2 media=m1 src=120f at=48f dur=24f
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=12f
";

    #[test]
    fn baked_clips_play_their_frames_and_keep_graph_effects() {
        let original = project(DOCUMENT);
        let planned = planned(&original);
        assert_eq!(planned.jobs.len(), 1);
        assert!(planned.prerenders.is_empty());
        let job = &planned.jobs[0];
        let frame = |n| Rate::FPS_24.time_of(n);
        // The clip's two seconds, plus the dissolve's handle after it.
        assert_eq!((job.from, job.to), (frame(48), frame(96 + 12)));
        assert_eq!(job.effects.len(), 1);
        assert_eq!(job.clip_time(frame(60)), frame(12));

        let clip = planned.project.clip("c1").unwrap();
        let Content::Media {
            media, source_in, ..
        } = &clip.content
        else {
            panic!("still a media clip");
        };
        let media = planned.project.media(media.as_str()).unwrap();
        assert_eq!(media.path, job.output);
        assert!(media.video.unwrap().alpha);
        assert_eq!(*source_in, Time::ZERO);
        assert_eq!(media.duration, job.to - job.from);
        let effects: Vec<_> = clip.effects.iter().map(|e| e.fx.as_str()).collect();
        assert_eq!(effects, ["color.basic"]);
        assert!(planned.project.clip("c2").unwrap().effects.is_empty());
    }

    #[test]
    fn titles_and_nested_sequences_are_rendered_alone_first() {
        let original = project(
            "kerf 1
project name=\"fx\"
media m1 path=\"a.mov\" rate=24 dur=10s video=1920x1080
sequence s1 rate=24 size=1280x720
  track V1 video
    clip n1 seq=s2 src=24f at=0f dur=48f
      fx color.curves@1
  track V2 video
    clip t1 gen=title@1 at=0f dur=24f
      set text=\"Hello\" y=0.3
      fx color.curves@1
sequence s2 rate=24 size=640x360
  track V1 video
    clip c1 media=m1 src=0f at=0f dur=96f
      fx color.huesaturation@1
",
        );
        let planned = planned(&original);
        assert_eq!(planned.prerenders.len(), 2);
        assert_eq!(planned.jobs.len(), 3);
        let frame = |n| Rate::FPS_24.time_of(n);

        // The nested sequence over the frames the clip shows.
        let nested = &planned.prerenders[0];
        assert_eq!(nested.sequence.as_str(), "s2");
        assert_eq!((nested.from, nested.to), (frame(24), frame(72)));
        let job = &planned.jobs[0];
        assert_eq!(job.prerender, Some(0));
        assert_eq!(job.source, nested.output);
        assert_eq!((job.width, job.height), (640, 360));

        // The title alone in a sequence of its own, at the clip's size.
        let title = &planned.prerenders[1];
        let alone = title.project.sequence(title.sequence.as_str()).unwrap();
        assert_eq!((alone.width, alone.height), (1280, 720));
        let clip = &alone.tracks[0].clips[0];
        assert!(clip.props.is_empty() && clip.effects.is_empty());
        assert_eq!(clip.duration, frame(24));

        // Both clips now play media; the title keeps where it was placed.
        let title_clip = planned.project.clip("t1").unwrap();
        assert!(matches!(title_clip.content, Content::Media { .. }));
        assert!(title_clip.props.contains_key("y"));
        assert!(title_clip.effects.is_empty());
        assert!(matches!(
            planned.project.clip("n1").unwrap().content,
            Content::Media { .. }
        ));
        // The nested sequence's own clip is baked too, for anything else
        // that plays it; its prerender renders it from the project as it was.
        assert_eq!(planned.jobs[2].source, "a.mov");
        assert_eq!(nested.project.sequence("s2"), original.sequence("s2"));
    }

    #[test]
    fn projects_without_raster_effects_need_no_bakes() {
        let original = project(&DOCUMENT.replace("      fx color.huesaturation@1\n", ""));
        let planned = planned(&original);
        assert!(planned.jobs.is_empty());
        assert_eq!(planned.project, original);
    }
}
