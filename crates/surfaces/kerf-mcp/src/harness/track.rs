//! `look` with `track`: where a point, a surface, or an object of the program's picture goes,
//! or where the camera went, as keyframes and the line that sets a clip to follow it.

use super::tools::LookInput;
use super::{program, program_position, SCOPE_WIDTH};
use crate::{KerfMcpRouter, McpError};
use kerf_model::{Clip, Project, Sequence};
use serde_json::{json, Value};

/// The most frames one `track` follows.
const MAX_TRACK_FRAMES: i64 = kerf_shots::tracks::MOST_FRAMES;

/// The most places in the world a solved camera offers to pin to.
const MOST_PINS: usize = 24;

/// How far a followed point may stray from the keyframes it is written with, in pixels of the frame
/// it was followed on: finer than that is the tracker's own jitter.
const TRACK_TOLERANCE_PIXELS: f64 = kerf_shots::tracks::POINT_TOLERANCE;

impl KerfMcpRouter {
    /// Where a point of the program's picture goes over `from`–`to`.
    pub(super) fn program_track(
        &self,
        project: &Project,
        revision: u64,
        input: &LookInput,
    ) -> Result<Value, McpError> {
        let Some(track) = &input.track else {
            return Err(McpError::new("INVALID_ARGUMENTS", "nothing to track"));
        };
        let sequence = program(project, input.sequence.as_deref())?;
        let duration = sequence.duration();
        // Without a range, for as long as the clip that will follow runs.
        let whole = track.clip.as_deref().and_then(|id| {
            sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|clip| clip.id.as_str() == id)
                .map(|clip| (clip.start, clip.end().min(duration)))
        });
        let (from, to) = match (&input.from, &input.to, whole) {
            (Some(from), Some(to), _) => (
                program_position(project, sequence, from)?,
                program_position(project, sequence, to)?,
            ),
            (None, None, Some(whole)) => whole,
            _ => {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "track needs `from` and `to`, or a `clip` of the sequence to follow for as long as it runs",
                ))
            }
        };
        if from.is_negative() || to > duration || to <= from {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("`from`–`to` is not a range of sequence `{}`", sequence.id),
            ));
        }
        let object = !track.inside.is_empty() || track.bbox.is_some();
        let point = match (track.x, track.y, track.corners, object, track.camera) {
            (Some(x), Some(y), None, false, false) => Some([x, y]),
            (None, None, Some(_), false, false)
            | (None, None, None, true, false)
            | (None, None, None, false, true) => None,
            _ => {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "give `x` and `y` to follow a point, `corners` to follow a surface, `inside` or `box` to follow an object, or `camera` to solve the camera",
                ))
            }
        };
        if let Some(fov) = track.fov {
            if !track.camera || !(1.0..=170.0).contains(&fov) {
                return Err(McpError::new(
                    "INVALID_ARGUMENTS",
                    "`fov` goes with `camera`, from 1 to 170 degrees",
                ));
            }
        }
        if !object && !track.outside.is_empty() {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "`outside` goes with `inside` or `box`",
            ));
        }
        let within = |value: &f64| (0.0..=1.0).contains(value);
        let all_within = point
            .iter()
            .chain(track.corners.iter().flatten())
            .chain(&track.inside)
            .chain(&track.outside)
            .flatten()
            .chain(track.bbox.iter().flatten())
            .all(within);
        if !all_within {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                "`x` and `y` run 0 to 1 across the frame",
            ));
        }
        let rate = sequence.rate;
        let frame = rate.frame_duration().flicks();
        let first = from.flicks().div_euclid(frame);
        let count = (to.flicks() - from.flicks()).div_euclid(frame).max(1);
        if count > MAX_TRACK_FRAMES {
            return Err(McpError::new(
                "INVALID_ARGUMENTS",
                format!("track follows at most {MAX_TRACK_FRAMES} frames at a time"),
            ));
        }
        let target = match &track.clip {
            Some(id) => Some(
                sequence
                    .tracks
                    .iter()
                    .flat_map(|track| &track.clips)
                    .find(|clip| clip.id.as_str() == id)
                    .ok_or_else(|| {
                        McpError::new(
                            "INVALID_ARGUMENTS",
                            format!("no clip `{id}` in the sequence"),
                        )
                    })?,
            ),
            None => None,
        };
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        // The clip that will follow a point is not drawn while the point is followed, or what
        // was followed would be the clip.
        if let (Some(_), Some(at)) = (
            point,
            target.and_then(|clip| project.locate(clip.id.as_str())),
        ) {
            resolved.clip_at_mut(at).enabled = false;
        }
        let scale = (SCOPE_WIDTH / f64::from(sequence.width.max(1))).min(1.0);
        let id = sequence.id.as_str();
        // One drawer for the run: its frames are decoded once through. One made for each
        // frame decoded each from its media's last keyframe again, and following a point for
        // five seconds of a clip with one keyframe took 70 s.
        let drawer = kerf_engine::render::still::drawer(&resolved, Some(id))
            .map_err(|error| McpError::new("RENDER_FAILED", error.to_string()))?;
        let read = |index: usize| {
            let at = rate.time_of(first + index as i64);
            let at = if at >= duration { to } else { at };
            let frame = drawer.frame(at, scale);
            Some((frame.width as usize, frame.height as usize, frame.rgba))
        };
        let drawn_width = f64::from(sequence.width.max(1)) * scale;
        // Times on the clip that will follow, or from `from` without one.
        let origin = target.map_or(first, |clip| clip.start.flicks().div_euclid(frame));
        if track.camera {
            // Up to `to`, and up to the last frame there is when that is the end of the sequence.
            let frames = if to >= duration { count } else { count + 1 } as usize;
            let solved = kerf_track::camera::solve(frames, track.fov, read);
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": frames,
            });
            match solved {
                Ok(shot) => {
                    solved_camera(&shot, sequence, target, (first, origin), scale, &mut result)
                }
                Err(why) => {
                    result["solved"] = json!(false);
                    result["reason"] = json!(why.to_string());
                }
            }
            return Ok(result);
        }
        if object {
            let hints = kerf_track::Hints {
                inside: track.inside.clone(),
                outside: track.outside.clone(),
                bbox: track.bbox,
            };
            let followed = kerf_track::follow_object(count as usize + 1, &hints, read);
            let pixels = picture_width(project, target).unwrap_or(f64::from(sequence.width));
            let cut = kerf_shots::tracks::outline(&followed, first - origin, drawn_width, pixels);
            let (followed, kept, outline) = (cut.followed, cut.kept, cut.points);
            let rounded = |value: f64| (value * 10_000.0).round() / 10_000.0;
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": followed.outlines.len(),
                "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
                "keyframes": kept.len(),
                "corners": followed.outlines.first().map_or(0, Vec::len),
                "area": followed.areas.last().map(|area| rounded(*area)),
            });
            // The outline once: in the line that masks the clip, or by itself without one.
            match target {
                Some(clip) => {
                    result["script"] =
                        json!(format!("mask {} add object points={outline}", clip.id));
                }
                None => result["outline"] = json!(outline),
            }
            return Ok(result);
        }
        if let Some(corners) = track.corners {
            let followed = kerf_track::follow_plane(count as usize + 1, corners, read);
            let kept =
                kerf_track::simplified_quads(&followed.quads, TRACK_TOLERANCE_PIXELS / drawn_width);
            let rounded = |value: f64| (value * 100_000.0).round() / 100_000.0;
            // One number of one corner over the kept frames, as keyframes.
            let channel = |corner: usize, axis: usize| {
                let words: Vec<String> = kept
                    .iter()
                    .map(|index| {
                        format!(
                            "{}f:{}",
                            first + *index as i64 - origin,
                            rounded(followed.quads[*index][corner][axis])
                        )
                    })
                    .collect();
                format!("{{{}}}", words.join(" "))
            };
            let words: Vec<String> = kept
                .iter()
                .map(|index| {
                    let corners: Vec<String> = followed.quads[*index]
                        .iter()
                        .map(|corner| format!("{},{}", rounded(corner[0]), rounded(corner[1])))
                        .collect();
                    format!("{}f:{}", first + *index as i64 - origin, corners.join(";"))
                })
                .collect();
            let mut result = json!({
                "sequence": sequence.id.to_string(),
                "revision": revision,
                "frames": followed.quads.len(),
                "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
                "quads": kept.iter().map(|index| json!({
                    "frame": first + *index as i64,
                    "corners": followed.quads[*index].map(|corner| corner.map(rounded)),
                    "agreeing": followed.agreeing[*index],
                })).collect::<Vec<_>>(),
                "outline": format!("{{{}}}", words.join(" ")),
            });
            if let Some(clip) = target {
                // Corner order, as the corner pin names them.
                let names = [
                    ("upper_left", 0),
                    ("upper_right", 1),
                    ("lower_right", 2),
                    ("lower_left", 3),
                ];
                let params: Vec<String> = names
                    .iter()
                    .map(|(name, corner)| {
                        format!(
                            "{name}_x={} {name}_y={}",
                            channel(*corner, 0),
                            channel(*corner, 1)
                        )
                    })
                    .collect();
                result["script"] = json!(format!(
                    "fx {} add distort.cornerpin@2 {}",
                    clip.id,
                    params.join(" ")
                ));
            }
            return Ok(result);
        }
        let point = point.ok_or_else(|| McpError::new("INVALID_ARGUMENTS", "nothing to follow"))?;
        let followed = kerf_track::follow(count as usize + 1, point, read);
        let (kept, x, y) = kerf_shots::tracks::point_keys(&followed, first - origin, drawn_width);
        let mut result = json!({
            "sequence": sequence.id.to_string(),
            "revision": revision,
            "frames": followed.points.len(),
            "lostAt": followed.lost_at.map(|index| format!("{}f", first + index as i64)),
            "points": kept.iter().map(|index| {
                let point = &followed.points[*index];
                json!({
                    "frame": first + *index as i64,
                    "x": (point.x * 10_000.0).round() / 10_000.0,
                    "y": (point.y * 10_000.0).round() / 10_000.0,
                    "confidence": (point.confidence * 100.0).round() / 100.0,
                })
            }).collect::<Vec<_>>(),
            "x": x,
            "y": y,
        });
        if let Some(clip) = target {
            result["script"] = json!(format!("set {} x={x} y={y}", clip.id));
        }
        Ok(result)
    }
}

