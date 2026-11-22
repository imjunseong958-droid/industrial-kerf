//! The project the window has open: the bridge to kerfd, the state its
//! events leave, and the picture playback last drew. One entity, observed
//! by every region of the window, so what kerfd says shows everywhere at
//! once; what playback moves many times a second is told only to the
//! regions that draw it ([`Playhead`]). Without a project (the launcher)
//! there is no bridge.

use crate::model::{ClipShow, SourceShow, Timeline, TimelineShow};
use crate::prefs::{Prefs, Provider};
use crate::store::{Change, EditorState};
use crate::theme::motion;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, RenderImage, Task, Window};
use kerf_bridge::event::{AgentUpdate, Export, ExportStatus, PreviewStatus};
use kerf_bridge::{AgentSettings, ChatProvider, Command, ConversationsRequest, Event};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::{self, Future};
use std::path::{Path, PathBuf};
use std::pin::{pin, Pin};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Instant;

/// How long Kerf's edits to a first cut are quiet before the cut renders
/// by itself (Qt's `cutSettle`); on a change reel's Before side, how long
/// the seek has to land on the program just reopened before it plays, and
/// how long the reel waits to hear that playback has reopened the program
/// before it takes up the segment anyway.
pub use crate::theme::motion::{CUT_SETTLE, REEL_PROGRAM_WAIT, REEL_SEEK_LANDS};

/// What Reconnect says when the bridge would not take it.
pub const RECONNECT_REFUSED: &str = "Reconnect could not be queued. Try again.";

/// How long the pointer rests before the frame under it is asked for
/// exactly: the rough seeks while it moves show the nearest picture kerfd
/// can give at once.
pub use crate::theme::motion::SCRUB_SETTLE;

/// What the system's keychain answered for each key, for every window of
/// this editor: asked once, written when a key is saved in any window, and
/// asked again when Settings opens (a key changed outside Kerf).
static KEYCHAIN: Mutex<BTreeMap<&'static str, Option<String>>> = Mutex::new(BTreeMap::new());

fn keychain() -> MutexGuard<'static, BTreeMap<&'static str, Option<String>>> {
    KEYCHAIN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a key was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKept {
    Keychain,
    /// The keychain could not be reached: in the editor's settings file.
    SettingsFile,
}

/// The newest picture playback drew, ready to draw.
pub struct Picture {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
    pub time: f64,
    /// The last frame of the playback before, standing in since playback
    /// was opened again until the new one's first frame comes (Qt's
    /// `m_hasShownFrame` false with a frame still up).
    pub standing_in: bool,
}

/// Whether the frame shown (`shown`, its pixels) stands in after playback
/// is opened again (`viewer_reset`) on a program whose canvas is `canvas`
/// and lasts `duration` seconds: only when it has the canvas's shape (it
/// would not be stretched) and there is something to play, so a first
/// frame will come to take its place (Qt's `sameShape` and duration).
pub fn stands_in(shown: (u32, u32), canvas: (u32, u32), duration: f64) -> bool {
    let shape = |(width, height): (u32, u32)| {
        (width > 0 && height > 0).then(|| f64::from(width) / f64::from(height))
    };
    let same_shape = match (shape(shown), shape(canvas)) {
        (Some(shown), Some(canvas)) => (shown - canvas).abs() < 0.01,
        _ => false,
    };
    same_shape && duration > 0.0
}

/// What playback moves many times a second: the playhead (`state.position`,
/// with the length and frames dropped its clocks say), the picture and its
/// caption. Notified, rather than the whole session, when only these change,
/// so only the views that draw them (the stage, the timeline, the
/// conversation and the inspector) draw again.
pub struct Playhead;

pub struct Session {
    pub project: Option<PathBuf>,
    bridge: Option<kerf_bridge::Bridge>,
    pub state: EditorState,
    pub prefs: Prefs,
    pub picture: Option<Picture>,
    /// Notified when the playhead, the picture or the caption change ([`Playhead`]).
    pub playhead: Entity<Playhead>,
    /// The playhead was moved here (a seek) rather than by a clock: the
    /// next drain notifies [`Session::playhead`].
    moved: bool,
    /// The conversation the composer speaks in; None starts one.
    pub conversation: Option<String>,
    /// Clips chosen on the timeline.
    pub selection: Vec<String>,
    /// The media chosen in the band.
    pub chosen_media: Option<String>,
    /// The mask chosen on the picture or in the inspector, by its id on
    /// the chosen clip, so both choose the same one.
    pub chosen_mask: Option<String>,
    /// The kerfd this window was started with, if one was given; a project
    /// opened from it keeps it.
    kerfd: Option<PathBuf>,
    _on_quit: gpui::Subscription,
    /// A kerf:// link followed to this project, carried out once it is connected.
    link: Option<crate::links::Link>,
    mark_in: Option<f64>,
    mark_out: Option<f64>,
    looping: bool,
    /// Loop Playback has sent playback back to the loop's start since the
    /// playhead last played short of its end (Qt's `m_wrapped`), so the
    /// clocks still on their way from before that seek do not send it back
    /// again.
    wrapped: bool,
    /// Where the pointer last sent the picture while it moves, and when
    /// that frame is asked for exactly ([`PointerSeek`]).
    pointer: Option<PointerSeek>,
    /// What the toolbar's Export renders in one press, as its menu was
    /// last left in this window: the master or a draft.
    pub export_quality: crate::delivery::Quality,
    /// The open proposal the window's keys and menus act on: the pinned
    /// card's, set by the window (`ChatView::proposal_target`) before it
    /// asks what can run. None takes the newest pending one.
    pub pinned_proposal: Option<String>,
    /// What the window and Kerf did, a line each (Activity, in the title's menu).
    pub activity: ActivityLog,
    /// Each media's poster, filmstrip and peaks, made off the window's
    /// thread as the project's media come and change ([`crate::artwork`]).
    pub artwork: crate::artwork::Artwork,
    /// Whether the window shows its picture: a request is stamped with the
    /// playhead only while there is a picture to play (and the stamp is on,
    /// `prefs.send_playhead`), and a cut's card says "On the stage" only
    /// while it is. The window sets it (`Some`) as its picture pane opens
    /// and closes; until it does (None), a request is stamped.
    pub picture_open: Option<bool>,
    /// The tracks a source's picture and sound go on (Insert, Overwrite),
    /// as the timeline's target chips left them (`TimelineEvent::Targets`,
    /// which the window keeps here); None follows the first unlocked track
    /// of each kind.
    pub source_targets: Option<Vec<String>>,
    /// Proposals accepted or rejected from this window, by the id sent,
    /// and when: a settled card says how long ago.
    settled_here: HashMap<String, Instant>,
    /// A first cut rendering by itself once Kerf's edits settle.
    cut: CutRender,
    /// The live proposal's changes already followed while Kerf works
    /// ("clip@end"), so the playhead goes to the newest.
    live_changes: HashSet<String>,
    /// Watching what a proposal changed, while it plays.
    reel: Option<Reel>,
    /// What each request sent from here was about, for Ask again.
    asked: FocusBook,
    /// What wakes the pump ([`Session::pump`]): the bridge rings it with
    /// events and frames, the artwork with what it has made, the state with
    /// each notice said, and the session itself when something is due
    /// sooner than the pump waits for.
    wake: Option<kerf_bridge::Wake>,
    /// When the pump wakes by itself next, for what is due then (None: only
    /// when rung).
    pump_due: Option<Instant>,
    /// When a making's clock next says another second, so the window draws
    /// it ([`EditorState::making_second`]).
    making_second: Option<Instant>,
    /// How many notices there had been when the window last drew them.
    noticed: u64,
    _pump: Option<Task<()>>,
    _due_sooner: gpui::Subscription,
}

/// Which side of a change reel plays: what the proposal makes, or the
/// program as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReelSide {
    #[default]
    After,
    Before,
}

/// Watching what a proposal changes (Qt's ChangeReel): its segments played
/// one after another, from the proposal's preview (After) or the program
/// (Before), each waiting for its picture first.
#[derive(Clone, Debug, Default)]
pub struct Reel {
    pub proposal: String,
    pub segments: Vec<crate::changes::Segment>,
    pub index: usize,
    pub side: ReelSide,
    /// Waiting for the picture of the side and segment shown.
    pub waiting: bool,
    /// Past the last segment's end: every change watched.
    pub finished: bool,
    rendered: bool,
    previewing: bool,
    rendered_span: (f64, f64),
    segment_end: f64,
    limit: f64,
    /// What waits on time: playing once a seek has landed on the program
    /// reopened, or taking up the Before side when no word came of it.
    due: Option<(Instant, ReelWait)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReelWait {
    Play,
    Enter,
}

impl Reel {
    /// Seconds left to watch, the playhead at `playhead`.
    pub fn remaining(&self, playhead: f64) -> f64 {
        if self.finished {
            return 0.0;
        }
        let mut left = if self.waiting {
            self.segments[self.index].length()
        } else {
            (self.segment_end - playhead).max(0.0)
        };
        left += self.segments[self.index + 1..]
            .iter()
            .map(crate::changes::Segment::length)
            .sum::<f64>();
        left
    }

    /// What the reel's strip says: "Trim sea · 18 s left", "… · getting
    /// the picture", "Watched every change".
    pub fn words(&self, playhead: f64) -> String {
        let title = &self.segments[self.index].title;
        if self.finished {
            "Watched every change".to_owned()
        } else if self.waiting {
            format!("{title} · getting the picture")
        } else {
            format!(
                "{title} · {} s left",
                self.remaining(playhead).ceil() as i64
            )
        }
    }
}

/// The most a preview renders at once, in seconds.
pub const REEL_MOST_RENDERED: f64 = 120.0;

/// A first cut rendering by itself (Qt's `m_cutPreview` and
/// `m_previewedCut`): the proposal last asked to render, when Kerf's edits
/// will have been quiet long enough, and whether a render is under way
/// (one at a time; the newest after). [`Session`] drives it from the
/// bridge's events; each step takes the first cut shown (`cut`, its
/// proposal's id; None once the program has a cut) and the time.
#[derive(Clone, Debug, Default)]
pub struct CutRender {
    previewed: Option<String>,
    due: Option<Instant>,
    rendering: bool,
}

impl CutRender {
    /// The first cut shown changed: once Kerf's edits have been quiet for
    /// [`CUT_SETTLE`] it renders, under Follow Kerf and while no reel
    /// plays, unless it is the one rendered last. A cut no longer shown
    /// forgets what it rendered.
    pub fn follow(&mut self, cut: Option<&str>, follow_kerf: bool, reel: bool, now: Instant) {
        let Some(id) = cut else {
            self.due = None;
            self.previewed = None;
            return;
        };
        if follow_kerf && !reel && self.previewed.as_deref() != Some(id) {
            self.due = Some(now + CUT_SETTLE);
        }
    }

    /// The cut to render now: the one shown, once it is due, unless a
    /// render is under way (the newest follows it) or a reel plays. It is
    /// then the one rendered last.
    pub fn take_due(&mut self, cut: Option<&str>, reel: bool, now: Instant) -> Option<String> {
        let due = self.due?;
        if now < due || self.rendering || reel {
            return None;
        }
        self.due = None;
        let id = cut?.to_owned();
        self.previewed = Some(id.clone());
        Some(id)
    }

    /// Whether the render [`CutRender::take_due`] gave is under way (the
    /// bridge took it).
    pub fn set_rendering(&mut self, rendering: bool) {
        self.rendering = rendering;
    }

