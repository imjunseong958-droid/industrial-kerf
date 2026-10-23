//! Raster effects on media clips, baked before the graph runs.
//!
//! The graph cannot run raster effects (`Engine::Raster`). A render first
//! bakes each media clip's raster effects into a file of the source frames
//! the clip can show, and the graph plays that file in the clip's place.
//! [`plan`] says which files a project needs and rewrites the project to
//! use them. It reads and writes nothing, so a check compiles exactly what
//! a render would; `kerf-engine` makes the files.

use kerf_model::{
    Catalog, Clip, Content, Effect, Engine, Media, Project, Rate, Ratio, Time, TrackKind,
};

/// One media clip's raster effects over the source frames it can show.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeJob {
    /// The media file the frames come from.
    pub source: String,
    pub rate: Rate,
    /// The picture the effects measure against: the media's own size.
    pub width: u32,
    pub height: u32,
    /// The source interval baked, on the media's frames.
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

    /// Everything the baked frames depend on but the media's contents.
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

/// A project whose media clips play their baked frames, and the bakes it
/// needs.
#[derive(Clone, Debug)]
pub struct Planned {
    pub project: Project,
    pub jobs: Vec<BakeJob>,
}

/// Plan the bakes `project` needs: every media clip on a picture track with
/// an enabled raster effect plays a new media in `place(job)` instead, with
/// its raster effects removed. Raster effects anywhere else stay, and the
/// graph reports them.
pub fn plan<E>(
    project: &Project,
    catalog: &Catalog,
    mut place: impl FnMut(&BakeJob) -> Result<String, E>,
) -> Result<Planned, E> {
    let raster = |effect: &Effect| {
        catalog
            .fx(effect.fx.as_str())
            .is_some_and(|fx| fx.engine == Engine::Raster)
    };
    let mut planned = project.clone();
    let mut jobs = Vec::new();
    let mut ids = project.id_gen();
    let mut added = Vec::new();
    for sequence in &mut planned.sequences {
        for track in sequence
            .tracks
            .iter_mut()
            .filter(|t| t.kind == TrackKind::Video)
        {
            let handles = |clip: &Clip| {
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
            };
            let mut baked = Vec::new();
            for clip in &track.clips {
                let Content::Media {
                    media,
                    source_in,
                    speed,
                } = &clip.content
                else {
                    continue;
                };
                let effects: Vec<Effect> = clip
                    .effects
                    .iter()
                    .filter(|effect| effect.enabled && raster(effect))
                    .cloned()
                    .collect();
                let Some(media) = project.media(media.as_str()) else {
                    continue;
                };
                let Some(video) = media.video.filter(|_| !effects.is_empty()) else {
                    continue;
                };
                let (pre, post) = handles(clip);
                let (a, b) = source_window(clip, clip.start - pre, clip.end() + post);
                let rate = media.rate;
                let from = rate.time_of(rate.frames_floor(a)).max(Time::ZERO);
                let to = rate
                    .time_of(rate.frames_floor(b + rate.frame_duration() - Time::from_flicks(1)))
                    .min(media.duration);
                let mut job = BakeJob {
                    source: media.path.clone(),
                    rate,
                    width: video.width,
                    height: video.height,
                    from,
                    to,
                    source_in: *source_in,
                    speed: *speed,
                    clip_duration: clip.duration,
                    effects,
                    seed: seed(clip.id.as_str()),
                    output: String::new(),
                };
                job.output = place(&job)?;
                let id = ids.next(&format!("{}fx", media.id));
                added.push(Media {
                    id: id.clone(),
                    path: job.output.clone(),
                    start: Time::ZERO,
                    duration: to - from,
                    audio: None,
                    reel: None,
                    ..media.clone()
                });
                baked.push((clip.id.clone(), id, *source_in - from, *speed));
                jobs.push(job);
            }
            for (clip, media, source_in, speed) in baked {
                let clip = track
                    .clips
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .expect("baked clips stay on their track");
                clip.content = Content::Media {
                    media,
                    source_in,
                    speed,
                };
                clip.effects.retain(|effect| !raster(effect));
            }
        }
    }
    planned.media.extend(added);
    Ok(Planned {
        project: planned,
        jobs,
    })
}

/// The source interval a clip plays over the sequence interval `[a, b)`,
/// extended past its ends as the clip's handles are.
fn source_window(clip: &Clip, a: Time, b: Time) -> (Time, Time) {
    let Content::Media {
        source_in, speed, ..
    } = &clip.content
    else {
        unreachable!("only media clips are baked");
    };
    let (a, b) = (a - clip.start, b - clip.start);
    if speed.is_reverse() {
        let out = *source_in + speed.source_span(clip.duration);
        (out - speed.source_span(b), out - speed.source_span(a))
    } else {
        (
            *source_in + speed.source_span(a),
            *source_in + speed.source_span(b),
        )
    }
}

/// FNV-1a of a clip id, so random effects differ between clips.
fn seed(id: &str) -> u32 {
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

    const DOCUMENT: &str = "kerf 1
project name=\"fx\"
media m1 path=\"a.mov\" rate=24 dur=10s video=1920x1080 audio=2ch@48000
sequence s1 rate=24 size=1920x1080
  track V1 video
    clip c1 media=m1 src=48f at=0f dur=48f
      fx channel.invert
      fx color.basic exposure=0.5
    clip c2 media=m1 src=120f at=48f dur=24f
    transition t1 from=c1 to=c2 fx=dissolve dur=12f
";

    #[test]
    fn baked_clips_play_their_frames_and_keep_graph_effects() {
        let original = project(DOCUMENT);
        let planned = plan(&original, Catalog::builtin(), |job| {
            Ok::<_, Infallible>(format!("/cache/{}.mkv", job.seed))
        })
        .unwrap();
        assert_eq!(planned.jobs.len(), 1);
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
        assert_eq!(*source_in, Time::ZERO);
        assert_eq!(media.duration, job.to - job.from);
        let effects: Vec<_> = clip.effects.iter().map(|e| e.fx.as_str()).collect();
        assert_eq!(effects, ["color.basic"]);
        assert!(planned.project.clip("c2").unwrap().effects.is_empty());
    }

    #[test]
    fn projects_without_raster_effects_need_no_bakes() {
        let original = project(&DOCUMENT.replace("      fx channel.invert\n", ""));
        let planned = plan(&original, Catalog::builtin(), |_| {
            Ok::<_, Infallible>(String::new())
        })
        .unwrap();
        assert!(planned.jobs.is_empty());
        assert_eq!(planned.project, original);
    }
}
