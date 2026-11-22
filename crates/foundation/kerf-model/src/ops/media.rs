//! Edits to a project's media: adding, relinking, replacing, and removing them, lining them up
//! with each other, and the steps that say how a media's file is made.

use super::{fail, Editor, Outcome};
use crate::id::Id;
use crate::making::Step;
use crate::model::{Content, Media, Synced};

/// So many steps, as an error says it.
fn counted(steps: usize) -> String {
    match steps {
        0 => "no steps".to_owned(),
        1 => "1 step".to_owned(),
        more => format!("{more} steps"),
    }
}

/// That a media has no step `index`, counted from 0, as an error says it.
fn no_step(steps: usize, index: usize) -> String {
    format!("it has {} and no step {}", counted(steps), index + 1)
}

impl Editor<'_> {
    pub(super) fn add_media(&mut self, media: &Media) -> Outcome {
        self.claim(&media.id)?;
        self.project.media.push(media.clone());
        Ok(())
    }

    pub(super) fn relink_media(&mut self, media: &Id, path: &String) -> Outcome {
        let found = self.project.media.iter_mut().find(|m| m.id == *media);
        let found = found.ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
        found.path.clone_from(path);
        Ok(())
    }

    pub(super) fn replace_media(&mut self, media: &Media) -> Outcome {
        let found = self.project.media.iter_mut().find(|m| m.id == media.id);
        let found = found.ok_or_else(|| fail("ref.missing", format!("no media `{}`", media.id)))?;
        // Which file it is changes; how it is made does not.
        let steps = std::mem::take(&mut found.steps);
        if media.recipe.is_some() && !steps.is_empty() {
            found.steps = steps;
            return Err(fail(
                "step.recipe",
                format!(
                    "media `{}` is made by the steps under its line, so it names no recipe file: leave `recipe=` out",
                    media.id
                ),
            ));
        }
        *found = Media {
            steps,
            ..media.clone()
        };
        Ok(())
    }

    pub(super) fn remove_media(&mut self, media: &Id) -> Outcome {
        let index = self
            .project
            .media
            .iter()
            .position(|m| m.id == *media)
            .ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
        let users: Vec<String> = self
            .project
            .sequences
            .iter()
            .flat_map(|s| &s.tracks)
            .flat_map(|t| &t.clips)
            .filter(|c| matches!(&c.content, Content::Media { media: m, .. } if m == media))
            .map(|c| c.id.to_string())
            .collect();
        if !users.is_empty() {
            return Err(fail(
                "media.in_use",
                format!("media `{media}` is used by {}", users.join(", ")),
            ));
        }
        self.project.media.remove(index);
        // What was lined up with it is lined up with nothing, as a child of a deleted clip is let
        // go.
        for other in &mut self.project.media {
            if other.sync.as_ref().is_some_and(|sync| sync.with == *media) {
                other.sync = None;
                self.notes.push(format!(
                    "media `{}` was lined up with `{media}`, and is now with nothing",
                    other.id
                ));
            }
        }
        Ok(())
    }

    pub(super) fn sync_media(&mut self, media: &Id, sync: Option<&Synced>) -> Outcome {
        let missing = |id: &Id| fail("ref.missing", format!("no media `{id}`"));
        if self.project.media(media.as_str()).is_none() {
            return Err(missing(media));
        }
        if let Some(sync) = sync {
            if self.project.media(sync.with.as_str()).is_none() {
                return Err(missing(&sync.with));
            }
            let ring = self.project.sync_comes_back(media, &sync.with);
            if ring {
                return Err(fail(
                    "sync.ring",
                    format!(
                        "media `{media}` cannot be lined up with `{}`, which is lined up with it",
                        sync.with
                    ),
                ));
            }
        }
        let found = self.project.media.iter_mut().find(|m| m.id == *media);
        if let Some(found) = found {
            found.sync = sync.cloned();
        }
        Ok(())
    }

    pub(super) fn add_step(&mut self, media: &Id, index: Option<usize>, step: &Step) -> Outcome {
        self.steps(media, |steps| {
            let at = index.unwrap_or(steps.len());
            if at > steps.len() {
                return Err(format!(
                    "it has {}, so a step added is step {} at most",
                    counted(steps.len()),
                    steps.len() + 1
                ));
            }
            steps.insert(at, step.clone());
            Ok(())
        })
    }

    pub(super) fn set_step(&mut self, media: &Id, index: usize, step: &Step) -> Outcome {
        self.steps(media, |steps| {
            let have = steps.len();
            *steps.get_mut(index).ok_or_else(|| no_step(have, index))? = step.clone();
            Ok(())
        })
    }

    pub(super) fn remove_step(&mut self, media: &Id, index: usize) -> Outcome {
        self.steps(media, |steps| {
            if index >= steps.len() {
                return Err(no_step(steps.len(), index));
            }
            steps.remove(index);
            Ok(())
        })
    }

    /// Change the steps of media `media` with `change`, and hold what it leaves to
    /// their order ([`crate::making::check`]).
    fn steps(
        &mut self,
        media: &Id,
        change: impl FnOnce(&mut Vec<Step>) -> Result<(), String>,
    ) -> Outcome {
        let found = self
            .project
            .media
            .iter_mut()
            .find(|m| m.id == *media)
            .ok_or_else(|| fail("ref.missing", format!("no media `{media}`")))?;
        if let Some(recipe) = &found.recipe {
            return Err(fail(
                "step.recipe",
                format!(
                    "media `{media}` names a recipe file, `{recipe}`, which holds its steps: `kerf carry {media}`, or an agent's `edit` with `carry`, writes them here; to write others in their place, take `recipe=` off its line with `media replace` first"
                ),
            ));
        }
        let mut steps = found.steps.clone();
        change(&mut steps)
            .and_then(|()| crate::making::check(&steps))
            .map_err(|why| fail("step.order", format!("media `{media}`: {why}")))?;
        found.steps = steps;
        Ok(())
    }
}