    /// A preview stopped running (`still`: another still renders): the
    /// next settled cut may render, and the cut shown, changed while it
    /// rendered, is due under Follow Kerf.
    pub fn rendered(&mut self, cut: Option<&str>, still: bool, follow_kerf: bool, now: Instant) {
        if !self.rendering || still {
            return;
        }
        self.rendering = false;
        if follow_kerf && cut.is_some_and(|id| self.previewed.as_deref() != Some(id)) {
            self.due = Some(now + CUT_SETTLE);
        }
    }

    /// A change reel ended over the first cut shown: the picture was the
    /// reel's, and the program (with no cut) came back under it, so the
    /// cut renders again once [`CUT_SETTLE`] has passed (Qt clears
    /// `m_previewedCut` and starts `m_cutPreview` as the reel ends).
    pub fn reel_ended(&mut self, cut: Option<&str>, now: Instant) {
        if cut.is_none() {
            return;
        }
        self.previewed = None;
        self.due = Some(now + CUT_SETTLE);
    }

    /// When the cut shown renders, if it is waiting to.
    pub fn due(&self) -> Option<Instant> {
        self.due
    }

    /// The proposal last asked to render.
    pub fn previewed(&self) -> Option<&str> {
        self.previewed.as_deref()
    }

    /// Whether a render it asked for is under way.
    pub fn rendering(&self) -> bool {
        self.rendering
    }
}

impl EventEmitter<Change> for Session {}

/// The most events one drain takes, so the window keeps drawing between them.
const DRAIN_MOST: usize = 256;

/// What the pump does after a drain.
enum Pump {
    /// Events are left over: drain again on the next turn, without waiting.
    Again,
    /// Wait to be rung, or until what waits on time is due.
    Wait(Option<Instant>),
}

/// Lets the window's other work run, then goes on.
async fn next_turn() {
    let mut yielded = false;
    future::poll_fn(|cx| {
        if yielded {
            return Poll::Ready(());
        }
        yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await;
}

/// Waits until `wake` rings or `timer` (if any) ends, whichever is first;
/// true when the timer ended.
async fn first_of(wake: impl Future<Output = ()>, timer: Option<&mut Task<()>>) -> bool {
    let mut wake = pin!(wake);
    let mut timer = timer;
    future::poll_fn(|cx| {
        if wake.as_mut().poll(cx).is_ready() {
            return Poll::Ready(false);
        }
        match timer.as_deref_mut() {
            Some(timer) => Pin::new(timer).poll(cx).map(|()| true),
            None => Poll::Pending,
        }
    })
    .await
}

/// Every event the bridge sends, as it sent it, for a region that keeps
/// state of its own (a check's findings, a making's plan) rather than in
/// [`EditorState`]. One event, shared by every region that hears it.
#[derive(Clone, Debug)]
pub struct BridgeEvent(pub Arc<Event>);

impl EventEmitter<BridgeEvent> for Session {}

impl Session {
    /// A session on `project`, its bridge started without waiting on it.
    pub fn open(
        project: Option<PathBuf>,
        kerfd: Option<PathBuf>,
        prefs: Prefs,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let bridge = project.as_deref().map(|project| {
                let kerfd = kerfd.clone().unwrap_or_else(discover_kerfd);
                kerf_bridge::Bridge::open(project, &kerfd)
            });
            let wake = bridge.as_ref().map(kerf_bridge::Bridge::wake);
            let pump = wake.clone().map(|wake| Self::pump(wake, window, cx));
            let artwork = crate::artwork::Artwork::new(project.clone());
            if let Some(wake) = wake.clone() {
                artwork.wake_with(move || wake.ring());
            }
            let mut session = Self {
                project,
                bridge,
                state: EditorState::default(),
                prefs,
                picture: None,
                playhead: cx.new(|_| Playhead),
                moved: false,
                conversation: None,
                selection: Vec::new(),
                chosen_media: None,
                chosen_mask: None,
                kerfd: kerfd.clone(),
                // Quitting closes the bridge as closing the window does: the
                // render running stops, the queued ones are cancelled.
                _on_quit: cx.on_app_quit(|session: &mut Session, _| {
                    session.bridge.take();
                    async {}
                }),
                link: None,
                mark_in: None,
                mark_out: None,
                looping: false,
                wrapped: false,
                pointer: None,
                export_quality: crate::delivery::Quality::default(),
                pinned_proposal: None,
                activity: ActivityLog::default(),
                artwork,
                picture_open: None,
                source_targets: None,
                settled_here: HashMap::new(),
                cut: CutRender::default(),
                live_changes: HashSet::new(),
                reel: None,
                asked: FocusBook::default(),
                wake,
                pump_due: None,
                making_second: None,
                noticed: 0,
                _pump: pump,
                // A seek or a reel's wait set from the window: the pump wakes
                // for it if it is due sooner than it waits for.
                _due_sooner: cx.observe_self(|session, _| session.wake_if_due_sooner()),
            };
            // The render queue is asked for once kerfd is connected
            // (`drain`), never before: asked of a runtime that is not there
            // it would be refused, and the refusal left standing.
            // A notice said from anywhere wakes the pump to draw it.
            session.state.wake = session.wake.clone();
            session.send(Command::Conversations(ConversationsRequest::List));
            session
        })
    }

    /// Drains the bridge each time `wake` rings, and when what waits on time
    /// is due; at once again while events are left over from a full drain.
    fn pump(wake: kerf_bridge::Wake, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            // The timer for what is due next, kept while that stays the same.
            let mut timer: Option<(Instant, Task<()>)> = None;
            loop {
                let Ok(next) = this.update_in(cx, |session, window, cx| session.drain(window, cx))
                else {
                    break;
                };
                let due = match next {
                    Pump::Again => {
                        next_turn().await;
                        continue;
                    }
                    Pump::Wait(due) => due,
                };
                if timer.as_ref().map(|(at, _)| *at) != due {
                    timer = due.map(|due| {
                        let wait = due.saturating_duration_since(Instant::now());
                        // Due already, and not done (a seek the bridge would
                        // not take yet): tried again shortly.
                        let wait = if wait.is_zero() { motion::POLL } else { wait };
                        (due, cx.background_executor().timer(wait))
                    });
                }
                if first_of(wake.wait(), timer.as_mut().map(|(_, timer)| timer)).await {
                    timer = None;
                }
            }
        })
    }

    /// What waits on time here, soonest first: the pointer's exact seek, a
    /// first cut that renders once settled, a change reel's wait, a notice
    /// going, a making's clock saying another second.
    fn next_due(&self) -> Option<Instant> {
        let pointer = self.pointer.and_then(|pointer| pointer.settle);
        // A cut waits for the render under way, or the reel, to end: an event
        // says when.
        let cut = self
            .cut
            .due()
            .filter(|_| !self.cut.rendering() && self.reel.is_none());
        let reel = self
            .reel
            .as_ref()
            .and_then(|reel| reel.due)
            .map(|(at, _)| at);
        let notice = self
            .state
            .notices_due(motion::NOTICE_HOLD, motion::NOTICE_ERROR_HOLD);
        [pointer, cut, reel, notice, self.making_second]
            .into_iter()
            .flatten()
            .min()
    }

    /// The playhead was moved from here: the pump drains at once and tells
    /// the views that draw it, as a clock would have.
    fn playhead_moved(&mut self) {
        self.moved = true;
        if let Some(wake) = &self.wake {
            wake.ring();
        }
    }

    /// Wakes the pump when something is due sooner than it waits for.
    fn wake_if_due_sooner(&self) {
        let Some(due) = self.next_due() else {
            return;
        };
        if self.pump_due.is_none_or(|waits| due < waits) {
            if let Some(wake) = &self.wake {
                wake.ring();
            }
        }
    }

    /// Takes every event the bridge has (at most a frame's worth), and its
    /// newest picture; what the pump does next.
    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Pump {
        let Some(bridge) = self.bridge.as_mut() else {
            return Pump::Wait(None);
        };
        let mut events = Vec::new();
        // Never more than a frame's worth at once, so the window keeps drawing.
        let mut more = false;
        while !more {
            match bridge.poll() {
                Some(event) => events.push(event),
                None => break,
            }
            more = events.len() >= DRAIN_MOST;
        }
        let frame = bridge.frame();
        // What any view may draw changed; only the playhead, the picture or
        // the caption did.
        let mut changed = false;
        let mut ticked = false;
        for event in events {
            if log::log_enabled!(log::Level::Debug)
                && !matches!(event, Event::Clock { .. } | Event::Meter(_))
            {
                let json = serde_json::to_value(&event).unwrap_or_default();
                log::debug!("bridge event: {}", truncate(&json.to_string(), 400));
            }
            // A clock, caption or meter reading the same as the last.
            let same = self.state.leaves_as_is(&event);
            let was_playing = self.state.playing;
            let change = self.state.apply(&event);
            if let Some((kind, text)) = activity_of(&event) {
                self.activity.push(kind, &text);
            }
            for (kind, text) in self.state.take_logged() {
                self.activity.push(kind, &text);
            }
            if change == Change::Connection
                && self.state.connection == crate::store::Connection::Connected
            {
                self.activity.push("project", "Project connected");
            }
            let event = Arc::new(event);
            cx.emit(BridgeEvent(event.clone()));
            if let Event::Preview(preview) = &*event {
                self.reel_follow_preview(preview.status, preview.proposal_id.as_deref());
            }
            match &change {
                Change::Snapshot => {
                    self.follow_link();
                    self.follow_media();
                    self.follow_live_proposal();
                    self.follow_cut();
                }
                Change::Preview => {
                    // A picture that needs a render shows none until it is
                    // rendered, as Qt cleared its viewer.
                    if let Event::Preview(preview) = &*event {
                        if matches!(
                            preview.status,
                            PreviewStatus::Required | PreviewStatus::Unavailable
                        ) {
                            self.drop_picture(window, cx);
                        }
                    }
                    self.cut_rendered();
                }
                Change::Connection
                    if self.state.connection == crate::store::Connection::Connected =>
                {
                    // The jobs kerfd has, this window's or another's, finished
                    // ones among them (Finish's Deliver page, the toolbar).
                    self.send(Command::RenderQueue);
                    // Live Sync as it was left for this project.
                    if self
                        .project
                        .as_ref()
                        .is_some_and(|project| self.prefs.live_sync.contains(project))
                    {
                        self.send(Command::CollabLive {
                            enabled: true,
                            interval: None,
                        });
                    }
                }
                // Gone, or only connecting: nothing plays, and the picture
                // and its caption go, as Qt's viewer was cleared.
                Change::Connection => self.lose_picture(window, cx),
                Change::Playback => self.follow_playing(was_playing),
                Change::ViewerReset => self.viewer_reset(window, cx),
                Change::Work if matches!(*event, Event::AgentFocus { .. }) => self.follow_kerf(),
                Change::Clock => {
                    // Going round the loop, or on through a change reel,
                    // changes more than the playhead.
                    let looped = self.keep_to_marks();
                    let reeled = self.reel_follow_playhead();
                    changed |= looped || reeled;
                }
                Change::TurnEnded => {
                    self.live_changes.clear();
                    // The conversation as it is kept now, with Kerf's answer.
                    if let Some(id) = self.conversation.clone() {
                        self.send(Command::Conversations(ConversationsRequest::Open { id }));
                    }
                }
                _ => {}
            }
            // The conversation shown is the state's choice (a request sent
            // with none chosen starts one; a deleted one leaves none).
            self.conversation = self.state.chosen.clone();
            if same {
                continue;
            }
            match change {
                Change::Clock => ticked = true,
                // No view draws the meter.
                _ if matches!(*event, Event::Meter(_)) => {}
                // Playing, or not, as it was (each seek while paused says
                // so): heard, and nothing drawn again.
                Change::Playback if self.state.playing == was_playing => {}
                _ => changed = true,
            }
            cx.emit(change);
        }
        if self.render_settled_cut() || self.reel_due() {
            changed = true;
        }
        self.settle_pointer();
        // Artwork made since: drawn now; what it let go of leaves the atlas.
        for image in self.artwork.take_dropped() {
            // gpui 0.2.2 answers Ok always: one not in the atlas is no matter.
            let _ = window.drop_image(image);
        }
        if self.artwork.take_changed() {
            changed = true;
        }
        if let Some(frame) = frame {
            if self.show_frame(frame, window, cx) {
                cx.emit(Change::Clock);
                ticked = true;
            }
        }
        if self
            .state
            .expire_notices(motion::NOTICE_HOLD, motion::NOTICE_ERROR_HOLD)
        {
            cx.emit(Change::Notice);
            changed = true;
        }
        // A notice said from anywhere (the state rang for it), drawn now.
        if self.state.noticed() != self.noticed {
            self.noticed = self.state.noticed();
            changed = true;
        }
        // A making's clock says another second ("0:41"), wherever it shows.
        let now = Instant::now();
        if self.making_second.is_some_and(|second| second <= now) {
            changed = true;
        }
        self.making_second = self.state.making_second(now);
        // Nothing heard, nothing drawn: the window holds still between events.
        if changed {
            cx.notify();
        }
        // Only the views that draw the playhead, the picture and the caption.
        ticked |= std::mem::take(&mut self.moved);
        if ticked {
            self.playhead.update(cx, |_, cx| cx.notify());
        }
        if more {
            return Pump::Again;
        }
        self.pump_due = self.next_due();
        Pump::Wait(self.pump_due)
    }

    /// Shows `frame` as the picture; whether it did.
    fn show_frame(
        &mut self,
        frame: kerf_bridge::Frame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // kerfd gone: what is left of its playback is not shown.
        if matches!(self.state.connection, crate::store::Connection::Failed(_)) {
            return false;
        }
        if let Some(pointer) = self.pointer {
            let fps = self.timeline().rate();
            let shown = self.picture.as_ref().map(|picture| picture.time);
            if !nearer_frame(shown, frame.time, pointer.target, fps) {
                return false;
            }
            // The frame the pointer let go on (or rested on) has come: what
            // comes after shows as it comes.
            if pointer.settle.is_none() && (frame.time - pointer.target).abs() <= 1.0 / fps.max(1.0)
            {
                self.pointer = None;
            }
        }
        let Some(image) = render_image(&frame.pixels, frame.width, frame.height) else {
            return false;
        };
        self.drop_picture(window, cx);
        self.picture = Some(Picture {
            image,
            width: frame.width,
            height: frame.height,
            time: frame.time,
            standing_in: false,
        });
        true
    }

    fn drop_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.picture.take() {
            cx.drop_image(old.image, Some(window));
        }
    }

    /// Playback was opened again (an edit, a reconnect, a preview left):
    /// the frame shown stands in until the new playback's first comes, as
    /// Qt's did, unless the new canvas has another shape (it would be
    /// stretched) or there is nothing to play (no first frame will come).
    /// The caption goes either way.
    fn viewer_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let canvas = self.timeline().canvas();
        let duration = self.state.snapshot.duration;
        let stands = self
            .picture
            .as_ref()
            .is_some_and(|picture| stands_in((picture.width, picture.height), canvas, duration));
        if stands {
            if let Some(picture) = self.picture.as_mut() {
                picture.standing_in = true;
            }
        } else {
            self.drop_picture(window, cx);
        }
        self.state.caption.clear();
        // The program the pointer was skimming is no longer the one shown.
        self.pointer = None;
    }

    /// kerfd is gone, or only connecting: nothing plays, and the picture
    /// and its caption go, with the pointer's seeks, as Qt's
    /// `setConnection` stopped playback and cleared its viewer.
    fn lose_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.playing = false;
        self.pointer = None;
        self.wrapped = false;
        self.drop_picture(window, cx);
        self.state.caption.clear();
    }

    /// Follows the project's media as a snapshot has it: its artwork is
    /// made for media new or changed, and kerfd is asked to describe media
    /// it has not been asked about since connecting (`media_info`: the
    /// pixel format a tile's alpha checkerboard follows).
    fn follow_media(&mut self) {
        let sources = &self.state.snapshot.timeline.sources;
        self.artwork.sync(sources);
        if self.state.connection != crate::store::Connection::Connected {
            return;
        }
        let fresh: Vec<String> = sources
            .iter()
            .map(|source| source.id.clone())
            .filter(|id| !id.is_empty() && !self.state.info_asked.contains(id))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let media = match fresh.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        };
        if self.send(Command::MediaInfo { media }) {
            self.state.info_asked.extend(fresh);
        }
    }

    /// Asks for media `media`'s index (its shots, silences and loudest
    /// moments) unless it is known, refused or asked for already. The
    /// Media band's Index page calls it for the chosen media.
    pub fn ask_media_index(&mut self, media: &str) {
        let state = &self.state;
        if media.is_empty()
            || state.media_indexes.contains_key(media)
            || state.media_index_errors.contains_key(media)
            || state.index_asked.contains(media)
        {
            return;
        }
        if self.send(Command::MediaIndex {
            media: media.to_owned(),
        }) {
            self.state.index_asked.insert(media.to_owned());
        }
    }

    /// Media `id`'s readable name ([`crate::names`]).
    pub fn media_name(&self, id: &str) -> String {
        crate::model::TimelineShow::media_name(self.timeline(), id)
    }

    /// Follows `link` once the project is connected: seeks to its moment,
    /// and says so when it names another revision.
    pub fn follow(&mut self, link: crate::links::Link) {
        self.link = Some(link);
        if self.state.has_snapshot {
            self.follow_link();
        }
    }

    fn follow_link(&mut self) {
        let Some(link) = self.link.take() else {
            return;
        };
        let fps = self.timeline().rate();
        if let Some(seconds) = crate::links::seconds(&link, fps) {
            self.seek(seconds, false);
        }
        if let Some(revision) = link
            .revision
            .filter(|revision| *revision != self.revision())
        {
            let now = self.revision();
            let said =
                format!("The link was to revision {revision}; the project is at revision {now}.");
            self.state.notify(&said, false);
            self.activity.push("link", &said);
        }
    }

    /// Sends one command; false when the bridge refused it (its queue is
    /// full), which the window says rather than dropping an edit unseen.
    ///
    /// While kerfd is connecting or gone, what waits on it (edits, renders,
    /// playback and seeks) is not sent, as Qt's `sendSimple` sent nothing
    /// then ([`sent_unconnected`]): the window says it is disconnected
    /// once, and an edit refused for it would only say so again. False then.
    pub fn send(&mut self, command: Command) -> bool {
        if !self.may_send(command.kind()) {
            return false;
        }
        let activity = activity_of_command(&command, self.timeline());
        let Some(bridge) = self.bridge.as_mut() else {
            return false;
        };
        let sent = bridge.send(command);
        if !sent {
            self.refused();
        } else if let Some((kind, text)) = activity {
            self.activity.push(kind, &text);
        }
        sent
    }

    /// Whether a command of `kind` goes now, as [`Session::send`] sends it:
    /// with a bridge, and (unless it is one [`sent_unconnected`]) once kerfd
    /// is connected. A seek or playback that goes takes the picture from the
    /// pointer, and a pause ends Loop Playback's wrap.
    fn may_send(&mut self, kind: &str) -> bool {
        if self.bridge.is_none() || (!self.connected() && !sent_unconnected(kind)) {
            return false;
        }
        // A seek or playback from anywhere but the pointer takes the
        // picture from it ([`Session::skim`]).
        if matches!(kind, "seek" | "play" | "pause") {
            self.pointer = None;
        }
        // Paused from here: a stop that follows is not Loop Playback's to
        // play on from.
        if kind == "pause" {
            self.wrapped = false;
        }
        true
    }

    /// The bridge would not take a command: the window says so. False.
    fn refused(&mut self) -> bool {
        self.state
            .notify("Kerf is busy; that was not sent. Try again.", true);
        false
    }

    pub fn has_project(&self) -> bool {
        self.bridge.is_some()
    }

    /// Whether kerfd is connected: until it is, and once it is gone, the
    /// timeline and the format bank take no edits and nothing that waits
    /// on it is sent (Qt's `m_connected`).
    pub fn connected(&self) -> bool {
        self.state.connection == crate::store::Connection::Connected
    }

    pub fn timeline(&self) -> &Timeline {
        &self.state.snapshot.timeline
    }

    pub fn revision(&self) -> u64 {
        self.state.snapshot.revision
    }

    /// The project's name: its document's, else its folder's.
    pub fn project_name(&self) -> String {
        let name = &self.state.snapshot.timeline.name;
        if !name.trim().is_empty() {
            return name.clone();
        }
        self.project
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Industrial Kerf".into())
    }

    /// Typed edits, in frames of the sequence, each written as JSON (an
    /// [`kerf_bridge::EditOp`]): refused, as the bridge refused them, when
    /// one does not read as an op.
    pub fn edit(&mut self, ops: Vec<Value>) -> bool {
        let revision = self.revision();
        let ops: Result<Vec<kerf_bridge::EditOp>, _> =
            ops.into_iter().map(serde_json::from_value).collect();
        match ops {
            Ok(ops) => self.send(Command::Edit { ops, revision }),
            Err(_) => self.may_send("edit") && self.refused(),
        }
    }

    pub fn undo(&mut self) {
        let revision = self.revision();
        self.send(Command::Undo { revision });
    }

    pub fn redo(&mut self) {
        let revision = self.revision();
        self.send(Command::Redo { revision });
    }

    /// Moves the playhead to `seconds`. `scrub`: the pointer is dragging
    /// it, and the seek is rough while it moves (unless the frame is just
    /// ahead of the one shown, [`scrubs_from`]), then exact once it rests
    /// ([`SCRUB_SETTLE`]); let go (`scrub` false after a drag), until the
    /// frame there comes none farther from it shows. Nothing moves while
    /// kerfd is not connected (Qt's `queueSeek`).
    pub fn seek(&mut self, seconds: f64, scrub: bool) {
        if !self.connected() {
            return;
        }
        let seconds = seconds.clamp(0.0, self.state.program_length().max(0.0));
        self.state.position = seconds;
        self.playhead_moved();
        if scrub {
            self.pointer_seek(seconds);
            return;
        }
        let pointed = self.pointer.is_some();
        if self.send(Command::Seek {
            position: seconds,
            scrub: false,
        }) && pointed
        {
            self.pointer = Some(PointerSeek {
                target: seconds,
                settle: None,
            });
        }
    }

    /// The pointer skims the picture or the lanes to `seconds`, the
    /// playhead staying where it is: kerfd shows the program there, roughly
    /// while the pointer moves and exactly once it rests
    /// ([`SCRUB_SETTLE`]), as Qt's `queueHoverSeek` and
    /// `settlePointerSeek` did. Nothing while kerfd is not connected.
    pub fn skim(&mut self, seconds: f64) {
        if !self.connected() || !seconds.is_finite() {
            return;
        }
        let seconds = seconds.clamp(0.0, self.state.program_length().max(0.0));
        self.pointer_seek(seconds);
    }

    /// Where the pointer has sent the picture while it moves, and when
    /// that frame is asked for exactly ([`PointerSeek`]).
    pub fn pointer(&self) -> Option<PointerSeek> {
        self.pointer
    }

    /// Seeks for the pointer to `seconds`: rough unless the frame is just
    /// ahead of the one shown, and then exact once the pointer has rested.
    fn pointer_seek(&mut self, seconds: f64) {
        let shown = self.picture.as_ref().map(|picture| picture.time);
        let scrub = scrubs_from(shown, seconds, self.timeline().rate());
        if self.send(Command::Seek {
            position: seconds,
            scrub,
        }) {
            self.pointer = Some(PointerSeek {
                target: seconds,
                settle: scrub.then(|| Instant::now() + SCRUB_SETTLE),
            });
            self.wake_if_due_sooner();
        }
    }

    /// The pointer has rested since its last rough seek: the frame under
    /// it is asked for exactly, once.
    fn settle_pointer(&mut self) {
        let Some(pointer) = self.pointer else {
            return;
        };
        if pointer.settle.is_none_or(|at| Instant::now() < at) {
            return;
        }
        let sent = self.send(Command::Seek {
            position: pointer.target,
            scrub: false,
        });
        self.pointer = Some(PointerSeek {
            target: pointer.target,
            settle: if sent { None } else { pointer.settle },
        });
    }

    /// Pauses what plays, always (kerfd plays on through a render started
    /// meanwhile); else plays, only while Play can run
    /// ([`EditorState::playable`]), as Qt's `togglePlayback` returned, so a
    /// click on the picture does not start it while a render is needed or
    /// runs. True when it was sent.
    pub fn toggle_playback(&mut self) -> bool {
        match self.state.playback_toggle() {
            Some("pause") => self.send(Command::Pause),
            Some(_) => self.send(Command::Play),
            None => false,
        }
    }

    pub fn accept(&mut self, proposal: &str) {
        let revision = self.revision();
        if self.send(Command::Accept {
            proposal_id: proposal.to_owned(),
            revision,
        }) {
            self.settled_here
                .insert(proposal.to_owned(), Instant::now());
        }
    }

    pub fn reject(&mut self, proposal: &str) {
        let revision = self.revision();
        if self.send(Command::Reject {
            proposal_id: proposal.to_owned(),
            revision,
        }) {
            self.settled_here
                .insert(proposal.to_owned(), Instant::now());
        }
    }

    /// When a proposal was settled from this window, by the id its Accept
    /// or Reject sent: `is` says whether an id is the proposal's (any of
    /// its versions). None for one settled elsewhere, or before.
    pub fn settled_at(&self, is: impl Fn(&str) -> bool) -> Option<Instant> {
        self.settled_here
            .iter()
            .filter(|(id, _)| is(id))
            .map(|(_, at)| *at)
            .max()
    }

    pub fn forward(&mut self, proposal: &str) {
        let revision = self.revision();
        self.send(Command::Forward {
            proposal_id: proposal.to_owned(),
            revision,
        });
    }

    /// Renders the preview of `proposal` (any of its versions' ids), over
    /// the stretch it changes ([`preview_request`]), or of the program.
    /// True when it was sent (not while kerfd is not connected).
    pub fn preview(&mut self, proposal: Option<&str>) -> bool {
        let command = preview_request(&self.state.snapshot, proposal);
        self.send(command)
    }

    pub fn import(&mut self, path: &Path, media_only: bool) {
        let sent = self.send(Command::Import {
            path: path.to_string_lossy().into_owned(),
            media_only,
        });
        if sent {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            self.activity.push("import", &format!("Importing {name}"));
        }
    }

    /// Queues a render of `quality` as [`crate::delivery::request`] makes it:
    /// a master goes where the project keeps it (its master output, else
    /// deliveries/), a review copy into deliveries/, as ⌘E and the
    /// toolbar's Export send them. False when it was not sent.
    pub fn export(&mut self, quality: &str) -> bool {
        let command = crate::delivery::request(self.project.as_deref(), &self.prefs, quality);
        let sent = self.send(command);
        if sent {
            let name = match quality {
                "master" => "Master",
                "review" => "Draft",
                "full" => "Full render",
                "preview" => "Preview",
                other => other,
            };
            self.activity
                .push("export", &format!("Export started ({name})"));
        }
        sent
    }

    /// Renders the master (`master`) or a review copy (`review`) into the
    /// project's deliveries/ (Finish, ⌘E): [`Session::export`].
    pub fn deliver(&mut self, quality: &str) {
        self.export(quality);
    }

    /// Turns Live Sync on or off for this project, and remembers it for the
    /// next time the project opens.
    pub fn set_live_sync(&mut self, on: bool) {
        self.send(Command::CollabLive {
            enabled: on,
            interval: None,
        });
        if let Some(project) = self.project.clone() {
            self.prefs.live_sync.retain(|known| known != &project);
            if on {
                self.prefs.live_sync.push(project);
            }
            self.prefs.save();
        }
    }

    pub fn cancel_export(&mut self, job: &str) {
        self.send(Command::ExportCancel {
            job_id: job.to_owned(),
        });
    }

    /// Opens a conversation kept beside the project; None starts a new one.
    pub fn open_conversation(&mut self, id: Option<String>) {
        self.state.choose(id.clone());
        self.conversation = id.clone();
        if let Some(id) = id {
            self.send(Command::Conversations(ConversationsRequest::Open { id }));
        }
    }

    /// Asks the bridge to reach kerfd again after it went away: connecting
    /// once the bridge has taken it; else still disconnected, saying so, so
    /// Reconnect stays to be pressed again.
    pub fn reconnect(&mut self) {
        let sent = self
            .bridge
            .as_mut()
            .is_some_and(|bridge| bridge.send(Command::Reconnect));
        self.state.connection = if sent {
            crate::store::Connection::Connecting
        } else {
            crate::store::Connection::Failed(RECONNECT_REFUSED.to_owned())
        };
    }

    /// While Kerf works, the playhead goes where it is looking (Follow
    /// Kerf, View ▸ Follow Kerf): the first time it names, else the start
    /// of the first clip it names (in the program, else in the cut it
    /// proposes).
    fn follow_kerf(&mut self) {
        let at = self.state.agent_focus.first().copied().or_else(|| {
            let clip = self.state.agent_clips.first()?;
            let start =
                |timeline: &Timeline| timeline.clip(clip).map(|(_, clip)| clip.timeline_start);
            start(self.timeline()).or_else(|| self.live_cut().and_then(|(_, cut)| start(cut)))
        });
        if let Some(at) = at {
            self.show_agent_at(at);
        }
    }

    /// The playhead goes to `at`, where Kerf is at work, under Follow Kerf,
    /// unless something is playing or there is nothing to play.
    fn show_agent_at(&mut self, at: f64) {
        if !self.prefs.follow_kerf
            || self.state.playing
            || self.reel.is_some()
            || !at.is_finite()
            || at < 0.0
        {
            return;
        }
        let proposed = self.live_cut().map_or(0.0, |(_, cut)| timeline_end(cut));
        let length = self.state.program_length().max(proposed);
        if length <= 0.0 {
            return;
        }
        let at = at.min(length);
        self.state.position = at;
        self.playhead_moved();
        self.send(Command::Seek {
            position: at,
            scrub: false,
        });
    }

    /// The open proposal the window's keys act on (the pinned card's), else
    /// the newest, with the timeline it would leave.
    pub fn live_cut(&self) -> Option<(&crate::model::Proposal, &Timeline)> {
        let snapshot = &self.state.snapshot;
        let proposal = self
            .pinned_proposal
            .as_deref()
            .and_then(|pinned| {
                snapshot
                    .proposals
                    .iter()
                    .find(|proposal| proposal.proposal_id == pinned && proposal.is_pending())
            })
            .or_else(|| snapshot.pending())?;
        let timeline = snapshot
            .proposal_timelines
            .get(&proposal.proposal_id)
            .or_else(|| {
                snapshot
                    .pending()
                    .filter(|pending| pending.proposal_id == proposal.proposal_id)
                    .and(snapshot.proposal_timeline.as_ref())
            })?;
        Some((proposal, timeline))
    }

    /// The first cut Kerf proposes on a program with no clips yet, and the
    /// timeline it would leave: what the picture and the timeline show
    /// until it is accepted. None once the program has a cut.
    pub fn first_cut(&self) -> Option<(&crate::model::Proposal, &Timeline)> {
        if self.timeline().has_clips() {
            return None;
        }
        self.live_cut()
            .filter(|(_, cut)| cut.has_clips() && timeline_end(cut) > 0.0)
    }

    /// Whether Kerf is making a first cut now: its card is at work.
    pub fn cutting(&self) -> bool {
        self.state.work.busy() && self.first_cut().is_some()
    }

    /// While Kerf works, the playhead goes to the newest change in the cut
    /// it proposes (where a trim cuts, else where the clip now starts), and
    /// a first cut's turn says how many clips it has placed.
    fn follow_live_proposal(&mut self) {
        if !self.state.work.busy() {
            return;
        }
        let Some((_, cut)) = self.live_cut() else {
            return;
        };
        let changes = crate::changes::clip_changes(self.timeline(), cut);
        let first = !self.timeline().has_clips();
        let placed = changes
            .iter()
            .filter(|change| !change.removed)
            .filter(|change| {
                cut.clip(&change.clip)
                    .is_some_and(|(track, _)| track.kind != crate::model::TrackKind::Audio)
            })
            .count();
        let (keys, newest) = newest_change(&changes, &self.live_changes);
        let moved = keys != self.live_changes;
        self.live_changes = keys;
        if first && moved {
            self.state.work.status = cutting_status(placed);
        }
        if let Some(at) = newest {
            self.show_agent_at(at);
        }
    }

    /// A first cut changed: once Kerf's edits have been quiet for
    /// [`CUT_SETTLE`] it renders by itself, under Follow Kerf. A cut no
    /// longer shown forgets what it rendered.
    fn follow_cut(&mut self) {
        let cut = self.first_cut_id();
        self.cut.follow(
            cut.as_deref(),
            self.prefs.follow_kerf,
            self.reel.is_some(),
            Instant::now(),
        );
    }

    /// The id of the first cut shown ([`Session::first_cut`]).
    fn first_cut_id(&self) -> Option<String> {
        self.first_cut()
            .map(|(proposal, _)| proposal.proposal_id.clone())
    }

    /// The settled cut renders, unless one renders already (the newest
    /// follows it). True when a render was asked for.
    fn render_settled_cut(&mut self) -> bool {
        if self.cut.due().is_none() {
            return false;
        }
        let cut = self.first_cut_id();
        let Some(id) = self
            .cut
            .take_due(cut.as_deref(), self.reel.is_some(), Instant::now())
        else {
            return false;
        };
        let command = preview_request(&self.state.snapshot, Some(&id));
        let asked = self.send(command);
        self.cut.set_rendering(asked);
        asked
    }

    /// A preview stopped running: the next settled cut may render, and one
    /// changed while it rendered does.
    fn cut_rendered(&mut self) {
        let cut = self.first_cut_id();
        self.cut.rendered(
            cut.as_deref(),
            self.state.preview.rendering(),
            self.prefs.follow_kerf,
            Instant::now(),
        );
    }

    /// The change reel, while one plays.
    pub fn reel(&self) -> Option<&Reel> {
        self.reel.as_ref()
    }

    /// Watches what `proposal` changes: `segments` one after another, from
    /// its preview. False with nothing to watch.
    pub fn watch_changes(
        &mut self,
        proposal: &str,
        segments: Vec<crate::changes::Segment>,
    ) -> bool {
        if segments.is_empty() {
            self.state
                .notify("This proposal changes nothing on the timeline", false);
            return false;
        }
        let same = self
            .reel
            .as_ref()
            .is_some_and(|reel| reel.previewing && reel.proposal == proposal);
        let limit = segments
            .iter()
            .map(|segment| segment.after_end.max(segment.before_end))
            .fold(0.0, f64::max);
        self.reel = Some(Reel {
            proposal: proposal.to_owned(),
            segments,
            rendered: same,
            previewing: same,
            limit,
            ..Reel::default()
        });
        self.reel_show();
        true
    }

    /// Stops watching: playback stops, and the program shows again; over
    /// a first cut, the cut renders again once settled, the picture
    /// having been the reel's.
    pub fn reel_stop(&mut self) {
        let Some(reel) = self.reel.take() else {
            return;
        };
        self.send(Command::Pause);
        if reel.previewing {
            self.send(Command::PreviewExit);
        }
        let cut = self.first_cut_id();
        self.cut.reel_ended(cut.as_deref(), Instant::now());
        self.wake_if_due_sooner();
    }

    /// The next segment, or the one before.
    pub fn reel_step(&mut self, forward: bool) {
        let Some(reel) = self.reel.as_mut() else {
            return;
        };
        let count = reel.segments.len();
        if forward && reel.index + 1 < count {
            reel.index += 1;
        } else if !forward && reel.index > 0 {
            reel.index -= 1;
        } else {
            return;
        }
        reel.finished = false;
        self.reel_show();
    }

    /// Plays the program as it is (Before) or what the proposal makes.
    pub fn reel_side(&mut self, side: ReelSide) {
        let Some(reel) = self.reel.as_mut().filter(|reel| reel.side != side) else {
            return;
        };
        reel.side = side;
        reel.finished = false;
        self.reel_show();
    }

    fn reel_show(&mut self) {
        let Some(reel) = self.reel.as_mut() else {
            return;
        };
        reel.due = None;
        let segment = reel.segments[reel.index].clone();
        if reel.side == ReelSide::Before {
            if reel.previewing {
                // The program comes back once playback has reopened it.
                reel.previewing = false;
                reel.rendered = false;
                reel.waiting = true;
                self.send(Command::Pause);
                self.send(Command::PreviewExit);
                return;
            }
            self.reel_enter();
            return;
        }
        if reel.rendered
            && segment.after >= reel.rendered_span.0
            && segment.after_end <= reel.rendered_span.1
        {
            self.reel_enter();
            return;
        }
        // All the segments when they fit in one render, else this one.
        let first = reel.segments[0].after;
        let last = reel.segments[reel.segments.len() - 1].after_end;
        reel.rendered_span = if last - first <= REEL_MOST_RENDERED {
            (first, last)
        } else {
            (
                segment.after,
                segment.after_end.min(segment.after + REEL_MOST_RENDERED),
            )
        };
        reel.rendered = false;
        reel.waiting = true;
        let (start, end) = reel.rendered_span;
        let proposal = reel.proposal.clone();
        self.send(Command::Pause);
        let asked = self.send(Command::Preview {
            proposal_id: Some(proposal),
            start: Some(start),
            duration: Some(end - start),
        });
        if !asked {
            self.reel_stop();
        }
    }

    fn reel_enter(&mut self) {
        let Some(reel) = self.reel.as_mut() else {
            return;
        };
        reel.waiting = false;
        let segment = &reel.segments[reel.index];
        let after = reel.side == ReelSide::After;
        reel.segment_end = if after {
            segment.after_end
        } else {
            segment.before_end
        };
        let at = if after { segment.after } else { segment.before };
        let limit = reel.limit.max(self.state.program_length());
        if !after {
            // The program has only just reopened: the seek lands first.
            reel.due = Some((Instant::now() + REEL_SEEK_LANDS, ReelWait::Play));
            self.wake_if_due_sooner();
        }
        let at = at.clamp(0.0, limit);
        self.state.position = at;
        self.playhead_moved();
        self.send(Command::Seek {
            position: at,
            scrub: false,
        });
        if after {
            self.send(Command::Play);
        }
    }

    /// What waits on time is done when it is due. True when anything was.
    fn reel_due(&mut self) -> bool {
        let Some((at, wait)) = self.reel.as_ref().and_then(|reel| reel.due) else {
            return false;
        };
        if Instant::now() < at {
            return false;
        }
        if let Some(reel) = self.reel.as_mut() {
            reel.due = None;
        }
        match wait {
            ReelWait::Play => {
                self.send(Command::Play);
            }
            ReelWait::Enter => {
                if self.reel.as_ref().is_some_and(|reel| reel.waiting) {
                    self.reel_enter();
                }
            }
        }
        true
    }

    /// The bridge's `preview` events: a render ready starts the segment,
    /// the program reopened starts the Before side, one gone or failed
    /// ends the reel.
    fn reel_follow_preview(&mut self, status: PreviewStatus, proposal: Option<&str>) {
        let Some(reel) = self.reel.as_mut() else {
            return;
        };
        let named = proposal.is_some_and(|id| !id.is_empty());
        match status {
            PreviewStatus::Running => {}
            PreviewStatus::Ready | PreviewStatus::Live => {
                if reel.side == ReelSide::After && reel.waiting && named {
                    reel.previewing = true;
                    reel.rendered = true;
                    self.reel_enter();
                } else if reel.side == ReelSide::Before && reel.waiting && !named {
                    self.reel_enter();
                }
            }
            PreviewStatus::Exited => {
                // Exit preview, from outside: watching a preview that is gone ends.
                if reel.side == ReelSide::After && !reel.waiting {
                    reel.previewing = false;
                    self.reel_stop();
                    return;
                }
                reel.previewing = false;
                reel.rendered = false;
                if reel.side == ReelSide::Before && reel.waiting {
                    reel.due = Some((Instant::now() + REEL_PROGRAM_WAIT, ReelWait::Enter));
                }
            }
            PreviewStatus::Failed | PreviewStatus::Stopped => self.reel_stop(),
            _ => {}
        }
    }

    /// Past a segment's end, the next one plays; past the last, it stops,
    /// every change watched. Whether it did either.
    fn reel_follow_playhead(&mut self) -> bool {
        let position = self.state.position;
        let Some(reel) = self.reel.as_mut() else {
            return false;
        };
        if reel.waiting || reel.finished || position + 1e-3 < reel.segment_end {
            return false;
        }
        if reel.index + 1 >= reel.segments.len() {
            reel.finished = true;
            self.send(Command::Pause);
            return true;
        }
        reel.index += 1;
        self.reel_show();
        true
    }

    /// The program's in and out marks, and whether playback loops between
    /// them (the whole program without marks).
    pub fn marks(&self) -> (Option<f64>, Option<f64>, bool) {
        (self.mark_in, self.mark_out, self.looping)
    }

    pub fn set_mark_in(&mut self, at: Option<f64>) {
        self.wrapped = false;
        self.mark_in = at;
        if let (Some(start), Some(end)) = (self.mark_in, self.mark_out) {
            if end <= start {
                self.mark_out = None;
            }
        }
    }

    pub fn set_mark_out(&mut self, at: Option<f64>) {
        self.wrapped = false;
        self.mark_out = at;
        if let (Some(start), Some(end)) = (self.mark_in, self.mark_out) {
            if end <= start {
                self.mark_in = None;
            }
        }
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
        self.wrapped = false;
    }

    /// What Loop Playback plays over now ([`LoopSpan`]).
    pub fn loop_span(&self) -> LoopSpan {
        LoopSpan::new(
            self.mark_in,
            self.mark_out,
            self.state.program_length(),
            self.timeline().rate(),
        )
    }

    /// Looping, playback that passes the out mark goes back to the in mark
    /// (or the start), still playing; one that comes within a frame of the
    /// program's end goes back before kerfd stops there
    /// ([`LoopSpan::clock`]). A change reel plays its own segments. Whether
    /// it went back.
    fn keep_to_marks(&mut self) -> bool {
        if !self.looping || !self.state.playing || self.reel.is_some() {
            return false;
        }
        let span = self.loop_span();
        let Some(start) = span.clock(self.state.position, &mut self.wrapped) else {
            return false;
        };
        self.seek(start, false);
        true
    }

    /// Playback started or stopped (`was_playing`, before). Started, the
    /// picture is playback's, not the pointer's. Stopped at the loop's end
    /// with Loop Playback on (kerfd stops by itself at the program's end):
    /// back to the loop's start, and playing again, as Qt's
    /// `followPlaying` did ([`LoopSpan::stopped`]).
    fn follow_playing(&mut self, was_playing: bool) {
        if self.state.playing {
            self.pointer = None;
            return;
        }
        if !was_playing || !self.looping || self.reel.is_some() || !self.state.playable() {
            return;
        }
        let span = self.loop_span();
        if let Some(start) = span.stopped(self.state.position) {
            self.wrapped = true;
            self.seek(start, false);
            self.send(Command::Play);
        } else if self.wrapped && span.just_wrapped(self.state.position) {
            // kerfd's last clock at the end came just before it stopped
            // there, and has sent the playhead back already: play on from
            // there. A pause sent from here clears `wrapped` first.
            self.send(Command::Play);
        }
    }

    /// Archives conversation `id` (or brings it back). The one open stays
    /// open, as Qt's did: its menu then offers Unarchive.
    pub fn archive_conversation(&mut self, id: &str, archived: bool) {
        self.send(Command::Conversations(ConversationsRequest::Archive {
            id: id.to_owned(),
            archived,
        }));
    }

    pub fn delete_conversation(&mut self, id: &str) {
        self.send(Command::Conversations(ConversationsRequest::Delete {
            id: id.to_owned(),
        }));
        if self.conversation.as_deref() == Some(id) {
            self.open_conversation(None);
        }
    }

    pub fn rename_conversation(&mut self, id: &str, title: &str) {
        self.send(Command::Conversations(ConversationsRequest::Rename {
            id: id.to_owned(),
            title: title.to_owned(),
        }));
    }

    /// The kerfd this window was started with; None finds one as at launch.
    pub fn kerfd(&self) -> Option<&Path> {
        self.kerfd.as_deref()
    }

    /// Asks Kerf, in the conversation open (or a new one), about the
    /// sequence at the playhead, as the composer does.
    pub fn ask(&mut self, text: &str) -> bool {
        let at = self.state.position;
        self.ask_at(text, at)
    }

    /// As [`Session::ask`], with the playhead time the request was written
    /// at (a request queued while Kerf worked keeps its own time). It goes
    /// stamped with that time while the picture is open and the stamp is on.
    pub fn ask_at(&mut self, text: &str, at: f64) -> bool {
        let timeline = self.timeline();
        let focus = ask_focus(timeline, None, AskScope::Sequence, at);
        let label = format!(
            "Sequence at {}",
            crate::model::timecode(at, timeline.rate())
        );
        let stamp = self.stamps().then_some(at);
        self.ask_with(text, focus, &label, stamp)
    }

    /// Whether a request from the composer goes with the playhead time: the
    /// stamp is on and there is a picture to play.
    pub fn stamps(&self) -> bool {
        self.prefs.send_playhead && self.picture_open != Some(false)
    }

    /// Asks Kerf from the Ask popover: about the chosen clip (`Clip`) or the
    /// whole sequence around it (`Sequence`), else the sequence at the
    /// playhead; stamped at the start of what it is about.
    pub fn ask_about(&mut self, text: &str, scope: AskScope) -> bool {
        let timeline = self.timeline();
        let clip = self
            .selection
            .first()
            .and_then(|id| timeline.clip(id))
            .map(|(_, clip)| clip.id.clone());
        let at = self.state.position;
        let focus = ask_focus(timeline, clip.as_deref(), scope, at);
        let label = ask_label(timeline, clip.as_deref(), scope, at);
        let from = focus["fromFrame"].as_f64().unwrap_or(0.0) / timeline.rate();
        self.ask_with(text, focus, &label, Some(from))
    }

    /// Asks `text` again (Ask again, under an answer that failed or was
    /// stopped) about what it was asked about the first time from this
    /// window: the Ask popover's clip or scope, or the sequence frame the
    /// composer sent, with the same words in the work row and the same
    /// stamp ([`FocusBook`], Qt's `m_lastFocus` and `m_lastFocusLabel`).
    /// Words this window did not send go about the sequence at the
    /// playhead, as the composer's do.
    pub fn ask_again(&mut self, text: &str) -> bool {
        match self.asked.of(text).cloned() {
            Some(asked) => self.ask_as(text, asked),
            None => self.ask(text),
        }
    }

    /// Asks `text` about what `about` went about, with its words in the
    /// work row and its stamp: an Ask again that waited for the turn under
    /// way goes about what it was pressed about.
    pub fn ask_as(&mut self, text: &str, about: Focused) -> bool {
        self.ask_with(text, about.focus, &about.label, about.stamp)
    }

    /// What each request sent from this window was about.
    pub fn asked(&self) -> &FocusBook {
        &self.asked
    }

    /// Sends an agent turn about `focus`, stamped `stamp`; the work row says
    /// `label` until Kerf says what it is doing.
    fn ask_with(&mut self, text: &str, focus: Value, label: &str, stamp: Option<f64>) -> bool {
        let text = text.trim();
        if text.is_empty() || self.state.work.busy() {
            return false;
        }
        // What another window chose since, as the person last left it.
        self.prefs.refresh();
        // The focus as written ([`ask_focus`]); one that does not read as a
        // focus is refused, as the bridge refused it.
        let Ok(read) = serde_json::from_value::<Option<kerf_bridge::Focus>>(focus.clone()) else {
            return self.may_send("agent") && self.refused();
        };
        let command = Command::Agent {
            text: text.to_owned(),
            focus: read,
            conversation: self.conversation.clone(),
            at: stamp.filter(|at| at.is_finite()),
            permissions: Some(self.prefs.permissions.wire().to_owned()),
            settings: self.agent_settings(),
        };
        let sent = self.send(command);
        if sent {
            self.asked.remember(
                text,
                Focused {
                    focus,
                    label: label.to_owned(),
                    stamp,
                },
            );
            self.state.work.request = Some(text.to_owned());
            self.state.work.conversation = self.conversation.clone();
            self.state
                .work
                .started
                .get_or_insert_with(std::time::Instant::now);
            self.state.work.status = label.to_owned();
            self.state.work.answer.clear();
            self.state.work.steps.clear();
            self.state.next_steps.clear();
            self.live_changes.clear();
            self.activity.push("agent", "Request sent");
        }
        sent
    }

    pub fn stop(&mut self) {
        self.send(Command::Abort);
    }

    /// What the command registry needs to know to say what is available
    /// and to resolve a command (kerf-commands' `Context`).
    pub fn command_context(&self) -> Value {
        let timeline = self.timeline();
        let count = |kind| {
            timeline
                .tracks
                .iter()
                .filter(|track| track.kind == kind)
                .count()
        };
        let mut context = json!({
            "connected": self.state.connection == crate::store::Connection::Connected,
            "revision": self.revision(),
            "canUndo": self.state.snapshot.can_undo,
            "canRedo": self.state.snapshot.can_redo,
            "playheadFrame": timeline.frame_at(self.state.position),
            "videoTracks": count(crate::model::TrackKind::Video),
            "audioTracks": count(crate::model::TrackKind::Audio),
            "agentBusy": self.state.work.busy(),
            // Play waits on a render, never a pause ([`EditorState::play_waits`]).
            "previewRequired": self.state.play_waits(),
            "projectionStatus": self.state.snapshot.projection_status,
            "previewRenderable": self.state.preview_renderable(),
        });
        if let Some((track, clip)) = self.selection.first().and_then(|id| timeline.clip(id)) {
            let kind = match track.kind {
                crate::model::TrackKind::Video => "video",
                crate::model::TrackKind::Audio => "audio",
                crate::model::TrackKind::Subtitle => "subtitle",
            };
            context["selection"] = json!({
                "clip": clip.id,
                "track": kind,
                "startFrame": clip.start_frame,
                "durationFrames": clip.duration_frames,
            });
        }
        if let Some(collab) = &self.state.collab {
            context["collab"] = collab.clone();
            // Consolidate Media follows the media outside the project folder.
            context["collab"]["externalMedia"] = json!(self.state.media_status.external.len());
        }
        // The pinned card's proposal, else (none pinned, or one gone since)
        // the newest pending one.
        let snapshot = &self.state.snapshot;
        let proposal = self
            .pinned_proposal
            .as_deref()
            .and_then(|pinned| {
                snapshot
                    .proposals
                    .iter()
                    .find(|proposal| proposal.proposal_id == pinned && proposal.is_pending())
            })
            .or_else(|| snapshot.pending());
        if let Some(proposal) = proposal {
            context["proposal"] = proposal_context(proposal);
        }
        if let Some(source) = self
            .chosen_media
            .as_deref()
            .and_then(|id| timeline.source(id))
        {
            context["source"] = source_range(source, timeline.rate());
            context["sourceTargets"] = json!(crate::model::source_targets(
                timeline,
                source,
                self.source_targets.as_deref()
            ));
        }
        context
    }

    /// Runs an engine command of the registry: the request it resolves to
    /// goes to the bridge; why it cannot run is said.
    pub fn run_command(&mut self, id: &str, params: Value) {
        self.request_command(id, params);
    }

    /// [`Session::run_command`], true when its request went to the bridge.
    fn request_command(&mut self, id: &str, params: Value) -> bool {
        let invocation =
            json!({"command": id, "context": self.command_context(), "params": params});
        let answer = kerf_bridge::commands_run(&invocation);
        // The registry writes its request as JSON; one that does not read
        // as a command is refused, as the bridge refused it.
        if let Some(request) = answer.get("request").filter(|request| request.is_object()) {
            match serde_json::from_value::<Command>(request.clone()) {
                Ok(command) => {
                    let preview = matches!(command, Command::Preview { .. });
                    let sent = self.send(command);
                    if sent && preview {
                        self.activity.push("preview", "Rendering preview");
                    }
                    sent
                }
                Err(_) => {
                    let kind = request["type"].as_str().unwrap_or_default();
                    self.may_send(kind) && self.refused()
                }
            }
        } else {
            if let Some(error) = answer["error"].as_str() {
                self.state.notify(error, true);
            }
            false
        }
    }

    /// File ▸ Export…: the registry's `file.export` at the quality chosen
    /// in Export's menu, as File ▸ Export… names it (`full` for Master,
    /// `preview` for Draft), rendered where kerfd puts it. The toolbar's
    /// Export renders into deliveries/ instead ([`Session::export`]).
    /// The workspace calls it for `file.export`.
    pub fn export_file(&mut self) {
        let quality = self.export_quality;
        if self.request_command("file.export", json!({"quality": quality.file_wire()})) {
            self.activity
                .push("export", &format!("Export started ({})", quality.name()));
        }
    }

    /// The key for `provider`: the keychain's, else the one kept in the
    /// editor's settings where the keychain could not be reached, else its
    /// environment variable. The keychain is asked once for every window;
    /// the settings are this window's as last refreshed.
    pub fn api_key(&self, provider: Provider) -> Option<String> {
        let stored = provider.secret_name().and_then(|name| {
            let in_keychain = keychain()
                .entry(name)
                .or_insert_with(|| {
                    kerf_bridge::secret(&json!({"op": "get", "name": name}))["value"]
                        .as_str()
                        .map(str::to_owned)
                })
                .clone();
            in_keychain.or_else(|| self.prefs.kept_keys.get(name).cloned())
        });
        stored
            .filter(|key| !key.is_empty())
            .or_else(|| provider.key_env().and_then(|name| std::env::var(name).ok()))
            .filter(|key| !key.trim().is_empty())
    }

    /// Keeps `key` for `provider` (empty forgets it): in the system's
    /// keychain, or where that cannot be reached (Linux with no Secret
    /// Service, `KERF_SECRETS=off`), in the editor's settings file, as the
    /// answer says. Err only when it could be kept nowhere.
    pub fn set_api_key(&mut self, provider: Provider, key: &str) -> Result<KeyKept, String> {
        let Some(name) = provider.secret_name() else {
            return Ok(KeyKept::Keychain);
        };
        let key = key.trim();
        let request = if key.is_empty() {
            json!({"op": "delete", "name": name})
        } else {
            json!({"op": "set", "name": name, "value": key})
        };
        let answer = kerf_bridge::secret(&request);
        let kept = match answer["error"].as_str() {
            None => {
                // Kept in the keychain: no copy stays in the file.
                if self.prefs.kept_keys.remove(name).is_some() {
                    self.prefs.save();
                }
                KeyKept::Keychain
            }
            Some(error) => {
                if key.is_empty() {
                    self.prefs.kept_keys.remove(name);
                } else {
                    self.prefs.kept_keys.insert(name.to_owned(), key.to_owned());
                }
                self.prefs.save_checked().map_err(|written| {
                    format!("{error}; nor could it be written to the settings file: {written}")
                })?;
                KeyKept::SettingsFile
            }
        };
        // Every window's next request takes it.
        let in_keychain = (kept == KeyKept::Keychain && !key.is_empty()).then(|| key.to_owned());
        keychain().insert(name, in_keychain);
        Ok(kept)
    }

    pub fn save_prefs(&self) {
        self.prefs.save();
    }

    /// What a request runs on, as the agent command takes it: Kerf's
    /// hosted model (`cloud`), Claude Code and its model, or a provider
    /// with its key, model, base URL and (OpenAI) API shape. The MCP
    /// servers' switch and how many times their file was saved go with each.
    pub fn agent_settings(&self) -> AgentSettings {
        let prefs = &self.prefs;
        let provider = prefs.provider;
        let mut settings = AgentSettings {
            mcp_enabled: prefs.mcp_enabled,
            mcp_config_version: prefs.mcp_config_version,
            ..AgentSettings::default()
        };
        settings.provider = Some(match provider {
            Provider::Cloud => {
                // The service picks the model.
                settings.cloud = true;
                return settings;
            }
            Provider::Anthropic => ChatProvider::Anthropic,
            Provider::Openai => ChatProvider::Openai,
            Provider::ClaudeCode => ChatProvider::ClaudeCode,
        });
        if !prefs.model().is_empty() {
            settings.model = Some(prefs.model().to_owned());
        }
        if !prefs.base_url().is_empty() {
            settings.base_url = Some(prefs.base_url().to_owned());
        }
        if provider == Provider::Openai {
            settings.api_mode = Some(match prefs.openai_api_mode {
                crate::prefs::ApiMode::Chat => kerf_bridge::OpenAiApiMode::Chat,
                crate::prefs::ApiMode::Responses => kerf_bridge::OpenAiApiMode::Responses,
            });
        }
        settings.api_key = self.api_key(provider);
        settings
    }

    /// Why the connection chosen cannot take requests, as the command
    /// registry's `aiAccess` names it: `noKey` for a provider with no key
    /// kept and none in the environment, Kerf's account standing (`cloud`,
    /// the last `cloud` event's `access`, `unavailable` until one is
    /// heard). None when it can.
    pub fn ai_access(&self, cloud: Option<&Value>) -> Option<String> {
        match self.prefs.provider {
            Provider::ClaudeCode => None,
            Provider::Cloud => {
                let access = cloud
                    .and_then(|cloud| cloud["access"].as_str())
                    .unwrap_or("unavailable");
                (access != "ready").then(|| access.to_owned())
            }
            provider => self.api_key(provider).is_none().then(|| "noKey".into()),
        }
    }

    /// Each registry command's id with why it is unavailable now, or null
    /// when it is (`kerf_bridge::commands_states`). `cloud` is the last
    /// `cloud` event heard, for Ask's reason.
    pub fn command_states(&self, cloud: Option<&Value>) -> Value {
        let mut context = self.command_context();
        if let Some(access) = self.ai_access(cloud) {
            context["aiAccess"] = json!(access);
        }
        kerf_bridge::commands_states(&context)
    }

    /// Asks the bridge for what has to be known again: Kerf's account
    /// ([`Command::CloudRefresh`]), the person's models
    /// ([`Command::Models`]). False without a project.
    pub fn ask_bridge(&mut self, command: Command) -> bool {
        self.bridge.is_some() && self.send(command)
    }

    /// Signs in to Kerf with `token` (`cloud_sign_in`), or out with None
    /// (`cloud_sign_out`); a `cloud` event answers. False without a project.
    pub fn cloud_sign(&mut self, token: Option<&str>) -> bool {
        if self.bridge.is_none() {
            return false;
        }
        match token.map(str::trim).filter(|token| !token.is_empty()) {
            Some(token) => self.send(Command::CloudSignIn {
                token: token.to_owned(),
            }),
            None => self.send(Command::CloudSignOut),
        }
    }

    /// Forgets what the keychain answered, so the next request in any
    /// window asks it again (a key changed outside Kerf).
    pub fn forget_keys(&self) {
        keychain().clear();
    }
}

