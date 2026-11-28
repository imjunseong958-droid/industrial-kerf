//! Inverse Director Solver and $SE(3)$ tracking-camera planner for `look` with `direct`.
//!
//! Uses the exact [`kerf_compose::sequence_world_spec`] executed by the compositor so solved
//! launch velocities and camera tracking trajectories match rendered frames identically.

use super::tools::DirectInput;
use super::{program, program_position};
use crate::{KerfMcpRouter, McpCode, McpError};
use kerf_compose::sequence_world_spec;
use kerf_model::{Catalog, Project, Sequence, Time, TrackKind};
use kerf_sim::{plan_tracking_camera, solve_launch_velocity, TrajectoryGoal, WorldTimeline};
use serde_json::{json, Value};

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0 + 0.0
}

fn unwrap_deg(prev: f64, next: f64) -> f64 {
    let mut d = (next - prev) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    prev + d
}

fn format_keys(entries: &[(i64, f64)]) -> String {
    let inner: Vec<String> = entries
        .iter()
        .map(|(frame, val)| format!("{frame}f:{}", round4(*val)))
        .collect();
    format!("{{{}}}", inner.join(" "))
}

fn fresh_video_track(seq: &Sequence) -> String {
    let mut n = seq
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video)
        .count()
        + 1;
    loop {
        let candidate = format!("V{n}");
        if !seq.tracks.iter().any(|t| t.id.as_str() == candidate) {
            return candidate;
        }
        n += 1;
    }
}

impl KerfMcpRouter {
    /// Handle `look` with `direct`: either inverse trajectory velocity solving (`clip` + `target`
    /// + `at`) or smooth $SE(3)$ tracking camera planning (`follow`).
    pub(crate) fn direct_scene(
        &self,
        project: &Project,
        sequence_id: Option<&str>,
        revision: u64,
        req: &DirectInput,
    ) -> Result<Value, McpError> {
        let seq = program(project, sequence_id)?;
        let Some(world) = sequence_world_spec(project, Catalog::builtin(), Some(seq.id.as_str()))
            .map_err(|e| McpError::new(McpCode::HandlerFailed, e.to_string()))?
        else {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "sequence has no 3D clips with `physics=dynamic` or `physics=static`",
            ));
        };

        if let Some(follow_id) = &req.follow {
            if req.clip.is_some() || req.target.is_some() {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "pass either `clip`+`target`+`at` (launch solve) or `follow` (tracking camera), not both",
                ));
            }
            return plan_camera(seq, &world, revision, follow_id, req);
        }

        let Some(clip_id) = &req.clip else {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`direct` requires either `clip`+`target`+`at` or `follow`",
            ));
        };
        let Some(target) = req.target else {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`direct` with `clip` requires `target` as `[x, y, z]`",
            ));
        };
        let Some(at_str) = &req.at else {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "`direct` with `clip` requires `at` (arrival time, e.g. `1.2s` or `36f`)",
            ));
        };
        let arrive_at = program_position(project, seq, at_str)?;
        if arrive_at.flicks() <= 0 {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "arrival time `at` must be strictly after 0",
            ));
        }

        let Some(body_idx) = world.body_index(clip_id) else {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                format!(
                    "clip `{clip_id}` is not a 3D physics body in sequence `{}`",
                    seq.id
                ),
            ));
        };
        if !world.spec.bodies[body_idx].is_dynamic() {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                format!("clip `{clip_id}` must have `physics=dynamic` to solve launch velocity"),
            ));
        }

        // Convert Kerf clip target [x (frame widths), y (heights), z (heights)] to frame heights.
        let goal = TrajectoryGoal {
            body: body_idx,
            target_pos: [target[0] * world.aspect, target[1], target[2]],
            arrive_at,
        };
        let sol = solve_launch_velocity(&world.spec, &goal, 30, 1e-4).ok_or_else(|| {
            McpError::new(
                McpCode::HandlerFailed,
                format!("failed to solve launch velocity for `{clip_id}`"),
            )
        })?;

        let vx = round4(sol.initial_velocity[0]);
        let vy = round4(sol.initial_velocity[1]);
        let vz = round4(sol.initial_velocity[2]);
        let script = format!("set {clip_id} physics=dynamic vel_x={vx} vel_y={vy} vel_z={vz}");
        Ok(json!({
            "sequence": seq.id.to_string(),
            "revision": revision,
            "clip": clip_id,
            "arriveAt": at_str,
            "initialVelocity": [vx, vy, vz],
            "finalPos": [
                round4(sol.final_pos[0] / world.aspect),
                round4(sol.final_pos[1]),
                round4(sol.final_pos[2]),
            ],
            "error": round4(sol.error),
            "iterations": sol.iterations,
            "script": script,
        }))
    }
}