/// What `look` says of a solved shot: its camera as keyframes, how well it fits, and where to pin
/// things. `first` is the frame the range starts on and `origin` the one keyframes count from.
/// `scale` is how much smaller than the sequence's frames it was read.
fn solved_camera(
    shot: &kerf_track::camera::SolvedShot,
    sequence: &Sequence,
    target: Option<&Clip>,
    (first, origin): (i64, i64),
    scale: f64,
    result: &mut Value,
) {
    let kept = shot.simplified(TRACK_TOLERANCE_PIXELS);
    let rounded = |value: f64, places: i32| {
        let by = 10f64.powi(places);
        (value * by).round() / by
    };
    let keys = |value: &dyn Fn(&kerf_track::camera::SolvedCamera) -> f64, places: i32| {
        let words: Vec<String> = kept
            .iter()
            .map(|index| {
                let camera = &shot.cameras[*index];
                format!(
                    "{}f:{}",
                    first + camera.frame as i64 - origin,
                    rounded(value(camera), places)
                )
            })
            .collect();
        format!("{{{}}}", words.join(" "))
    };
    let fov = rounded(shot.fov, 3);
    let lens = if shot.zooms {
        keys(&|camera| camera.fov, 3)
    } else {
        fov.to_string()
    };
    let params = format!(
        "x={} y={} z={} pan={} tilt={} roll={} fov={lens}",
        keys(&|camera| camera.x, 5),
        keys(&|camera| camera.y, 5),
        keys(&|camera| camera.z, 5),
        keys(&|camera| camera.pan, 4),
        keys(&|camera| camera.tilt, 4),
        keys(&|camera| camera.roll, 4),
    );
    let script = match target {
        Some(clip) => format!("set {} {params}", clip.id),
        None => {
            // A new camera on a new track, which is on top and so is the one that sees.
            let free = |taken: &dyn Fn(&str) -> bool, stem: &str| {
                (1..)
                    .map(|index| format!("{stem}{index}"))
                    .find(|name| !taken(name))
                    .unwrap_or_default()
            };
            let on = free(&|name| sequence.track(name).is_some(), "V");
            let id = free(
                &|name| {
                    sequence
                        .tracks
                        .iter()
                        .any(|track| track.clip(name).is_some())
                },
                "camera",
            );
            let dur = shot.cameras.last().map_or(1, |camera| camera.frame + 1);
            format!(
                "track add {on} video\noverwrite gen=camera@1 at={first}f dur={dur}f on={on} as={id}\nset {id} {params}"
            )
        }
    };
    result["solved"] = json!(true);
    result["solvedFrames"] = json!(shot.cameras.len());
    result["lostAt"] = json!(shot
        .ended_at
        .map(|index| format!("{}f", first + index as i64)));
    result["fov"] = json!(fov);
    result["lensKnown"] = json!(shot.lens_known);
    result["zooms"] = json!(shot.zooms);
    if shot.zoom_or_push {
        result["zoomOrPush"] = json!(
            "This shot cannot tell a zoom from a push in: what is in the picture grows alike, which \
             either would do. It is written as the camera moving along its line with one lens, and \
             if the lens zoomed that way is not true."
        );
    }
    result["fit"] = json!({
        "error": rounded(shot.error / scale, 3),
        "worst": rounded(shot.worst / scale, 3),
        "agreeing": rounded(shot.agreeing, 3),
    });
    result["keyframes"] = json!(kept.len());
    result["points"] = json!(shot
        .spread_points(MOST_PINS)
        .iter()
        .map(|point| json!({
            "x": rounded(point.x, 5),
            "y": rounded(point.y, 5),
            "z": rounded(point.z, 5),
            "frame": first + point.frame as i64,
            "at": [rounded(point.seen_at[0], 4), rounded(point.seen_at[1], 4)],
            "lastFrame": first + point.last_frame as i64,
        }))
        .collect::<Vec<_>>());
    result["script"] = json!(script);
}

/// How many pixels wide the picture of `clip` is, if it has one of its own.
fn picture_width(project: &kerf_model::Project, clip: Option<&kerf_model::Clip>) -> Option<f64> {
    let kerf_model::Content::Media { media, .. } = &clip?.content else {
        return None;
    };
    let media = project.media.iter().find(|found| found.id == *media)?;
    Some(f64::from(media.video.as_ref()?.width))
}