/// Where Loop Playback plays (Qt's `loopStart` and `loopEnd`): from the in
/// mark, else the start, to the out mark held to the program's length (an
/// out mark past a program that got shorter ends at its end), else the
/// program's end. `length` is the program's seconds, `fps` its rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopSpan {
    pub start: f64,
    pub end: f64,
    pub length: f64,
    pub fps: f64,
}

impl LoopSpan {
    pub fn new(mark_in: Option<f64>, mark_out: Option<f64>, length: f64, fps: f64) -> Self {
        let length = if length.is_finite() {
            length.max(0.0)
        } else {
            0.0
        };
        let start = mark_in.unwrap_or(0.0).clamp(0.0, length);
        let end = mark_out.map_or(length, |out| out.min(length)).max(0.0);
        let fps = if fps.is_finite() { fps.max(1.0) } else { 1.0 };
        Self {
            start,
            end,
            length,
            fps,
        }
    }

    fn frame(&self) -> f64 {
        1.0 / self.fps
    }

    /// Whether there is a frame at least between its start and its end.
    pub fn loops(&self) -> bool {
        self.end - self.start >= self.frame()
    }

    /// A clock at `position` while playing: the time to go back to now,
    /// if any. Past an out mark short of the program's end (by half a
    /// frame, Qt's `followPlayhead`) it goes back; within a frame of the
    /// program's end it goes back before kerfd stops there (the stop itself
    /// is [`LoopSpan::stopped`]). `wrapped` says it has gone back since the
    /// playhead last played short of the end, so the clocks still on their
    /// way from before that seek do not send it back again; one short of
    /// the end clears it.
    pub fn clock(&self, position: f64, wrapped: &mut bool) -> Option<f64> {
        let frame = self.frame();
        let at_program_end = self.end >= self.length - frame;
        let reach = if at_program_end {
            self.end - frame
        } else {
            self.end - frame / 2.0
        };
        if position < reach {
            *wrapped = false;
            return None;
        }
        if *wrapped || !self.loops() {
            return None;
        }
        *wrapped = true;
        Some(self.start)
    }