fn plan_camera(
    seq: &Sequence,
    world: &kerf_compose::SequenceWorldSpec,
    revision: u64,
    follow_id: &str,
    req: &DirectInput,
) -> Result<Value, McpError> {
    let Some(body_idx) = world.body_index(follow_id) else {
        return Err(McpError::new(
            McpCode::InvalidArguments,
            format!(
                "clip `{follow_id}` is not a 3D physics body in sequence `{}`",
                seq.id
            ),
        ));
    };
    let offset = req.offset.unwrap_or([0.0, -0.25, -2.2]);
    let smoothing = req.smoothing.unwrap_or(0.25).clamp(0.01, 1.0);
    let fov = req.fov.unwrap_or(40.0).clamp(5.0, 150.0);
    let every = i64::from(req.every.unwrap_or(2).max(1));

    let total_frames = ((seq.duration().seconds() * seq.rate.fps()).round() as i64).max(1);
    let mut frames: Vec<i64> = (0..=total_frames).step_by(every as usize).collect();
    if frames.last().copied() != Some(total_frames) {
        frames.push(total_frames);
    }
    let times: Vec<Time> = frames.iter().map(|&f| seq.rate.time_of(f)).collect();

    let mut timeline = WorldTimeline::new(world.spec.clone(), 12, 128);
    let poses = plan_tracking_camera(&mut timeline, body_idx, offset, &times, smoothing);

    let (w_f, h_f) = (
        f64::from(world.size.0.max(1)),
        f64::from(world.size.1.max(1)),
    );
    let mut xs = Vec::with_capacity(poses.len());
    let mut ys = Vec::with_capacity(poses.len());
    let mut zs = Vec::with_capacity(poses.len());
    let mut pans = Vec::with_capacity(poses.len());
    let mut tilts = Vec::with_capacity(poses.len());
    let mut rolls = Vec::with_capacity(poses.len());

    let mut prev_angles: Option<(f64, f64, f64)> = None;
    for (&frame, pose) in frames.iter().zip(&poses) {
        let pose_px = kerf_geom::lie::Se3::new(
            pose.rot,
            [
                pose.trans[0] * h_f,
                pose.trans[1] * h_f,
                pose.trans[2] * h_f,
            ],
        );
        let lens = pose_px.to_lens((w_f, h_f), fov);
        let (pan, tilt, roll) = match prev_angles {
            Some((pp, pt, pr)) => (
                unwrap_deg(pp, lens.pan),
                unwrap_deg(pt, lens.tilt),
                unwrap_deg(pr, lens.roll),
            ),
            None => (lens.pan, lens.tilt, lens.roll),
        };
        prev_angles = Some((pan, tilt, roll));
        xs.push((frame, lens.at[0]));
        ys.push((frame, lens.at[1]));
        zs.push((frame, lens.at[2]));
        pans.push((frame, pan));
        tilts.push((frame, tilt));
        rolls.push((frame, roll));
    }

    let cam_id = req.camera.as_deref().unwrap_or("cam");
    let has_cam = seq
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .any(|c| c.id.as_str() == cam_id);
    let set_line = format!(
        "set {cam_id} x={} y={} z={} pan={} tilt={} roll={} fov={}",
        format_keys(&xs),
        format_keys(&ys),
        format_keys(&zs),
        format_keys(&pans),
        format_keys(&tilts),
        format_keys(&rolls),
        round4(fov),
    );
    let script = if has_cam {
        set_line
    } else {
        let on = fresh_video_track(seq);
        format!(
            "track add {on} video\noverwrite gen=camera@1 at=0f dur={total_frames}f on={on} as={cam_id}\n{set_line}"
        )
    };

    Ok(json!({
        "sequence": seq.id.to_string(),
        "revision": revision,
        "follow": follow_id,
        "camera": cam_id,
        "frames": frames.len(),
        "smoothing": smoothing,
        "script": script,
    }))
}
