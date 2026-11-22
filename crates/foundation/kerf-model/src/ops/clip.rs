//! Edits to what a clip carries: its properties, effects, and masks, and the transitions on the
//! cut after it.

use super::{fail, ClipChange, Editor, Outcome};
use crate::catalog::FxKind;
use crate::diag::Diagnostic;
use crate::id::Id;
use crate::model::{Align, ClipAt, Content, Effect, Mask, MaskMode, MaskShape, Params, Transition};
use crate::time::Time;
use crate::value::Param;

impl Editor<'_> {
    pub(super) fn set(&mut self, clip: &Id, changes: &[ClipChange]) -> Outcome {
        let at = self.editable(clip)?;
        let domain = self.project.sequences[at.sequence].tracks[at.track]
            .kind
            .domain();
        for change in changes {
            match change {
                ClipChange::Name(name) => self.project.clip_at_mut(at).name.clone_from(name),
                ClipChange::Enabled(enabled) => self.project.clip_at_mut(at).enabled = *enabled,
                ClipChange::Parent(parent) => {
                    if let Some(parent) = parent {
                        self.check_parent(at, clip, parent)?;
                    }
                    self.project.clip_at_mut(at).parent.clone_from(parent);
                }
                ClipChange::Retime(retime) => {
                    let target = self.project.clip_at_mut(at);
                    if !matches!(target.content, Content::Media { .. }) {
                        return Err(fail(
                            "clip.domain",
                            format!("`{clip}` is not footage, so it has no frames to retime"),
                        ));
                    }
                    target.retime = *retime;
                }
                ClipChange::Angle(angle) => {
                    let target = self.project.clip_at_mut(at);
                    if !matches!(target.content, Content::Sequence { .. }) {
                        return Err(fail(
                            "clip.domain",
                            format!("`{clip}` holds no sequence to show one track of"),
                        ));
                    }
                    target.angle.clone_from(angle);
                }
                ClipChange::Caption(text) => match &mut self.project.clip_at_mut(at).content {
                    Content::Caption { text: current } => current.clone_from(text),
                    _ => return Err(fail("clip.domain", format!("`{clip}` is not a caption"))),
                },
                ClipChange::Param(name, value) => {
                    let domain = domain
                        .ok_or_else(|| fail("clip.domain", "captions carry no properties"))?;
                    if let Some(def) = self.catalog.prop(domain, name) {
                        if let Some(value) = value {
                            def.check(value).map_err(|m| fail("param.invalid", m))?;
                        }
                        let props = &mut self.project.clip_at_mut(at).props;
                        match value {
                            Some(value) => props.insert(name.clone(), value.clone()),
                            None => props.remove(name),
                        };
                        continue;
                    }
                    let generator = match &self.clip(at).content {
                        Content::Generator { fx, .. } => self.catalog.fx(fx.as_str()),
                        _ => None,
                    };
                    let def = generator.and_then(|g| g.find(name)).ok_or_else(|| {
                        let mut known: Vec<&str> = self
                            .catalog
                            .props(domain)
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect();
                        known.extend(
                            generator
                                .into_iter()
                                .flat_map(|g| g.params.iter().map(|p| p.name.as_str())),
                        );
                        fail(
                            "param.unknown",
                            format!(
                                "`{clip}` has no property `{name}` (has: {})",
                                known.join(", ")
                            ),
                        )
                    })?;
                    if let Some(value) = value {
                        def.check(value).map_err(|m| fail("param.invalid", m))?;
                    }
                    if let Content::Generator { params, .. } =
                        &mut self.project.clip_at_mut(at).content
                    {
                        match value {
                            Some(value) => params.insert(name.clone(), value.clone()),
                            None => params.remove(name),
                        };
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether `clip`, at `at`, can move with `parent`: a clip of its sequence that does not
    /// already move with it.
    fn check_parent(&self, at: ClipAt, clip: &Id, parent: &Id) -> Result<(), Diagnostic> {
        let sequence = &self.project.sequences[at.sequence];
        let find = |id: &Id| {
            sequence
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|clip| clip.id == *id)
        };
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if id == clip {
                return Err(fail(
                    "clip.parent",
                    format!("`{clip}` cannot move with `{parent}`, which moves with it"),
                ));
            }
            let found = find(id).ok_or_else(|| {
                fail(
                    "clip.parent",
                    format!("`{id}` is not a clip of the sequence `{clip}` is in"),
                )
            })?;
            ancestor = found.parent.as_ref();
        }
        Ok(())
    }

    pub(super) fn fx_add(&mut self, clip: &Id, effect: &Effect, index: Option<usize>) -> Outcome {
        let at = self.editable(clip)?;
        let domain = self.domain(at)?;
        self.check_fx(
            &effect.fx,
            effect.version,
            FxKind::Effect,
            domain,
            &effect.params,
        )?;
        let effects = &mut self.project.clip_at_mut(at).effects;
        let index = index.unwrap_or(effects.len()).min(effects.len());
        effects.insert(index, effect.clone());
        Ok(())
    }

    pub(super) fn fx_set(
        &mut self,
        clip: &Id,
        index: usize,
        enabled: Option<bool>,
        params: &[(String, Option<Param>)],
    ) -> Outcome {
        let at = self.editable(clip)?;
        let domain = self.domain(at)?;
        let effect = self.effect(at, index)?.clone();
        let mut updated = effect.params.clone();
        for (name, value) in params {
            match value {
                Some(value) => updated.insert(name.clone(), value.clone()),
                None => updated.remove(name),
            };
        }
        self.check_fx(&effect.fx, effect.version, FxKind::Effect, domain, &updated)?;
        let target = &mut self.project.clip_at_mut(at).effects[index];
        target.params = updated;
        target.enabled = enabled.unwrap_or(target.enabled);
        Ok(())
    }

    pub(super) fn fx_remove(&mut self, clip: &Id, index: usize) -> Outcome {
        let at = self.editable(clip)?;
        self.effect(at, index)?;
        self.project.clip_at_mut(at).effects.remove(index);
        Ok(())
    }

    pub(super) fn fx_flatten(&mut self, clip: &Id, index: usize) -> Outcome {
        let at = self.editable(clip)?;
        self.effect(at, index)?;
        let sequence = &self.project.sequences[at.sequence];
        let (rate, markers) = (sequence.rate, sequence.markers.clone());
        let catalog = self.catalog;
        let clip = self.project.clip_at_mut(at);
        crate::look::flatten_into(clip, index, rate, &markers, catalog)
            .map_err(|message| fail("look.flatten", message))
    }

    pub(super) fn fx_move(&mut self, clip: &Id, index: usize, to: usize) -> Outcome {
        let at = self.editable(clip)?;
        self.effect(at, index)?;
        let effects = &mut self.project.clip_at_mut(at).effects;
        let effect = effects.remove(index);
        let to = to.min(effects.len());
        effects.insert(to, effect);
        Ok(())
    }

    // The outline, mode, and properties are checked with the rest of
    // the project when the edit is done.
    pub(super) fn mask_add(&mut self, clip: &Id, mask: &Mask, index: Option<usize>) -> Outcome {
        let at = self.editable(clip)?;
        let masks = &mut self.project.clip_at_mut(at).masks;
        if masks.iter().any(|other| other.name == mask.name) {
            return Err(fail(
                "mask.duplicate",
                format!("clip `{clip}` already has a mask `{}`", mask.name),
            ));
        }
        let index = index.unwrap_or(masks.len()).min(masks.len());
        masks.insert(index, mask.clone());
        Ok(())
    }

    pub(super) fn mask_set(
        &mut self,
        clip: &Id,
        name: &Id,
        shape: Option<&MaskShape>,
        mode: Option<MaskMode>,
        inverted: Option<bool>,
        params: &[(String, Option<Param>)],
    ) -> Outcome {
        let at = self.editable(clip)?;
        let index = self.mask(at, name)?;
        let mask = &mut self.project.clip_at_mut(at).masks[index];
        if let Some(shape) = shape {
            mask.shape = shape.clone();
        }
        mask.mode = mode.unwrap_or(mask.mode);
        mask.inverted = inverted.unwrap_or(mask.inverted);
        for (name, value) in params {
            match value {
                Some(value) => mask.params.insert(name.clone(), value.clone()),
                None => mask.params.remove(name),
            };
        }
        Ok(())
    }

    pub(super) fn mask_remove(&mut self, clip: &Id, name: &Id) -> Outcome {
        let at = self.editable(clip)?;
        let index = self.mask(at, name)?;
        self.project.clip_at_mut(at).masks.remove(index);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_transition(
        &mut self,
        id: Option<&Id>,
        from: &Id,
        to: &Id,
        (fx, version): (&Id, u32),
        duration: Time,
        align: Align,
        params: &Params,
    ) -> Outcome {
        let a = self.locate(from)?;
        let b = self.locate(to)?;
        let s = a.sequence;
        if a.sequence != b.sequence
            || a.track != b.track
            || b.clip != a.clip + 1
            || self.clip(a).end() != self.clip(b).start
        {
            return Err(fail(
                "transition.not_adjacent",
                format!("`{to}` must start exactly where `{from}` ends on the same track"),
            ));
        }
        self.unlocked(s, a.track)?;
        self.aligned(s, duration, "transition duration")?;
        let track = &self.project.sequences[s].tracks[a.track];
        if track
            .transitions
            .iter()
            .any(|t| t.from == *from && t.to == *to)
        {
            return Err(fail(
                "transition.duplicate",
                format!("the cut between `{from}` and `{to}` already has a transition"),
            ));
        }
        let domain = track
            .kind
            .domain()
            .ok_or_else(|| fail("transition.domain", "subtitle tracks have no transitions"))?;
        self.check_fx(fx, version, FxKind::Transition, domain, params)?;
        let id = match id {
            Some(id) => {
                self.claim(id)?;
                id.clone()
            }
            None => self.ids.next("t"),
        };
        self.project.sequences[s].tracks[a.track]
            .transitions
            .push(Transition {
                id,
                from: from.clone(),
                to: to.clone(),
                fx: fx.clone(),
                version,
                duration,
                align,
                params: params.clone(),
            });
        Ok(())
    }

    pub(super) fn set_transition(
        &mut self,
        id: &Id,
        duration: Option<Time>,
        align: Option<Align>,
        params: &[(String, Option<Param>)],
    ) -> Outcome {
        let (s, t, index) = self.transition(id)?;
        self.unlocked(s, t)?;
        let mut transition = self.project.sequences[s].tracks[t].transitions[index].clone();
        if let Some(duration) = duration {
            self.aligned(s, duration, "transition duration")?;
            transition.duration = duration;
        }
        transition.align = align.unwrap_or(transition.align);
        for (name, value) in params {
            match value {
                Some(value) => transition.params.insert(name.clone(), value.clone()),
                None => transition.params.remove(name),
            };
        }
        let domain = self.project.sequences[s].tracks[t].kind.domain();
        let domain = domain
            .ok_or_else(|| fail("transition.domain", "subtitle tracks have no transitions"))?;
        self.check_fx(
            &transition.fx,
            transition.version,
            FxKind::Transition,
            domain,
            &transition.params,
        )?;
        self.project.sequences[s].tracks[t].transitions[index] = transition;
        Ok(())
    }

    pub(super) fn remove_transition(&mut self, id: &Id) -> Outcome {
        let (s, t, index) = self.transition(id)?;
        self.unlocked(s, t)?;
        self.project.sequences[s].tracks[t]
            .transitions
            .remove(index);
        Ok(())
    }
}