    /// Whether the playhead at `position` is where Loop Playback just sent
    /// it back to: within the larger of two frames and [`LOOP_SLACK`] after
    /// its start (a frame before it too).
    pub fn just_wrapped(&self, position: f64) -> bool {
        let slack = (2.0 * self.frame()).max(LOOP_SLACK);
        self.loops() && position >= self.start - self.frame() && position <= self.start + slack
    }

    /// Playback stopped with the playhead at `position`: the time to go
    /// back to and play again from, when it stopped within the larger of
    /// two frames and [`LOOP_SLACK`] of the loop's end, as Qt's
    /// `followPlaying` had it. kerfd stops by itself at the program's end,
    /// its last clock before it up to 50 ms short.
    pub fn stopped(&self, position: f64) -> Option<f64> {
        let slack = (2.0 * self.frame()).max(LOOP_SLACK);
        (self.loops() && position >= self.end - slack).then_some(self.start)
    }
}

/// The least a stop may fall short of the loop's end and still loop
/// (Qt's 0.1 s), in seconds.
pub const LOOP_SLACK: f64 = 0.1;

/// How far ahead of the frame shown, in seconds, the pointer's seek still
/// decodes on to the frame exactly rather than showing the nearest picture
/// at once (Qt's 0.40 s in `scrubFromShownFrame`).
pub const SCRUB_AHEAD: f64 = 0.4;

