//! `look` with `mesh`: what a `gen=mesh@1` clip's glTF file holds, and a camera of it, or one
//! a model found and wrote to a file, as a camera clip's keys; or the place that puts the file
//! where another mesh's of the same space is.

use super::tools::MeshInput;
use super::{catalog, resolve_project_source_path};
use crate::{KerfMcpRouter, McpCode, McpError};
use kerf_compose::{CameraPlace, Compositor, MeshFile};
use kerf_model::{ClipAt, Content, Project, Sequence};
use serde_json::{json, Value};

/// The mesh clip `look` with `mesh` reads, and what it is read with.
struct MeshClip<'a> {
    project: &'a Project,
    /// The project with its media where they are, which `compositor` draws.
    resolved: &'a Project,
    input: &'a MeshInput,
    at: ClipAt,
    sequence: &'a Sequence,
    compositor: &'a Compositor,
    file: &'a MeshFile,
    /// The frame of the sequence the clip starts on, and how many frames it runs.
    first: i64,
    length: i64,
}

fn invalid(message: String) -> McpError {
    McpError::new(McpCode::InvalidArguments, message)
}

/// `value` to three places, as `mesh` says its numbers.
fn rounded(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

impl KerfMcpRouter {
    /// What the file of a mesh clip holds, and one of its cameras as a camera clip's keys.
    pub(super) fn mesh_file(
        &self,
        project: &Project,
        revision: u64,
        input: &MeshInput,
    ) -> Result<Value, McpError> {
        let at = self.clip_located(project, &input.clip)?;
        let clip = project.clip_at(at);
        let sequence = &project.sequences[at.sequence];
        let mut resolved = project.clone();
        kerf_runtime::resolve_media_paths(&mut resolved, &self.dir);
        let compositor = kerf_compose::Compositor::new(
            &resolved,
            catalog(),
            &kerf_compose::ComposeOptions {
                sequence: Some(sequence.id.to_string()),
                ..Default::default()
            },
        )
        .map_err(|error| McpError::new(McpCode::RenderFailed, error.to_string()))?;
        let file = compositor.mesh_file(&input.clip).map_err(|why| {
            invalid(format!(
                "`{}`: {why}; `mesh` is of a `gen=mesh@1` clip with a `file`",
                input.clip
            ))
        })?;
        let mut result = file_said(revision, &input.clip, &file);
        // Where the camera is on each frame of the mesh's clip.
        let rate = sequence.rate;
        let (first, length) = (
            rate.frames_round(clip.start),
            rate.frames_round(clip.duration).max(1),
        );
        let mesh = MeshClip {
            project,
            resolved: &resolved,
            input,
            at,
            sequence,
            compositor: &compositor,
            file: &file,
            first,
            length,
        };
        if let Some(beside) = &input.beside {
            return self.mesh_beside(&mesh, beside, result);
        }
        let (places, fov) = match (&input.camera, &input.from) {
            (None, None) => return Ok(result),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "`camera` is one the mesh's file holds and `from` a file that says where another was: give one".to_owned(),
                ));
            }
            (Some(wanted), None) => mesh.file_camera(wanted, &mut result)?,
            (None, Some(from)) => self.camera_from(&mesh, from, &mut result)?,
        };
        if places.is_empty() {
            return Err(McpError::new(
                McpCode::RenderFailed,
                format!("the camera could not be placed on `{}`", input.clip),
            ));
        }
        let (script, moves) = self.camera_script(&mesh, &places, fov)?;
        result["moves"] = json!(moves);
        result["script"] = json!(script);
        if input.camera.is_some() && !moves && file.playing.is_none() && !file.animations.is_empty()
        {
            result["note"] = json!(format!(
                "`{}` plays no animation, so the camera is where it is at rest: set `animation=` on the clip, and ask again, to have it move as the file moves it.",
                input.clip
            ));
        }
        Ok(result)
    }

    /// Where the clip `id` is in `project`, or what says it is not there.
    fn clip_located(&self, project: &Project, id: &str) -> Result<ClipAt, McpError> {
        project.locate(id).ok_or_else(|| {
            invalid(format!(
                "{}{}",
                project.no_clip(id),
                self.any_pending("proposal")
            ))
        })
    }

    /// `mesh` with `beside`: the place that puts the clip's file where the file of `beside`,
    /// another mesh clip of the same space, is.
    fn mesh_beside(
        &self,
        mesh: &MeshClip,
        beside: &str,
        mut result: Value,
    ) -> Result<Value, McpError> {
        let input = mesh.input;
        if input.camera.is_some() || input.from.is_some() {
            return Err(invalid(
                "`beside` puts a mesh's file where another's is, and `camera` and `from` write a camera: ask for one".to_owned(),
            ));
        }
        let other = self.clip_located(mesh.project, beside)?;
        if other.sequence != mesh.at.sequence {
            return Err(invalid(format!(
                "`{beside}` is in another sequence than `{}`",
                input.clip
            )));
        }
        let placed = crate::set_scale::beside(mesh.resolved, mesh.at.sequence, &input.clip, beside)
            .map_err(invalid)?;
        result["beside"] = json!(beside);
        result["script"] = json!(placed);
        Ok(result)
    }

    /// `mesh` with `from`: a camera the mesh's file does not hold, from `from`, a file of the
    /// project's folder that says where one was, as a model wrote it: where it is on each frame
    /// of the clip, and its lens.
    fn camera_from(
        &self,
        mesh: &MeshClip,
        from: &str,
        result: &mut Value,
    ) -> Result<(Vec<CameraPlace>, f64), McpError> {
        let input = mesh.input;
        let path = resolve_project_source_path(&self.dir, from);
        let text = std::fs::read_to_string(&path)
            .map_err(|why| invalid(format!("`{from}` cannot be read: {why}")))?;
        let origin = input.frame.unwrap_or(0);
        let mut lens = match input.lens {
            Some(lens) if lens > 0.0 && lens < 180.0 => Some(lens),
            Some(other) => {
                return Err(invalid(format!(
                    "`lens` is degrees down the picture, between 0 and 180, not {other}"
                )))
            }
            None => None,
        };
        let (scale, deep) =
            match (&input.fits, input.scale) {
                (Some(_), Some(_)) => return Err(invalid(
                    "`scale` is the size given and `fits` the footage to find it from: give one"
                        .to_owned(),
                )),
                (None, scale) => (scale.unwrap_or(1.0), 1.0),
                (Some(footage), None) => self.scale_fitted(
                    mesh,
                    (from, text.as_str(), origin),
                    footage,
                    &mut lens,
                    result,
                )?,
            };
        if !(scale > 0.0 && scale.is_finite()) {
            return Err(invalid(format!("`scale` is more than 0, not {scale}")));
        }
        let mut found = crate::camera_file::read(&text, origin, scale)
            .map_err(|why| invalid(format!("`{from}`: {why}")))?;
        // Forward is along the camera the set was seen by, which is the file's z.
        for view in &mut found.views {
            view.eye[2] *= deep;
        }
        let places = mesh.path_places(&found)?;
        let frames = found.views.len();
        let least = found.fovs.iter().copied().fold(f64::INFINITY, f64::min);
        let most = found.fovs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        result["from"] = json!({
            "frames": frames,
            "placed": places.len(),
            "fov": [rounded(least), rounded(most)],
        });
        mesh.note_path(from, &found, &places, result);
        Ok((places, lens.unwrap_or_else(|| found.lens())))
    }

    /// `mesh` with `fits`: the size of the camera path `from` against the set, found by
    /// drawing the set through it at a run of sizes on a few frames of the footage and holding
    /// it to them, and how much deeper the set is for the lens it is seen through, the set's own
    /// where `lens` gives none and the footage says one. What was found is said in `fit`.
    fn scale_fitted(
        &self,
        mesh: &MeshClip,
        (from, text, origin): (&str, &str, usize),
        footage: &str,
        lens: &mut Option<f64>,
        result: &mut Value,
    ) -> Result<(f64, f64), McpError> {
        let (input, rate) = (mesh.input, mesh.sequence.rate);
        let media = mesh.project.media(footage).ok_or_else(|| {
            invalid(format!(
                "no media `{footage}`: `fits` is the footage `{from}` is the camera of"
            ))
        })?;
        // At its own size, the path's places, to be made larger and smaller.
        let unit = crate::camera_file::read(text, origin, 1.0)
            .map_err(|why| invalid(format!("`{from}`: {why}")))?;
        let eyes: Vec<[f64; 3]> = unit.views.iter().map(|view| view.eye).collect();
        let frames = crate::set_scale::frames_to_read(&eyes, origin);
        if frames.is_empty() {
            return Err(invalid(format!(
                "the camera of `{from}` does not move, so the set looks the same at any size: `scale` is not to be found, and is not needed"
            )));
        }
        let seen = crate::set_scale::Seen {
            project: mesh.resolved,
            sequence: mesh.at.sequence,
            clip: &input.clip,
            small: (180.0 / f64::from(mesh.sequence.height.max(1))).min(1.0),
        };
        let size = seen.size().ok_or_else(|| {
            McpError::new(
                McpCode::RenderFailed,
                format!("`{}` could not be drawn", input.clip),
            )
        })?;
        let per_second = input.rate.unwrap_or_else(|| rate.fps());
        let mut pictures: Vec<Option<Vec<u8>>> = vec![None; frames.len()];
        let mut own: Option<Vec<u8>> = None;
        let mut number = 0usize;
        kerf_engine::media::index::small_color_frames(
            &resolve_project_source_path(&self.dir, &media.path),
            per_second.round().max(1.0) as u32,
            size,
            |_, rgba| {
                if let Some(index) = frames.iter().position(|frame| *frame == number) {
                    pictures[index] = Some(rgba.to_vec());
                }
                if number == origin {
                    own = Some(rgba.to_vec());
                }
                number += 1;
            },
        )
        .map_err(|error| McpError::new(McpCode::MediaFailed, error.to_string()))?;
        let Some(pictures) = pictures.into_iter().collect::<Option<Vec<_>>>() else {
            return Err(invalid(format!(
                "`{footage}` has {number} frames at {per_second} a second, and `{from}` is of more: `fits` is the footage the path was found of, and `rate` its frames a second"
            )));
        };
        // The set's own lens, where none is given and the footage says one.
        let through = set_lens(&seen, own.as_deref(), lens);
        // A set is as deep as the lens its model took its frame to have: through
        // a narrower one than the path's the same picture is of things further
        // off, so what the path goes forward goes that much further in the set.
        let deep = lens.map_or(1.0, |lens| crate::set_scale::deeper(unit.lens(), lens));
        let fit = crate::set_scale::fit(
            &seen,
            lens.unwrap_or_else(|| unit.lens()),
            &frames,
            &pictures,
            |scale, frame| {
                let view = unit.views.get(frame)?;
                Some(kerf_compose::FileView {
                    eye: [
                        view.eye[0] * scale,
                        view.eye[1] * scale,
                        view.eye[2] * scale * deep,
                    ],
                    ..*view
                })
            },
        )
        .map_err(|why| invalid(format!("`{footage}`: {why}")))?;
        result["fit"] = json!({
            "lens": through,
            "deep": rounded(deep),
            "scale": rounded(fit.scale),
            "error": rounded(fit.error),
            "sure": rounded(fit.sure),
            "frames": fit.frames,
        });
        Ok((fit.scale, deep))
    }

    /// The line that writes `places` as a camera clip's keys, with `fov` its lens: on the
    /// camera clip `as` names, or on a new one over the mesh's clip; and whether the camera
    /// moves.
    fn camera_script(
        &self,
        mesh: &MeshClip,
        places: &[CameraPlace],
        fov: f64,
    ) -> Result<(String, bool), McpError> {
        let (project, input, sequence) = (mesh.project, mesh.input, mesh.sequence);
        let (rate, first, length) = (sequence.rate, mesh.first, mesh.length);
        let target = match &input.onto {
            Some(id) => {
                let target = project.clip_at(self.clip_located(project, id)?);
                if !matches!(&target.content, Content::Generator { fx, .. } if fx.as_str() == "camera")
                {
                    return Err(invalid(format!(
                        "`{id}` is not a camera: `as` is a `gen=camera@1` clip, or leave it out for a new one"
                    )));
                }
                Some(target)
            }
            None => None,
        };
        // A clip's keys count from its own first frame.
        let origin = target.map_or(first, |target| rate.frames_round(target.start));
        // What a model found of a camera is not as fine as what a file says of its own.
        let fine = if input.from.is_some() { 10.0 } else { 1.0 };
        let (params, moves) = crate::mesh_camera::params(places, first - origin, fov, fine);
        let script = match target {
            Some(target) => format!("set {} {params}", target.id),
            None => {
                // A new camera on a new track, which is on top and so is the one that sees.
                let free = |taken: &dyn Fn(&str) -> bool, stem: &str| {
                    (1..)
                        .map(|index| format!("{stem}{index}"))
                        .find(|name| !taken(name))
                        .unwrap_or_default()
                };
                let on = free(&|name| sequence.track(name).is_some(), "V");
                let id = free(&|name| project.locate(name).is_some(), "camera");
                // Where there is more than one sequence, a line that adds says which.
                let within = if project.sequences.len() > 1 {
                    format!("use {}\n", sequence.id)
                } else {
                    String::new()
                };
                format!(
                    "{within}track add {on} video\noverwrite gen=camera@1 at={first}f dur={length}f on={on} as={id}\nset {id} {params}"
                )
            }
        };
        Ok((script, moves))
    }
}

