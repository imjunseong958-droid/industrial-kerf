//! A sequence drawn at a moment: its tracks laid one over another, those of clips in depth put far
//! to near and seen and lit as its camera and lights have them, clips that cross cut where they
//! meet, its transitions mixing two clips and its dips fading a color through, each clip laid down
//! as the pass drawn has it, and its captions burned in over all.

use super::*;

/// What a sequence is being drawn at: when, how large, and, with clips in depth, how they are seen.
#[derive(Clone, Copy)]
pub(super) struct Moment<'a> {
    pub(super) at: Time,
    pub(super) scale: f64,
    pub(super) scene: Option<&'a three_d::Scene>,
    /// Whether a clip a model films (`filmed=true`) leaves its place clear: the sequence is
    /// drawn as a clip of another, where the take laid under that clip is to show.
    pub(super) clears: bool,
}

impl Drawing {
    /// Sequence `index` at its time `at`, at `scale` of its canvas,
    /// transparent where nothing is.
    pub(super) fn draw(&self, index: usize, at: Time, scale: f64) -> Image {
        self.draw_of(index, at, scale, None)
    }

    /// [`Drawing::draw`], of the one track `angle` when it names one.
    pub(super) fn draw_of(&self, index: usize, at: Time, scale: f64, angle: Option<&Id>) -> Image {
        let plan = &self.sequences[index];
        let (w, h) = output_size(plan.width, plan.height, scale);
        let mut canvas = Image::new(w, h);
        let tracks = self.in_order(plan, at, scale, angle);
        let in_depth = tracks.iter().any(|track| {
            track
                .shown
                .iter()
                .any(|shown| shown.shows(at) && shown.plan.three_d)
        });
        let scene = in_depth.then(|| self.scene(plan, &tracks, at, scale, (w, h)));
        let moment = Moment {
            at,
            scale,
            scene: scene.as_ref(),
            // The program's sequence is the first, and every other is a clip of another.
            clears: index != 0,
        };
        let mut next = 0;
        while next < tracks.len() {
            let crossing = self.crossing_run(plan, &tracks[next..], &moment);
            if crossing.is_empty() {
                self.track(&mut canvas, plan, tracks[next], &moment);
                next += 1;
            } else {
                next += crossing.len();
                self.crossing(&mut canvas, plan, &crossing, &moment);
            }
        }
        if self.captions && self.pass == Pass::Picture {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                let text_size = caption_size(plan, scale);
                let text = caption_text(caption, text_size);
                text::draw(&mut canvas, &text, [w as f32 / 2.0, h as f32 * 0.95], 1.0);
            }
        }
        canvas
    }

    /// The tracks to draw, with adjacent ones showing clips in depth put far to near. Adjacent
    /// among those that put something on the canvas at `at`: a track of a camera, a light, a
    /// null, or a signal, and one with no clip then, is left out, so that the clips in depth
    /// under it and over it are one run, sorted and cut where they cross as if it were not
    /// there. A camera sees the whole sequence from whichever track it is on, and where it is
    /// put should not change what is before what.
    pub(super) fn in_order<'p>(
        &self,
        plan: &'p SequencePlan,
        at: Time,
        scale: f64,
        angle: Option<&Id>,
    ) -> Vec<&'p TrackPlan> {
        let mut tracks: Vec<&TrackPlan> = plan
            .tracks
            .iter()
            .filter(|track| track.drawn(angle) && !track.idle(at))
            .collect();
        fn in_depth(track: &TrackPlan, at: Time) -> Option<&ClipPlan> {
            track
                .shown
                .iter()
                .find(|shown| shown.shows(at) && shown.plan.three_d)
                .map(|shown| &shown.plan)
        }
        if !tracks.iter().any(|track| in_depth(track, at).is_some()) {
            return tracks;
        }
        let size = output_size(plan.width, plan.height, scale);
        let scene = three_d::Scene::at(plan, at, size);
        let depth = |track: &TrackPlan| {
            in_depth(track, at).map(|clip| {
                let place = Affine::scale(scale, scale).then_apply(&self.geometry(
                    plan,
                    clip,
                    at - clip.clip.start,
                ));
                scene.depth(clip, &place, at - clip.clip.start)
            })
        };
        let mut start = 0;
        while start < tracks.len() {
            if depth(tracks[start]).is_none() {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < tracks.len() && depth(tracks[end]).is_some() {
                end += 1;
            }
            // Stable, so that clips at one depth stack as their tracks do.
            tracks[start..end].sort_by(|a, b| {
                let (a, b) = (depth(a).unwrap_or(0.0), depth(b).unwrap_or(0.0));
                b.total_cmp(&a)
            });
            start = end;
        }
        tracks
    }

    /// How the clips in depth of `tracks` are seen and lit at `at`, with their pictures when a light
    /// casts shadows, and only then, since it takes every one of them at once.
    pub(super) fn scene(
        &self,
        plan: &SequencePlan,
        tracks: &[&TrackPlan],
        at: Time,
        scale: f64,
        size: (usize, usize),
    ) -> three_d::Scene {
        let mut scene = three_d::Scene::at(plan, at, size);
        if self.pass != Pass::Picture {
            return scene.bare();
        }
        if scene.casts_shadows() {
            let casters = self.casters(plan, tracks, at, scale, size, &scene);
            scene.casters = casters;
            // A mesh stands in a light's way by its triangles.
            let standing = self.standing(plan, tracks, at, scale, &scene);
            scene.standing = three_d::mesh::InTheWay::of(standing);
        }
        for wanted in scene.environments() {
            let picture = plan
                .tracks
                .iter()
                .find(|track| track.id.as_str() == wanted)
                .and_then(|track| track.shown.iter().find(|shown| shown.shows(at)))
                .and_then(|shown| self.ready(plan, shown, at, scale, size, None));
            if let Some(ready) = picture {
                scene.light_from(&wanted, &ready.picture);
            }
        }
        scene
    }

    /// The clips in depth of `tracks` that show at `at` with a picture of their own, as the lights
    /// of `scene` cast their shadows: each through its depth, on a canvas of `size`.
    fn casters(
        &self,
        plan: &SequencePlan,
        tracks: &[&TrackPlan],
        at: Time,
        scale: f64,
        size: (usize, usize),
        scene: &three_d::Scene,
    ) -> Vec<three_d::Caster> {
        tracks
            .iter()
            .flat_map(|track| &track.shown)
            .filter(|shown| {
                shown.shows(at)
                    && shown.plan.three_d
                    && !matches!(
                        shown.plan.source,
                        Source::Nothing | Source::Adjustment | Source::Mesh
                    )
            })
            .filter_map(|shown| {
                let ready = self.ready(plan, shown, at, scale, size, Some(scene))?;
                let time = at - shown.plan.clip.start;
                let caster = three_d::Caster::new(
                    scene,
                    &shown.plan,
                    ready.picture,
                    (&ready.to_output, ready.crop),
                    ready.opacity,
                    time,
                );
                let height = size.1 as f64;
                let depth = three_d::depth_of(&shown.plan, time, height);
                let bevel = three_d::bevel_of(&shown.plan, time, height);
                let round = three_d::bevel_is_round(&shown.plan);
                Some(caster.through_its_depth(depth, (bevel, round), scene.is_wide()))
            })
            .flatten()
            .collect()
    }

    /// The meshes of `tracks` that show at `at`, as they stand in the way of the lights of `scene`.
    fn standing(
        &self,
        plan: &SequencePlan,
        tracks: &[&TrackPlan],
        at: Time,
        scale: f64,
        scene: &three_d::Scene,
    ) -> Vec<three_d::mesh::Standing> {
        tracks
            .iter()
            .flat_map(|track| &track.shown)
            .filter(|shown| shown.shows(at) && matches!(shown.plan.source, Source::Mesh))
            .filter_map(|shown| {
                let time = at - shown.plan.clip.start;
                let opacity = number(&param(&shown.plan.clip, "opacity").value_at(time)) as f32;
                (opacity > 0.0).then(|| {
                    let to_output = Affine::scale(scale, scale).then_apply(&self.geometry(
                        plan,
                        &shown.plan,
                        time,
                    ));
                    three_d::mesh::Standing::of(scene, &shown.plan, &to_output, time, opacity)
                })
            })
            .collect()
    }

    /// The clips of the run of tracks in depth at the start of `tracks`, far to near as they stand, if
    /// any two of them cross; else none, and they draw one by one as ever.
    pub(super) fn crossing_run<'p>(
        &self,
        plan: &'p SequencePlan,
        tracks: &[&'p TrackPlan],
        moment: &Moment,
    ) -> Vec<&'p Shown> {
        let Moment {
            at, scale, scene, ..
        } = *moment;
        let Some(scene) = scene else {
            return Vec::new();
        };
        let run: Vec<&Shown> = tracks
            .iter()
            .map_while(|track| track.lone_in_depth(at))
            .collect();
        if run.len() < 2 {
            return Vec::new();
        }
        let planes: Vec<(three_d::Plane, Rect)> = run
            .iter()
            .map(|shown| self.plane_in_run(plan, &shown.plan, at, scale, scene))
            .collect();
        // Two that cross are cut where they meet. Two that do not are one way round all over, but
        // not always the way their middles have them: then they too are drawn by the pixel, so
        // that a floor is not drawn over what stands on it for having its middle the nearer.
        // A mesh has no one plane to be before or behind, so what stands with one is drawn by
        // the pixel too.
        let meshes = run
            .iter()
            .any(|shown| matches!(shown.plan.source, Source::Mesh));
        let crosses = meshes
            || planes.iter().enumerate().any(|(index, (plane, crop))| {
                planes[index + 1..].iter().any(|(other, other_crop)| {
                    plane.crosses(*crop, other, *other_crop)
                        || !plane.surely_behind(*crop, other, *other_crop, scene)
                })
            });
        if crosses {
            run
        } else {
            Vec::new()
        }
    }

    /// The plane a clip of a run in depth is on at `at`, as `scene` sees it, and what of its picture
    /// is on it, in its own pixels: as far as its picture reaches, for a generator that reaches past
    /// its frame, and what its crop leaves.
    fn plane_in_run(
        &self,
        plan: &SequencePlan,
        clip: &ClipPlan,
        at: Time,
        scale: f64,
        scene: &three_d::Scene,
    ) -> (three_d::Plane, Rect) {
        let time = at - clip.clip.start;
        let place = Affine::scale(scale, scale).then_apply(&self.geometry(plan, clip, time));
        let (w, h) = (f64::from(clip.size.0), f64::from(clip.size.1));
        let frame = Rect::new(0.0, 0.0, w, h);
        // As far as its picture reaches, for a generator that reaches past its frame.
        let crop = match self.reach(plan, clip, time, scale, Some(scene)) {
            Some(Reach { beyond: (x, y), .. }) => {
                let whole = Rect::new(-x, -y, w + 2.0 * x, h + 2.0 * y);
                crop_past(clip, time, frame, whole).unwrap_or(whole)
            }
            None => {
                let size = (clip.size.0 as usize, clip.size.1 as usize);
                crop_rect(clip, time, (w, h), 1.0, size).unwrap_or(frame)
            }
        };
        (three_d::Plane::of(scene, clip, (w, h), &place, time), crop)
    }

    /// Clips in depth that cross, each cut where it meets another, over `canvas`.
    pub(super) fn crossing(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        run: &[&Shown],
        moment: &Moment,
    ) {
        if moment.scene.is_none() {
            return;
        }
        let size = (canvas.w, canvas.h);
        let layers: Vec<three_d::Layer> = run
            .iter()
            .filter_map(|shown| {
                let mut layer = self.in_run(sequence, shown, moment, size)?;
                if self.hard {
                    // Whole or not at all, and so too where two cross.
                    harden(
                        &mut layer.picture,
                        layer.opacity,
                        &[self.id_color(&shown.plan.clip.id)],
                    );
                    layer.opacity = 1.0;
                }
                layer.clears = moment.clears && shown.plan.filmed;
                Some(layer)
            })
            .collect();
        three_d::composite_by_depth(canvas, &layers, self.hard);
    }

    /// A clip of a run that crosses, projected, and how far ahead it is at each pixel.
    pub(super) fn in_run(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        size: (usize, usize),
    ) -> Option<three_d::Layer> {
        let Moment {
            at, scale, scene, ..
        } = *moment;
        let scene = scene?;
        if matches!(shown.plan.source, Source::Mesh) {
            return self.mesh(sequence, shown, moment, size);
        }
        let ready = self.ready(sequence, shown, at, scale, size, Some(scene))?;
        let (picture, depth) = three_d::project_with_depth(
            scene,
            &shown.plan,
            &ready.picture,
            (&ready.to_output, ready.crop),
            at - shown.plan.clip.start,
            size,
        );
        Some(three_d::Layer {
            picture,
            depth,
            opacity: ready.opacity,
            blend: self.blend(shown.plan.blend),
            clears: false,
        })
    }

    /// A track of `sequence` at `moment` over `canvas`.
    pub(super) fn track(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        track: &TrackPlan,
        moment: &Moment,
    ) {
        let at = moment.at;
        for (index, shown) in track.shown.iter().enumerate() {
            if !shown.shows(at) {
                continue;
            }
            match track.pair_at(index, at) {
                // Drawn once, at the first of its clips that shows.
                Some(pair) if index == pair.from || !track.shown[pair.from].shows(at) => {
                    self.pair(canvas, sequence, track, pair, moment);
                }
                Some(_) => {}
                None => self.layer(canvas, sequence, shown, moment, shown.plan.blend),
            }
        }
        if self.pass != Pass::Picture {
            return;
        }
        for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
            let opacity = f64::from(dip.color.a) / 255.0
                * (1.0 - (2.0 * progress(at, dip.from, dip.to) - 1.0).abs());
            let solid = Image::filled(
                canvas.w,
                canvas.h,
                premultiplied(Rgba {
                    a: 255,
                    ..dip.color
                }),
            );
            composite(canvas, &solid, opacity as f32, Blend::Normal);
        }
    }

    /// Two clips mixed by a FilmCraft transition at sequence time `at`,
    /// over `canvas`.
    pub(super) fn pair(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        track: &TrackPlan,
        pair: &Pair,
        moment: &Moment,
    ) {
        let Moment { at, scale, .. } = *moment;
        let (w, h) = (canvas.w, canvas.h);
        let (from, to) = (&track.shown[pair.from], &track.shown[pair.to]);
        // Each side is its clip's own picture, which is what a transition mixes: what the two
        // leave clear is said of the mix.
        let own = Moment {
            clears: false,
            ..*moment
        };
        let side = |shown: &Shown| {
            let mut picture = Image::new(w, h);
            if shown.shows(at) {
                self.layer(&mut picture, sequence, shown, &own, Blend::Normal);
            }
            picture
        };
        let (a, b) = match (pair.from == pair.to, pair.from_nothing) {
            (false, _) => (side(from), side(to)),
            // One clip alone: it enters from nothing, or leaves into it.
            (true, true) => (Image::new(w, h), side(to)),
            (true, false) => (side(from), Image::new(w, h)),
        };
        let Some(fx) = Fx::new(&pair.def, &pair.params) else {
            return;
        };
        let p = progress(at, pair.start, pair.end) as f32;
        let mix = |a: &Image, b: &Image| apply_scaled(&fx, a, b, p, scale as f32);
        let lay = |canvas: &mut Image, mut mixed: Image| {
            if self.hard {
                // Each pixel is the one of the two clips it is the more of.
                let colors = [&from.plan.clip.id, &to.plan.clip.id].map(|id| self.id_color(id));
                lay_hard(canvas, &mut mixed, &colors);
                return;
            }
            // A fade from or to nothing keeps the one clip's blend mode;
            // between two they mix normally.
            let blend = match (from.shows(at), to.shows(at)) {
                (true, false) => from.plan.blend,
                (false, true) => to.plan.blend,
                _ if pair.from == pair.to => from.plan.blend,
                _ => Blend::Normal,
            };
            composite(canvas, &mixed, 1.0, self.blend(blend));
        };
        // Which side is a clip a model films, where such clips leave their place clear. One
        // clip that enters from nothing or leaves into it is all there is of its pair.
        let clears = |shown: &Shown| moment.clears && shown.shows(at) && shown.plan.filmed;
        let (a_clears, b_clears) = match pair.from == pair.to {
            true => (clears(from), clears(from)),
            false => (clears(from), clears(to)),
        };
        match (a_clears, b_clears) {
            (false, false) => lay(canvas, mix(&a, &b)),
            (true, true) => clear_under(canvas, &mut mix(&a, &b), self.hard),
            // One side is a model's and the other Kerf's. The mix is one that takes the model's
            // picture as a share of it, as the take shows in its place, and so a plain
            // dissolve where the transition goes by what that picture shows.
            (a_filmed, _) => {
                let filmed = match a_filmed {
                    true => Side::Outgoing,
                    false => Side::Incoming,
                };
                let scale = scale as f32;
                if self.hard {
                    // Each pixel of the mix is the one of the two it is the more of: left
                    // clear, or laid down.
                    let colors = [&from.plan.clip.id, &to.plan.clip.id].map(|id| self.id_color(id));
                    let models = usize::from(!a_filmed);
                    let mut mixed = apply_filmed(&fx, &a, &b, p, scale, filmed);
                    lay_hard_or_clear(canvas, &mut mixed, &colors, models);
                } else {
                    // What the mix covers is left clear, and Kerf's part of it laid there,
                    // whichever of the two lies over the other in it and however it blends
                    // them.
                    split_filmed(&fx, &a, &b, p, scale, filmed).lay(canvas);
                }
            }
        }
    }

    /// Draw a shown clip over `canvas` at `moment` with `blend`. Drawn hard, it is drawn by itself
    /// and laid down whole where it covers half a pixel or more. One a model films, where
    /// `moment` has such clips leave their place clear, is drawn by itself too, and what it
    /// covers of `canvas` is made clear by as much.
    pub(super) fn layer(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        blend: Blend,
    ) {
        // An adjustment has no picture to leave clear, whatever it says.
        let clears =
            moment.clears && shown.plan.filmed && !matches!(shown.plan.source, Source::Adjustment);
        if !self.hard && !clears {
            self.lay(canvas, sequence, shown, moment, blend);
            return;
        }
        let mut alone = self.clear_picture(canvas.w, canvas.h);
        self.lay(&mut alone, sequence, shown, moment, Blend::Normal);
        if clears {
            clear_under(canvas, &mut alone, self.hard);
        } else {
            lay_hard(canvas, &mut alone, &[self.id_color(&shown.plan.clip.id)]);
        }
        self.keep_clear(alone);
    }

    /// [`Self::layer`], with the clip covering as much of each pixel as it does.
    pub(super) fn lay(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        blend: Blend,
    ) {
        let Moment {
            at, scale, scene, ..
        } = *moment;
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        if matches!(plan.source, Source::Nothing) {
            return;
        }
        if matches!(plan.source, Source::Adjustment) {
            if self.pass == Pass::Picture {
                self.adjust(canvas, sequence, plan, time, scale);
            }
            return;
        }
        let blend = self.blend(blend);
        let (w, h) = (canvas.w, canvas.h);
        if matches!(plan.source, Source::Mesh) {
            if let Some(mesh) = self.mesh(sequence, shown, moment, (w, h)) {
                composite(canvas, &mesh.picture, mesh.opacity, blend);
            }
            return;
        }
        let Some(Ready {
            picture,
            to_output,
            crop,
            opacity,
        }) = self.ready(sequence, shown, at, scale, (w, h), scene)
        else {
            return;
        };
        if let (true, Some(scene)) = (plan.three_d, scene) {
            let projected =
                three_d::project(scene, plan, &picture, (&to_output, crop), time, (w, h));
            composite(canvas, &projected, opacity, blend);
            return;
        }
        match crop {
            None if (picture.w, picture.h) == (w, h) && identity(&to_output) => {
                composite(canvas, &picture, opacity, blend);
            }
            crop => {
                let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
                let crop = crop.unwrap_or(whole);
                composite_placed(canvas, &picture, &to_output, crop, opacity, blend);
            }
        }
    }

    /// A shown clip that is a mesh, as the scene of `moment` sees it on a canvas of `size`: its
    /// picture, lit, or as the pass drawn has it, and how far ahead it is at each pixel.
    pub(super) fn mesh(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        size: (usize, usize),
    ) -> Option<three_d::Layer> {
        let Moment {
            at, scale, scene, ..
        } = *moment;
        let scene = scene?;
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        // Its picture is the one the camera sees of it, on the canvas: that is what a wipe
        // crosses, a mask cuts, and an effect is done to.
        let reveal = shown
            .reveal
            .map(|(reveal, start, end)| (reveal, progress(at, start, end)));
        let fade = match reveal {
            Some((Reveal::Dissolve, progress)) => progress,
            _ => 1.0,
        };
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        if opacity <= 0.0 {
            return None;
        }
        let shade = self.mesh_shade(&clip.id);
        let to_output =
            Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        let (mut picture, depth) = three_d::mesh::draw(scene, plan, &to_output, time, size, shade);
        if let Some((Reveal::Wipe { angle, softness }, progress)) = reveal {
            let covered = wipe::coverage(picture.w, picture.h, angle, softness, progress);
            picture.mul_coverage(&covered);
        }
        let frame = (f64::from(plan.size.0), f64::from(plan.size.1));
        let masks = mask::prepare(&clip.masks, frame, scale, time);
        if let Some(covered) = mask::coverage(&masks, picture.w, picture.h) {
            picture.mul_coverage(&covered);
        }
        // A pass says which clip a pixel is of and how near it is, which no effect changes.
        if self.pass == Pass::Picture && !plan.effects.is_empty() {
            let placed = Placed {
                sequence,
                at,
                scale,
                to_output: Affine::IDENTITY,
                per_source: scale,
                env: false,
                reach: None,
            };
            self.effects(&mut picture, plan, time, &placed, plan.effects.len());
        }
        Some(three_d::Layer {
            picture,
            depth,
            opacity,
            blend: self.blend(plan.blend),
            clears: false,
        })
    }

    /// What the mesh clip `id` is drawn as in the pass drawn: lit, in its color, by how near it
    /// is, or by the way it faces.
    fn mesh_shade<'c>(&self, id: &'c Id) -> three_d::mesh::Shade<'c> {
        match self.pass {
            Pass::Picture => three_d::mesh::Shade::Lit(id),
            Pass::Ids => {
                let [r, g, b, _] = self.id_color(id);
                three_d::mesh::Shade::Flat([r, g, b])
            }
            Pass::Depth | Pass::Relative => three_d::mesh::Shade::Near,
            Pass::Normals => three_d::mesh::Shade::Facing,
        }
    }
}