/// Where the pointer sent the picture while it moves (skimming the picture
/// or the lanes, dragging the playhead), and when the frame there is asked
/// for exactly: [`SCRUB_SETTLE`] after its last rough seek, None once that
/// seek has gone (or the seek was exact). Until the frame there comes, a
/// frame farther from it than the one shown is not shown
/// ([`nearer_frame`]), so a late rough picture does not jump back over a
/// nearer one (Qt's `m_pointerOwnsPlayhead`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSeek {
    pub target: f64,
    pub settle: Option<Instant>,
}

/// Whether the pointer's seek to `target` is rough (`scrub`), the frame
/// shown being at `shown` (None before any), the sequence at `fps`: when
/// it lies more than a frame behind the frame shown or more than
/// [`SCRUB_AHEAD`] ahead of it, as Qt's `scrubFromShownFrame` had it. Just
/// ahead, decoding on to it exactly costs little.
pub fn scrubs_from(shown: Option<f64>, target: f64, fps: f64) -> bool {
    let Some(shown) = shown else {
        return true;
    };
    let delta = target - shown;
    delta < -(1.0 / fps.max(1.0)) || delta > SCRUB_AHEAD
}

/// Whether a frame at `frame` may take the place of the one shown at
/// `shown` (None: nothing shown) while the pointer has sent the picture to
/// `target`: when it is no farther from it, within half a frame at `fps`
/// (Qt's `nearerScrubFrame`).
pub fn nearer_frame(shown: Option<f64>, frame: f64, target: f64, fps: f64) -> bool {
    let Some(shown) = shown else {
        return true;
    };
    let slack = 0.5 / fps.max(1.0);
    (frame - target).abs() <= (shown - target).abs() + slack
}

