//! What Finish keeps of each open project: the check running or the last
//! one's report, the loudness measured, the captions written and why the
//! last render did not go, folded in from the bridge's events as they come,
//! so closing Finish loses nothing and a check goes on behind it.

use super::deliver::Captions;
use super::loudness::Loudness;
use super::qc::Report;
use crate::session::{BridgeEvent, Session};
use gpui::{prelude::*, App, Context, Entity, EntityId, Global, Subscription};
use kerf_bridge::event::{Check, Export, ExportStatus, LoudnessEvent};
use kerf_bridge::Event;
use std::collections::HashMap;

/// What Finish knows of a project: the check running or the last one's
/// report, the loudness measured, the captions written. One for each open
/// project, kept while its session lives, so Finish can close and open
/// again without losing a check.
pub(super) struct Finishing {
    /// A check's progress while it runs.
    pub(super) checking: Option<f64>,
    /// The revision the running check reads.
    pub(super) check_revision: Option<u64>,
    pub(super) report: Option<Report>,
    pub(super) check_failed: Option<String>,
    pub(super) measuring: Option<f64>,
    pub(super) loudness: Option<Loudness>,
    pub(super) loudness_failed: Option<String>,
    pub(super) captions: Option<Captions>,
    /// Why the last render could not be queued, or failed.
    pub(super) export_failed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// The [`Finishing`] of every open project, by its session.
#[derive(Default)]
struct Kept(HashMap<EntityId, Entity<Finishing>>);

impl Global for Kept {}

/// The project's [`Finishing`], made the first time it is asked for.
pub(super) fn finishing(session: &Entity<Session>, cx: &mut App) -> Entity<Finishing> {
    let id = session.entity_id();
    if let Some(kept) = cx.try_global::<Kept>().and_then(|kept| kept.0.get(&id)) {
        return kept.clone();
    }
    let made = cx.new(|cx| Finishing::new(session, cx));
    cx.default_global::<Kept>().0.insert(id, made.clone());
    made
}

impl Finishing {
    fn new(session: &Entity<Session>, cx: &mut Context<Self>) -> Self {
        let id = session.entity_id();
        let subscriptions = vec![
            cx.subscribe(session, |this, session, event: &BridgeEvent, cx| {
                let revision = session.read(cx).revision();
                if this.apply(&event.0, revision) {
                    cx.notify();
                }
            }),
            // Gone with its project.
            cx.observe_release(session, move |_, _, cx| {
                if cx.has_global::<Kept>() {
                    cx.update_global::<Kept, _>(|kept, _| {
                        kept.0.remove(&id);
                    });
                }
            }),
        ];
        Self {
            checking: None,
            check_revision: None,
            report: None,
            check_failed: None,
            measuring: None,
            loudness: None,
            loudness_failed: None,
            captions: None,
            export_failed: None,
            _subscriptions: subscriptions,
        }
    }

    /// Folds one bridge event in; true when it changed anything shown.
    fn apply(&mut self, event: &Event, revision: u64) -> bool {
        match event {
            Event::Check(check) => match check {
                Check::Running { progress, .. } => {
                    // A check another view started reads the project as it is now.
                    self.check_revision.get_or_insert(revision);
                    self.checking = Some(*progress);
                    self.check_failed = None;
                }
                Check::Done {
                    valid,
                    findings,
                    summary,
                    ..
                } => {
                    let revision = self.check_revision.take().unwrap_or(revision);
                    self.checking = None;
                    self.check_failed = None;
                    self.report = Some(Report::new(*valid, findings, summary, revision));
                }
                Check::Failed { message, .. } => {
                    self.checking = None;
                    self.check_revision = None;
                    self.check_failed = Some(message.clone());
                }
            },
            Event::Loudness(loudness) => match loudness {
                LoudnessEvent::Running { progress, .. } => self.measuring = Some(*progress),
                LoudnessEvent::Ready(ready) => {
                    self.measuring = None;
                    self.loudness_failed = None;
                    self.loudness = Some(Loudness::of(ready));
                }
                LoudnessEvent::Failed { message, .. } => {
                    self.measuring = None;
                    self.loudness_failed = Some(message.clone());
                }
            },
            Event::Export(Export::Captions(captions)) => {
                let format = captions.format.clone();
                let outcome = match captions.status {
                    ExportStatus::Completed => Ok(captions.path.clone().unwrap_or_default()),
                    ExportStatus::Failed => Err(captions
                        .message
                        .as_deref()
                        .unwrap_or("The captions could not be written")
                        .to_owned()),
                    _ => return false,
                };
                self.captions = Some(Captions { format, outcome });
            }
            Event::Export(Export::Render(render)) => match render.status {
                ExportStatus::Queued => self.export_failed = None,
                ExportStatus::Failed => {
                    self.export_failed = Some(
                        render
                            .message
                            .as_deref()
                            .unwrap_or("The render failed")
                            .to_owned(),
                    )
                }
                _ => return false,
            },
            Event::Error(error) if error.command.as_deref() == Some("export") => {
                self.export_failed = Some(error.message.clone());
            }
            _ => return false,
        }
        true
    }
}