impl MeshClip<'_> {
    /// A camera the mesh's file holds, `wanted` by its name or its number: where it is on each
    /// frame of the clip, and its lens.
    fn file_camera(
        &self,
        wanted: &Value,
        result: &mut Value,
    ) -> Result<(Vec<CameraPlace>, f64), McpError> {
        let (file, input, rate) = (self.file, self.input, self.sequence.rate);
        let wanted = match wanted {
            Value::String(name) => name.clone(),
            other => other.to_string(),
        };
        let which = file
            .cameras
            .iter()
            .position(|camera| camera.name == wanted)
            .or_else(|| wanted.parse().ok().filter(|number| *number < file.cameras.len()))
            .ok_or_else(|| {
                let has: Vec<String> = file
                    .cameras
                    .iter()
                    .enumerate()
                    .map(|(number, camera)| format!("{number} `{}`", camera.name))
                    .collect();
                invalid(match has.len() {
                    0 => format!(
                        "the file of `{}` has no camera: `from` takes a file that says where one was",
                        input.clip
                    ),
                    _ => format!(
                        "the file of `{}` has no camera `{wanted}`: it has {}",
                        input.clip,
                        has.join(", ")
                    ),
                })
            })?;
        let fov = file.cameras[which].fov.ok_or_else(|| {
            invalid(format!(
                "camera `{wanted}` has no perspective, and a `gen=camera@1` has a lens"
            ))
        })?;
        result["camera"] = json!(which);
        let places: Vec<kerf_compose::CameraPlace> = (0..self.length)
            .filter_map(|frame| {
                self.compositor
                    .mesh_camera(&input.clip, which, rate.time_of(self.first + frame))
            })
            .collect();
        Ok((places, fov))
    }

    /// Where `found`, a file's camera path, has the camera on each frame of the clip it says
    /// where it was.
    fn path_places(&self, found: &crate::camera_file::Found) -> Result<Vec<CameraPlace>, McpError> {
        let (input, rate) = (self.input, self.sequence.rate);
        // The file's frames are its shot's, which may not be the sequence's.
        let a_frame = match input.rate {
            Some(per_second) if per_second > 0.0 && per_second.is_finite() => {
                per_second / rate.fps()
            }
            Some(other) => return Err(invalid(format!("`rate` is frames a second, not {other}"))),
            None => 1.0,
        };
        let frames = found.views.len();
        let lasts = (((frames - 1) as f64 / a_frame).floor() as i64 + 1).min(self.length);
        let places: Vec<kerf_compose::CameraPlace> = (0..lasts)
            .filter_map(|frame| {
                self.compositor.mesh_view(
                    &input.clip,
                    &found.at(frame as f64 * a_frame),
                    rate.time_of(self.first + frame),
                )
            })
            .collect();
        Ok(places)
    }

    /// What `mesh` notes of the camera path `from`, `found`, that placed the camera on the
    /// clip at `places`: that it counts its places from another frame than `frame`, that it
    /// ends before the clip does, or that a size half or twice the one `fits` found fits the
    /// footage nearly as well.
    fn note_path(
        &self,
        from: &str,
        found: &crate::camera_file::Found,
        places: &[CameraPlace],
        result: &mut Value,
    ) {
        let (input, length) = (self.input, self.length);
        let origin = input.frame.unwrap_or(0);
        let frames = found.views.len();
        let mut notes = Vec::new();
        if let Some(anchor) = found.anchor.filter(|_| frames > 1) {
            result["from"]["anchor"] = json!(anchor);
            let given = input.frame.unwrap_or(0);
            if anchor != given {
                notes.push(format!(
                    "`{from}` counts its places from frame {anchor}, and `frame` is {given}. A set made of the same reading as the path is in the space of frame {anchor} whichever frame it shows, and wants `frame: {anchor}`; a set another model rebuilt of frame {given} alone is as asked."
                ));
            }
        }
        if frames > 1 && (places.len() as i64) < length {
            notes.push(format!(
                "`{from}` says where the camera was for {} of the {length} frames of `{}`: it holds still after.",
                places.len(),
                input.clip
            ));
        }
        if let Some(fit) = result.get("fit").cloned()
            && fit["sure"].as_f64().is_some_and(|sure| sure < 1.1)
        {
            notes.push(format!(
                    "a size half or twice {} fits the footage nearly as well (`sure` {}): the camera does not travel far enough through the set to say, or the set is not of frame {origin} of `{from}`.",
                    fit["scale"], fit["sure"]
                ));
        }
        if !notes.is_empty() {
            result["note"] = json!(notes.join(" "));
        }
    }
}