/// Whether `command` goes to the bridge while kerfd is connecting or gone:
/// what the bridge answers by itself (the conversations kept beside the
/// project, Git, Kerf's account and models, the media's own analyses, the
/// checks, a making stopped, a preview stopped or left, the volume and the
/// tracks heard alone), Reconnect and Stop, and a request to Kerf (the
/// composer says whether it can ask). Edits, renders and the rest of what
/// kerfd answers, and playback and seeks, wait for it: Qt's `sendSimple`
/// sent nothing but Reconnect and Stop.
pub fn sent_unconnected(kind: &str) -> bool {
    kind.starts_with("collab_")
        || kind.starts_with("cloud_")
        || matches!(
            kind,
            "reconnect"
                | "abort"
                | "conversations"
                | "agent"
                | "agent_versions"
                | "models"
                | "media_info"
                | "media_index"
                | "media_make_cancel"
                | "check"
                | "loudness"
                | "scope_signal"
                | "effect_previews"
                | "preset_library"
                | "preview_stop"
                | "preview_exit"
                | "volume"
                | "solo"
        )
}

/// What a request from the Ask popover is about: the chosen clip itself,
/// or the whole sequence (around the chosen clip, or at the playhead).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AskScope {
    Clip,
    #[default]
    Sequence,
}

impl AskScope {
    pub fn wire(self) -> &'static str {
        match self {
            AskScope::Clip => "clip",
            AskScope::Sequence => "sequence",
        }
    }
}

/// A request's `focus`: with `clip` (one on `timeline`), its span, as the
/// clip (`Clip`) or the sequence around it (`Sequence`); without, the
/// sequence at `playhead`, which `Clip` cannot be.
pub fn ask_focus(timeline: &Timeline, clip: Option<&str>, scope: AskScope, playhead: f64) -> Value {
    if let Some((_, clip)) = clip.and_then(|id| timeline.clip(id)) {
        return json!({
            "scope": scope.wire(),
            "clip": clip.id,
            "fromFrame": clip.start_frame,
            "toFrame": clip.end_frame(),
        });
    }
    let frame = timeline.frame_at(playhead.max(0.0));
    json!({"scope": "sequence", "fromFrame": frame, "toFrame": frame})
}

/// What the work row says a request is about until Kerf says what it is
/// doing: the clip's name, "Sequence around <clip>", or "Sequence at <tc>".
pub fn ask_label(
    timeline: &Timeline,
    clip: Option<&str>,
    scope: AskScope,
    playhead: f64,
) -> String {
    match clip.and_then(|id| timeline.clip(id)) {
        Some((_, clip)) if scope == AskScope::Clip => clip.label().to_owned(),
        Some((_, clip)) => format!("Sequence around {}", clip.label()),
        None => format!(
            "Sequence at {}",
            crate::model::timecode(playhead.max(0.0), timeline.rate())
        ),
    }
}

/// What a request went about: its `focus` as sent, the words the work row
/// said until Kerf said what it was doing (Qt's `m_lastFocusLabel`), and
/// the playhead time it was stamped with.
#[derive(Clone, Debug, PartialEq)]
pub struct Focused {
    pub focus: Value,
    pub label: String,
    pub stamp: Option<f64>,
}

/// What each request sent from a window was about, by its words, the
/// newest kept last (Qt kept the last request's, `m_lastFocus`): Ask again
/// under a turn that failed or was stopped asks about what that turn's
/// request was about. The newest [`FocusBook::KEPT`] are kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusBook {
    sent: VecDeque<(String, Focused)>,
}

impl FocusBook {
    pub const KEPT: usize = 32;

    /// `text` went about `focused`.
    pub fn remember(&mut self, text: &str, focused: Focused) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.sent.retain(|(sent, _)| sent != text);
        self.sent.push_back((text.to_owned(), focused));
        while self.sent.len() > Self::KEPT {
            self.sent.pop_front();
        }
    }

    /// What `text` went about when it was last sent, if it was.
    pub fn of(&self, text: &str) -> Option<&Focused> {
        let text = text.trim();
        self.sent
            .iter()
            .rev()
            .find(|(sent, _)| sent == text)
            .map(|(_, focused)| focused)
    }

    /// What the last request sent went about.
    pub fn last(&self) -> Option<&Focused> {
        self.sent.back().map(|(_, focused)| focused)
    }

    pub fn len(&self) -> usize {
        self.sent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }
}

/// The stretch of the program a proposal's preview renders, as Qt's
/// `ChatWiring::preview` asked for it: from the earliest clip or
/// transition `proposed` changes against `program` to the latest
/// (`ProposalModel`'s range), as (start, duration) in seconds, the
/// duration held to one frame at least and [`REEL_MOST_RENDERED`] at most.
/// It never runs past `proposed`'s latest clip end: kerfd renders a
/// proposal no further than its own length, and one that takes out, trims
/// or pulls in the last clips ends sooner than the program, though what it
/// changes reaches the program's old end. None when it changes nothing
/// there, or nothing of what it changes lies within the proposal, a frame
/// at least (the whole program renders).
pub fn preview_span(program: &Timeline, proposed: &Timeline) -> Option<(f64, f64)> {
    let (start, end) = crate::changes::clip_changes(program, proposed)
        .iter()
        .map(crate::changes::ClipChange::span)
        .filter(|(start, end)| start.is_finite() && end.is_finite())
        .reduce(|(low, high), (start, end)| (low.min(start), high.max(end)))?;
    let start = start.max(0.0);
    let frame = 1.0 / program.rate().max(1.0);
    let last = timeline_end(proposed);
    if last - start < frame {
        return None;
    }
    Some((
        start,
        (end.min(last) - start).clamp(frame, REEL_MOST_RENDERED),
    ))
}