/// What `mesh` says of the file of `clip`: its triangles, its size at rest, its animations and
/// the one the clip plays, and its cameras.
fn file_said(revision: u64, clip: &str, file: &MeshFile) -> Value {
    json!({
        "revision": revision,
        "clip": clip,
        "triangles": file.triangles,
        "size": file.size.map(rounded),
        "animations": file.animations.iter().enumerate().map(|(number, (name, seconds))| json!({
            "number": number, "name": name, "seconds": rounded(*seconds),
        })).collect::<Vec<_>>(),
        "playing": file.playing,
        "cameras": file.cameras.iter().enumerate().map(|(number, camera)| json!({
            "number": number, "name": camera.name,
            "fov": camera.fov.map(rounded), "aspect": camera.aspect.map(rounded),
        })).collect::<Vec<_>>(),
    })
}

/// The set's own lens, through which the set seen from the mesh's frame is the footage's `own`
/// frame again, as `fit.lens` says it; the camera gets it, in `lens`, where none is given and
/// it is sure of it.
fn set_lens(seen: &crate::set_scale::Seen, own: Option<&[u8]>, lens: &mut Option<f64>) -> Value {
    let mut through = json!(null);
    if lens.is_none() {
        let said =
            own.and_then(|own| crate::set_scale::lens(seen, &kerf_compose::FileView::ORIGIN, own));
        if let Some((fov, error, sure)) = said {
            through = json!({"fov": rounded(fov), "error": rounded(error), "sure": rounded(sure)});
            if sure >= 1.1 {
                *lens = Some(fov);
            }
        }
    }
    through
}