/// The `preview` request for `proposal` (any of its versions' ids; None
/// renders the program): over the stretch it changes ([`preview_span`]),
/// measured on the timeline of the version named when kerfd drew it, else
/// of the proposal's newest version that has one; the whole of it when
/// none is known, or nothing it changes lies within the proposal.
pub fn preview_request(snapshot: &crate::model::Snapshot, proposal: Option<&str>) -> Command {
    let Some(id) = proposal else {
        return Command::Preview {
            proposal_id: None,
            start: None,
            duration: None,
        };
    };
    let named = snapshot.proposals.iter().find(|known| {
        known.proposal_id == id
            || known
                .versions
                .iter()
                .any(|version| version.proposal_id == id)
    });
    let proposed = named.and_then(|proposal| {
        snapshot.timeline_of(proposal, Some(id)).or_else(|| {
            proposal
                .versions
                .iter()
                .rev()
                .find_map(|version| snapshot.proposal_timelines.get(&version.proposal_id))
        })
    });
    let span = proposed.and_then(|proposed| preview_span(&snapshot.timeline, proposed));
    Command::Preview {
        proposal_id: Some(id.to_owned()),
        start: span.map(|(start, _)| start),
        duration: span.map(|(_, duration)| duration),
    }
}

/// "Cutting · 3 clips": what a first cut's turn says as clips are placed.
pub fn cutting_status(placed: usize) -> String {
    if placed == 1 {
        "Cutting · 1 clip".to_owned()
    } else {
        format!("Cutting · {placed} clips")
    }
}

/// The keys of `changes` ("clip@end"), and where the newest of them not
/// among `seen` is to be looked at: where a trim cuts, else where the clip
/// now starts (the latest start among the new ones). Removed clips are
/// nothing to look at.
pub fn newest_change(
    changes: &[crate::changes::ClipChange],
    seen: &HashSet<String>,
) -> (HashSet<String>, Option<f64>) {
    let key =
        |change: &crate::changes::ClipChange| format!("{}@{:.4}", change.clip, change.new_end);
    let keys: HashSet<String> = changes.iter().map(key).collect();
    let newest = changes
        .iter()
        .filter(|change| !change.removed && !seen.contains(&key(change)))
        .max_by(|a, b| a.new_start.total_cmp(&b.new_start))
        .map(|change| {
            if change.trimmed && !change.moved {
                change.new_end
            } else {
                change.new_start
            }
        });
    (keys, newest)
}

/// Where a timeline's last clip ends, in seconds.
fn timeline_end(timeline: &Timeline) -> f64 {
    timeline
        .tracks
        .iter()
        .flat_map(|track| track.clips.iter())
        .map(|clip| clip.end())
        .fold(0.0, f64::max)
}

/// What the command registry is told of the media chosen in the bin
/// (kerf-commands' `SourceRange`): the whole of it, from its first frame,
/// in frames at `rate`; a still, which has no length of its own, for
/// [`crate::model::STILL_SECONDS`], as Qt's `commandContext` had it.
pub fn source_range(source: &crate::model::Source, rate: f64) -> Value {
    let frames = (source.placed_seconds() * rate).round() as i64;
    json!({"media": source.id, "inFrame": 0, "durationFrames": frames})
}

/// What the command registry is told of the proposal its commands act on
/// (kerf-commands' `ProposalState`): its id, and `blocked: "stale"` once
/// the edit has changed under it, so Accept says to bring it forward and
/// Bring Proposal Forward can run. [`Session::command_context`] sends it.
pub fn proposal_context(proposal: &crate::model::Proposal) -> Value {
    json!({
        "id": proposal.proposal_id,
        "blocked": proposal.is_stale().then_some("stale"),
    })
}

/// How many lines Activity keeps, and how many it shows.
pub const ACTIVITY_KEPT: usize = 80;
pub const ACTIVITY_SHOWN: usize = 8;

/// What the window and Kerf did, a line each, oldest first: imports,
/// exports, previews, analyses, a link to another revision, the project
/// connecting. The newest [`ACTIVITY_KEPT`] are kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityLog {
    lines: VecDeque<(String, String)>,
}

impl ActivityLog {
    /// Keeps `text` under `kind` ("export", "import", …); empty text is not kept.
    pub fn push(&mut self, kind: &str, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.lines.push_back((kind.to_owned(), text.to_owned()));
        while self.lines.len() > ACTIVITY_KEPT {
            self.lines.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The newest `most` lines, oldest first, as (kind, text).
    pub fn latest(&self, most: usize) -> impl Iterator<Item = (&str, &str)> {
        self.lines
            .iter()
            .skip(self.lines.len().saturating_sub(most))
            .map(|(kind, text)| (kind.as_str(), text.as_str()))
    }
}

/// The Activity line a bridge event leaves, if any: an export that ended
/// (the file it made, or how it ended), an analysis made in the
/// background, why the picture needs a rendered preview (the bridge's
/// reason, a render that failed among them, else "Native preview
/// unavailable", as Qt kept it), and what Kerf did: each tool call that
/// ended, each status line, any other note (at most 120 characters of
/// each).
pub fn activity_of(event: &Event) -> Option<(&'static str, String)> {
    match event {
        Event::Agent(update) => {
            let (kind, text) = match update {
                AgentUpdate::Delta { text } => ("delta", text),
                AgentUpdate::Status { text } => ("status", text),
                AgentUpdate::Tool { text, .. } => ("tool", text),
                AgentUpdate::Done { text } => ("done", text),
                AgentUpdate::Other { kind, text } => (kind.as_str(), text),
            };
            let text: String = text.trim().chars().take(120).collect();
            if text.is_empty() {
                return None;
            }
            match (kind, update) {
                ("delta" | "done", _) => None,
                (
                    "tool",
                    AgentUpdate::Tool {
                        state: kerf_bridge::event::ToolState::Running,
                        ..
                    },
                ) => None,
                ("tool", _) => Some(("tool", text)),
                ("status", _) => Some(("status", text)),
                _ => Some(("agent", text)),
            }
        }
        Event::Export(export) => {
            let (status, path, message) = match export {
                Export::Render(render) => (
                    render.status,
                    render.path.as_deref(),
                    render.message.as_deref(),
                ),
                Export::Captions(captions) => (
                    captions.status,
                    captions.path.as_deref(),
                    captions.message.as_deref(),
                ),
            };
            let path = path.unwrap_or_default();
            match status {
                ExportStatus::Completed if !path.is_empty() => Some(("export", path.to_owned())),
                ExportStatus::Failed => Some((
                    "export",
                    format!(
                        "Export failed: {}",
                        message.unwrap_or("it could not render")
                    ),
                )),
                ExportStatus::Cancelled => Some(("export", "Export cancelled".to_owned())),
                _ => None,
            }
        }
        Event::Analysis { message } => Some(message.as_str())
            .filter(|message| !message.trim().is_empty())
            .map(|message| ("analysis", message.to_owned())),
        // A render that failed says why next (`required`, with the
        // bridge's reason), which is the line kept, as Qt kept it.
        Event::Preview(preview)
            if matches!(
                preview.status,
                PreviewStatus::Unavailable | PreviewStatus::Required
            ) =>
        {
            Some((
                "preview",
                preview
                    .message
                    .as_deref()
                    .filter(|message| !message.trim().is_empty())
                    .unwrap_or("Native preview unavailable")
                    .to_owned(),
            ))
        }
        _ => None,
    }
}

/// The Activity line a command sent leaves, if any: a making a person said
/// yes to ("Making m4 from its steps in the document", "Making 3 takes of
/// m4 …"), steps carried into the document, a take chosen ("Using take 1
/// of m5"), a file put behind a media ("Replacing m2 with b.mov").
pub fn activity_of_command(
    command: &Command,
    timeline: &Timeline,
) -> Option<(&'static str, String)> {
    match command {
        Command::MediaMake {
            media,
            confirmed: true,
            rolls,
        } => {
            let from = timeline
                .source(media)
                .filter(|source| source.steps_in_document())
                .map(|_| " from its steps in the document")
                .unwrap_or_default();
            let rolls = rolls.unwrap_or(1);
            Some((
                "import",
                if rolls > 1 {
                    format!("Making {rolls} takes of {media}{from}")
                } else {
                    format!("Making {media}{from}")
                },
            ))
        }
        Command::MediaCarry {
            media,
            confirmed: true,
        } => Some((
            "import",
            format!("Carrying the steps of {media} into the document"),
        )),
        Command::MediaReplace { media, path } => {
            let take = timeline.source(media).and_then(|source| {
                source
                    .takes
                    .iter()
                    .find(|take| !take.path.is_empty() && take.path == *path)
            });
            Some((
                "import",
                match take {
                    Some(take) => format!("Using take {} of {media}", take.number),
                    None => format!("Replacing {media} with {}", crate::names::file_name(path)),
                },
            ))
        }
        _ => None,
    }
}

/// A playback picture (RGBA, `width` × `height`) as an image GPUI draws,
/// whose textures take BGRA.
pub fn render_image(pixels: &[u8], width: u32, height: u32) -> Option<Arc<RenderImage>> {
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if width == 0 || height == 0 || pixels.len() != expected {
        return None;
    }
    let mut bgra = pixels.to_vec();
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// The kerfd beside the editor, else the checkout's build, else the PATH's.
pub fn discover_kerfd() -> PathBuf {
    if let Some(path) = std::env::var_os("KERFD_BIN").filter(|path| !path.is_empty()) {
        return path.into();
    }
    let name = if cfg!(windows) { "kerfd.exe" } else { "kerfd" };
    if let Some(beside) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|folder| folder.join(name)))
        .filter(|path| path.is_file())
    {
        return beside;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for profile in ["debug", "release"] {
        let built = root.join("target").join(profile).join(name);
        if built.is_file() {
            return built;
        }
    }
    PathBuf::from(name)
}

/// The kerfd the editor was started with (its argument, or `KERFD_BIN`).
struct StartedKerfd(PathBuf);

impl Global for StartedKerfd {}

/// Keeps the kerfd the editor was started with, so every window opened
/// later (a project from the launcher, New and Open Project, a link) uses
/// it too. `main` calls it once, before the first window.
pub fn set_kerfd(kerfd: Option<PathBuf>, cx: &mut App) {
    if let Some(kerfd) = kerfd.filter(|kerfd| !kerfd.as_os_str().is_empty()) {
        cx.set_global(StartedKerfd(kerfd));
    }
}

/// The kerfd the editor was started with, if one was given.
pub fn started_kerfd(cx: &App) -> Option<PathBuf> {
    cx.try_global::<StartedKerfd>().map(|kerfd| kerfd.0.clone())
}

/// The last `cloud` event any window heard: the person's Kerf account.
struct Cloud(Value);

impl Global for Cloud {}

/// What was last heard of the person's Kerf account (`cloud {access,
/// account?, message?}`); None before anything is.
pub fn cloud(cx: &App) -> Option<&Value> {
    cx.try_global::<Cloud>().map(|cloud| &cloud.0)
}

/// Keeps a `cloud` event for every window.
pub fn hear_cloud(event: &Value, cx: &mut App) {
    cx.set_global(Cloud(event.clone()));
}

/// At most `most` bytes of `text`, cut at a character boundary.
fn truncate(text: &str, most: usize) -> &str {
    if text.len() <= most {
        return text;
    }
    let mut end = most;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
