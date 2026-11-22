//! A shot's graph, over the window ([`ShotGraph`]): what it holds and what
//! the bridge says of it, the camera's moves, a change sent as one edit,
//! the node chosen, its menus, what opens over it, and the panel drawn as a
//! whole; and the graph the editor starts on (`--graph`).

use editor_session::session::{BridgeEvent, Session};
use editor_state::store::Making;
use editor_ui::motion;
use editor_ui::theme::{self, motion as scene, radius};
use editor_ui::ui;
use gpui::{
    anchored, deferred, div, prelude::*, px, size, AnimationExt, AnyElement, App, Bounds, Context,
    DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Point, SharedString, Subscription, Window,
};
use kerf_bridge::event::{Recipe as RecipeEvent, TakeRead};
use kerf_bridge::{Command, Event, RecipeEdit};
use serde_json::Value;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::{bind_close, bind_keys, Close, MakePlan, Recipe, Workflow, CONTEXT, HEAD};
use drag::Drag;
use fields::Inspecting;
use footage::{PointStage, Pointing};
use layout::{graph_size, load_places, INSPECTOR};
use pictures::{Still, STILL_WORKERS};
use signal::Drawing;
use takes::Holding;

mod canvas;
mod cards;
mod drag;
mod fields;
mod footage;
mod head;
mod inspector;
mod layout;
mod paint;
mod pictures;
mod signal;
mod takes;
mod workflow;

pub(super) use canvas::bind_graph_keys;
pub use fields::{another_seed, one_line, part_number};
pub use footage::{fitted_in, point_on};
pub use layout::{
    arrange, card_height, card_rect, card_size, draws_curve, extent, in_port, nearest_toward,
    out_port, placed_over, placement_key, settle, workflow_key, zoom_about, Camera, Toward,
    ZOOM_LEAST, ZOOM_MOST, ZOOM_READ, ZOOM_STEP,
};
pub use pictures::{output_within, PictureQueue};
pub use signal::{draw_signal, signal_keys, signal_line, signal_under};
pub use takes::{reading_lines, take_tip};

/// What opens over the graph and gives it back as it was: a making's plan,
/// Cast and Styles, Settings at its Models.
enum Over {
    Plan(Entity<MakePlan>),
    Cast(Entity<editor_launcher::cast::CastView>),
    Settings(Entity<editor_launcher::settings::SettingsView>),
}

/// Which menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuKind {
    Several,
    AddStep,
    Port,
    Pass,
    Sound,
    Choice,
}

impl MenuKind {
    /// How wide its menu is.
    fn width(self) -> f32 {
        match self {
            MenuKind::Port => 420.0,
            MenuKind::Several | MenuKind::Pass => 220.0,
            MenuKind::AddStep | MenuKind::Sound | MenuKind::Choice => 240.0,
        }
    }
}

struct GraphMenu {
    kind: MenuKind,
    menu: Entity<ui::Menu>,
    at: Point<Pixels>,
    _closed: Subscription,
}

/// A shot's graph, over the window: its steps and what they read, wired
/// left to right on a dotted canvas, a chosen node's settings at the right,
/// and its making and takes. A model run with kerf-comfy opens into its
/// ComfyUI workflow, on the same canvas.
pub struct ShotGraph {
    session: Entity<Session>,
    media: SharedString,
    focus_handle: FocusHandle,
    /// The canvas's own, for its keys.
    canvas_focus: FocusHandle,
    recipe: Option<Recipe>,
    /// The model's workflow shown in place of the shot's graph.
    workflow: Option<Workflow>,
    /// The line of the model whose workflow is asked for or shown; 0 for none.
    workflow_line: usize,
    /// Why the graph could not be read, before any was.
    refused: Option<String>,
    /// Why the last change was refused.
    said: Option<String>,
    /// The revision the graph was last asked for at.
    asked: Option<u64>,
    /// What the shot's cards are kept under, by its project and media.
    shot_key: String,
    /// Where the person put a card, by its node, over where it was laid out,
    /// in what is shown (the shot's graph or its model's workflow).
    placed: HashMap<String, (f32, f32)>,
    camera: Camera,
    /// The canvas looks at all of the graph, and goes on doing so as the
    /// canvas's size or the nodes change, until it is panned or zoomed.
    fitted: bool,
    /// Fit asked for all of it, however small.
    whole: bool,
    /// The canvas's size in the window as it is now
    /// ([`ShotGraph::follow_window`]).
    room: (f32, f32),
    drag: Drag,
    hovered_port: Option<String>,
    selected: Option<String>,
    inspecting: Option<Inspecting>,
    /// The field that had the keys when they were last followed
    /// ([`ShotGraph::follow_typing`]): left since, what it says is set.
    typing_in: Option<String>,
    menu: Option<GraphMenu>,
    /// Each take's reading beside its guide, the whole of what was said.
    readings: HashMap<u32, TakeRead>,
    best: Option<u32>,
    reading: Holding,
    pointing: Pointing,
    drawing: Option<Drawing>,
    /// Each file's picture, under the key of the file as it was read.
    stills: HashMap<PathBuf, (String, Still)>,
    /// The files whose pictures are asked for and not yet being made.
    queue: PictureQueue,
    over: Option<Over>,
    _over: Option<Subscription>,
    /// What `--graph-node` chose, once the graph is read.
    pending_node: Option<String>,
    /// Where the canvas, a signal's curve and a picture pointed on were last drawn.
    canvas_bounds: Rc<Cell<Bounds<Pixels>>>,
    curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The inspector's column, for its fades.
    inspector_scroll: gpui::ScrollHandle,
    point_bounds: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl ShotGraph {
    pub fn new(
        session: Entity<Session>,
        media: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        bind_close(cx);
        bind_graph_keys(cx);
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &session,
                window,
                |this, _, event: &BridgeEvent, window, cx| this.heard(&event.0, window, cx),
            ),
            // Its pictures go out of the window's atlas with it: each graph
            // opened makes its own again.
            cx.on_release(|graph, cx| {
                for image in graph.let_go_of_pictures() {
                    cx.drop_image(image, None);
                }
            }),
            // The canvas's room is the window's: as the window's size
            // changes, so does the room, and a graph fitted in it settles.
            cx.observe_window_bounds(window, |graph, window, cx| {
                if graph.follow_window(window) {
                    cx.notify();
                }
            }),
        ];
        let shot_key = placement_key(session.read(cx).project.as_deref(), &media);
        let placed = load_places(&session, &shot_key, cx);
        let pending_node = start_node(&media);
        let canvas_focus = cx.focus_handle();
        let mut graph = Self {
            session,
            media,
            focus_handle: cx.focus_handle(),
            canvas_focus,
            recipe: None,
            workflow: None,
            workflow_line: 0,
            refused: None,
            said: None,
            asked: None,
            shot_key,
            placed,
            camera: Camera::default(),
            fitted: true,
            whole: false,
            room: (0.0, 0.0),
            drag: Drag::None,
            hovered_port: None,
            selected: None,
            inspecting: None,
            typing_in: None,
            menu: None,
            readings: HashMap::new(),
            best: None,
            reading: Holding::Idle,
            pointing: Pointing::default(),
            drawing: None,
            stills: HashMap::new(),
            queue: PictureQueue::new(STILL_WORKERS),
            over: None,
            _over: None,
            pending_node,
            canvas_bounds: Rc::default(),
            curve_bounds: Rc::default(),
            inspector_scroll: gpui::ScrollHandle::new(),
            point_bounds: Rc::default(),
            _subscriptions: subscriptions,
        };
        graph.follow_window(window);
        graph.request(cx);
        graph
    }

    fn request(&mut self, cx: &mut Context<Self>) {
        let media = self.media.to_string();
        let revision = self.session.update(cx, |session, _| {
            session.send(Command::RecipeGet { media });
            session.revision()
        });
        self.asked = Some(revision);
    }

    fn heard(&mut self, event: &Event, window: &mut Window, cx: &mut Context<Self>) {
        // What is said of this shot's media; a recipe and a workflow say it among their fields.
        let media = match event {
            Event::Recipe(recipe) => recipe.graph.get("media").and_then(Value::as_str),
            Event::Workflow(fields) => fields.get("media").and_then(Value::as_str),
            Event::Error(error) => error.media.as_deref(),
            Event::TakeRead(read) => Some(read.media.as_str()),
            Event::TakesRead(read) => Some(read.media.as_str()),
            Event::MediaMade { media, .. } | Event::MediaMakeCancelled { media } => {
                Some(media.as_str())
            }
            _ => None,
        };
        let ours = media == Some(self.media.as_str());
        match event {
            Event::Recipe(recipe) if ours => self.set_recipe(recipe, window, cx),
            Event::Workflow(fields) if ours => {
                let line = fields.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
                if line > 0 && line == self.workflow_line {
                    self.set_workflow(Workflow::read(fields), window, cx);
                }
            }
            Event::Error(error) => {
                let command = error.command.as_deref().unwrap_or_default();
                let message = error.message.clone();
                match command {
                    "workflow" if ours => {
                        self.said = Some(message);
                        // Asked for and refused: the shot's graph stays, and its
                        // model's Open workflow asks again.
                        if self.workflow.is_none() {
                            self.workflow_line = 0;
                        }
                    }
                    "recipe" if ours => {
                        if self.recipe.is_some() {
                            self.said = Some(message);
                        } else {
                            self.refused = Some(message);
                        }
                    }
                    "takes_read" if ours => self.reading = Holding::Failed(message),
                    _ if command.starts_with("shot_") => {
                        self.followed(error.clip.as_deref(), error.of, message, true)
                    }
                    "media_make" if ours => self.request(cx),
                    _ => return,
                }
            }
            Event::TakeRead(read) if ours => {
                self.readings.insert(read.take, read.clone());
            }
            Event::TakesRead(read) if ours => {
                self.best = read.best.filter(|best| *best > 0);
                self.reading = Holding::Read;
            }
            // A clip read from footage: the card that asked knows its clip.
            Event::ShotPicture { clip, of, path, .. } => {
                let pointing = &mut self.pointing;
                let asked = *clip == pointing.clip
                    && pointing.of == Some(*of)
                    && pointing.stage == PointStage::Drawing;
                if !asked {
                    return;
                }
                pointing.picture = Some(path.clone());
                pointing.stage = PointStage::Pointing;
                self.want_pictures(window, cx);
            }
            Event::ShotFollowed { clip, of, says, .. } => {
                self.followed(Some(clip.as_str()), Some(*of), says.clone(), false);
            }
            // Its takes are other than they were.
            Event::MediaMade { .. } | Event::MediaMakeCancelled { .. } if ours => self.request(cx),
            // The project changed: a signal's keys, a clip's way, a take chosen.
            Event::State { snapshot } => {
                let revision = Some(snapshot.revision);
                if revision != self.asked {
                    self.request(cx);
                    if self.workflow.is_some() {
                        self.request_workflow(cx);
                    }
                }
            }
            Event::Models { .. } | Event::Cast(_) => self.request(cx),
            Event::Connection { .. } if self.recipe.is_none() => self.request(cx),
            _ => return,
        }
        // What it said may have made the chosen node's fields again, and
        // left the one being typed in.
        self.follow_typing(window, cx);
        cx.notify();
    }

    /// The graph shown: the model's workflow, or the shot's.
    fn shown(&self) -> Option<&Recipe> {
        match &self.workflow {
            Some(workflow) => Some(&workflow.graph),
            None => self.recipe.as_ref(),
        }
    }

    /// What the cards shown are kept under.
    fn places_key(&self) -> String {
        match &self.workflow {
            Some(workflow) => workflow_key(&self.shot_key, &workflow.name),
            None => self.shot_key.clone(),
        }
    }

    fn places(&self) -> HashMap<String, (f32, f32)> {
        self.shown()
            .map(|graph| placed_over(graph, &self.placed))
            .unwrap_or_default()
    }

    /// The canvas looks at all of what is shown, or where a graph too large
    /// to read whole starts.
    fn settle(&mut self) {
        let extent = self.shown().and_then(|graph| extent(graph, &self.places()));
        self.camera = settle(extent, self.room, self.whole);
        self.fitted = true;
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        self.whole = true;
        self.settle();
        cx.notify();
    }

    fn zoom_by(&mut self, factor: f32, about: (f32, f32), cx: &mut Context<Self>) {
        let camera = zoom_about(self.camera, factor, about);
        if camera != self.camera {
            self.camera = camera;
            self.fitted = false;
            cx.notify();
        }
    }

    /// The middle of the canvas.
    fn middle(&self) -> (f32, f32) {
        (self.room.0 / 2.0, self.room.1 / 2.0)
    }

    /// A point of the window on the canvas.
    fn local(&self, at: Point<Pixels>) -> (f32, f32) {
        let origin = self.canvas_bounds.get().origin;
        (f32::from(at.x - origin.x), f32::from(at.y - origin.y))
    }

    /// The canvas's room in `window` as it is now: the panel's size there
    /// less its head and its inspector; a canvas looking at all of the
    /// graph settles in it. Followed as the graph opens and as the window's
    /// bounds change. Whether the room is other than it was.
    fn follow_window(&mut self, window: &Window) -> bool {
        let viewport = window.viewport_size();
        let (width, height) = graph_size((f32::from(viewport.width), f32::from(viewport.height)));
        let room = (width - INSPECTOR - 1.0, height - HEAD - 1.0);
        if room == self.room {
            return false;
        }
        self.room = room;
        if self.fitted {
            self.settle();
        }
        true
    }

    fn set_recipe(&mut self, event: &RecipeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let recipe = Recipe::read(event);
        let ids = |recipe: &Recipe| -> Vec<String> {
            recipe.nodes.iter().map(|node| node.id.clone()).collect()
        };
        let before = self.recipe.as_ref().map(ids).unwrap_or_default();
        let now = ids(&recipe);
        // A recipe's next version is the same shot, by its name: the view stays.
        let another = self
            .recipe
            .as_ref()
            .is_none_or(|old| old.media != recipe.media || old.name != recipe.name);
        let edited = event.edited;
        // A step just added is the one to set next.
        if let Some(line) = event.added
            && let Some(node) = recipe.nodes.iter().find(|node| node.line == Some(line))
        {
            self.selected = Some(node.id.clone());
        }
        let pending = self.pending_node.take();
        self.recipe = Some(recipe);
        self.refused = None;
        self.want_pictures(window, cx);
        // A curve drawn is the project's now, and drawn as it says.
        if self.drawing.as_ref().is_some_and(|drawing| !drawing.held) {
            self.drawing = None;
        }
        if let Some(node) = pending {
            match node
                .strip_prefix("workflow:")
                .map(|line| line.parse::<usize>())
            {
                Some(Ok(line)) => self.open_workflow(line, window, cx),
                Some(Err(_)) => {}
                None => self.selected = Some(node),
            }
        }
        // While a workflow is only asked for, the shot's graph is the one
        // shown, and follows its steps as ever.
        if self.workflow.is_some() {
            // The line a workflow is of was changed: what fills its blanks,
            // and which file it is, are read again.
            if edited {
                self.request_workflow(cx);
            }
            return;
        }
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.recipe.as_ref().and_then(|r| r.node(id)).is_none())
        {
            self.selected = None;
        }
        if another {
            self.fitted = true;
            self.whole = false;
        }
        if self.fitted && (another || before != now) {
            self.settle();
        }
        self.inspect(window, cx);
    }

    /// The graph gives the window back, what was typed set first.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        cx.emit(DismissEvent);
    }

    /// A change to the steps, as they read when the graph was drawn.
    fn edit(&mut self, change: RecipeEdit, cx: &mut Context<Self>) {
        let Some(recipe) = &self.recipe else {
            return;
        };
        let change = RecipeEdit {
            media: self.media.to_string(),
            was: recipe.text.clone(),
            ..change
        };
        self.said = None;
        self.session
            .update(cx, |session, _| session.send(Command::RecipeEdit(change)));
        cx.notify();
    }

    fn select(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != id {
            // What is typed sets its own node's line as it gives way.
            self.commit_fields(cx);
            self.selected = id;
            self.said = None;
            self.inspect(window, cx);
        }
        cx.notify();
    }

    fn open_menu(
        &mut self,
        kind: MenuKind,
        items: Vec<ui::MenuItem>,
        header: Option<String>,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = kind.width();
        let menu = cx.new(|cx| {
            let menu = ui::Menu::new(items, window, cx).width(width);
            match header {
                Some(header) => menu.header(header),
                None => menu,
            }
        });
        let closed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            // The keys are back where they were as it opened.
            this.follow_typing(window, cx);
            cx.notify();
        });
        self.menu = Some(GraphMenu {
            kind,
            menu,
            at,
            _closed: closed,
        });
        cx.notify();
    }

    /// Puts away the menu open, if any: true when it was of `kind`, which a
    /// second press on what opened it puts away and opens no other.
    fn close_menu(&mut self, kind: MenuKind, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.menu.take() else {
            return false;
        };
        open.menu.update(cx, |menu, cx| menu.close(window, cx));
        cx.notify();
        open.kind == kind
    }

    /// Make, or Several: what it would run is asked first, over the graph,
    /// which stays as it is under it and follows the making after a yes.
    fn open_plan(&mut self, rolls: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let session = self.session.clone();
        let media = self.media.clone();
        let plan = cx.new(|cx| MakePlan::new(session, media, rolls, window, cx));
        let closed = cx.subscribe_in(&plan, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_over(window, cx)
        });
        window.focus(&plan.focus_handle(cx));
        self.over = Some(Over::Plan(plan));
        self._over = Some(closed);
        cx.notify();
    }

    /// Cast and Styles…, over the graph.
    fn open_cast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let cast =
            cx.new(|cx| editor_launcher::cast::CastView::new(self.session.clone(), window, cx));
        let closed = cx.subscribe_in(&cast, window, |this, _, _: &DismissEvent, window, cx| {
            this.close_over(window, cx)
        });
        window.focus(&cast.focus_handle(cx));
        self.over = Some(Over::Cast(cast));
        self._over = Some(closed);
        cx.notify();
    }

    /// Settings, at its Models, over the graph.
    fn open_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        let session = self.session.clone();
        let settings = cx.new(|cx| {
            editor_launcher::settings::SettingsView::new_at(
                session,
                editor_launcher::settings::Page::Models,
                window,
                cx,
            )
        });
        let closed = cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &editor_launcher::settings::SettingsEvent, window, cx| match event {
                editor_launcher::settings::SettingsEvent::Dismiss => this.close_over(window, cx),
            },
        );
        window.focus(&settings.focus_handle(cx));
        self.over = Some(Over::Settings(settings));
        self._over = Some(closed);
        cx.notify();
    }

    fn close_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.over.take().is_some() {
            self._over = None;
            // A keymap chosen in Settings bound the window's keys again
            // without the graph's.
            bind_keys(cx);
            window.focus(&self.canvas_focus);
            // Its takes may be other than they were.
            self.request(cx);
            cx.notify();
        }
    }

    fn making<'a>(&self, session: &'a Session) -> Option<&'a Making> {
        session.state.makings.get(self.media.as_ref())
    }

    /// What opens over the graph, on a dim of it. The plan and Cast and
    /// Styles are as large as they make themselves; Settings fills what the
    /// window would give it ([`ui::modal_bounds`]), here of the graph's room
    /// (`room`, as large as the graph is drawn).
    fn over_box(&self, over: &Over, room: (f32, f32), cx: &mut Context<Self>) -> AnyElement {
        let (view, sized): (AnyElement, Option<gpui::Size<Pixels>>) = match over {
            Over::Plan(plan) => (plan.clone().into_any_element(), None),
            Over::Cast(cast) => (cast.clone().into_any_element(), None),
            Over::Settings(settings) => {
                let (width, height) = editor_launcher::settings::MOST;
                let placed =
                    ui::modal_bounds(size(px(width), px(height)), size(px(room.0), px(room.1)));
                (settings.clone().into_any_element(), Some(placed.size))
            }
        };
        let rise = scene::PAGE_RISE;
        div()
            .id("graph-over")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::with_alpha(theme::overlay_screen(), 0.35))
            .occlude()
            // A press on the dim gives the graph back.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_over(window, cx)
                }),
            )
            .child(
                div()
                    .id("graph-over-panel")
                    .rounded(px(radius::PANEL))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme::line2())
                    .shadow_lg()
                    .when_some(sized, |panel, sized| {
                        panel.w(sized.width).h(sized.height).flex().flex_col()
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(view)
                    .with_animation(
                        "graph-over-rise",
                        motion::animation(scene::PAGE),
                        move |panel, t| panel.opacity(t).mt(px(rise * (1.0 - t))),
                    ),
            )
            .into_any_element()
    }
}

impl EventEmitter<DismissEvent> for ShotGraph {}

impl Focusable for ShotGraph {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.canvas_focus.clone()
    }
}

impl Render for ShotGraph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height) = graph_size((f32::from(viewport.width), f32::from(viewport.height)));
        let head = self.head(cx);
        let canvas = self.canvas(window, cx);
        let inspector = self.inspector(cx);
        // A menu, over all of it.
        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.at)
                    .snap_to_window_with_margin(px(8.0))
                    .child(open.menu.clone()),
            )
            .with_priority(2)
        });
        let over = self
            .over
            .as_ref()
            .map(|over| self.over_box(over, (width, height), cx));
        let covered = self.over.is_some();
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Close, _, cx| this.dismiss(cx)))
            // A press on the dim around the panel gives the window back.
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                if !covered && this.menu.is_none() {
                    this.dismiss(cx)
                }
            }))
            // A press or a key may take the keys out of a field: what it
            // says is set once that has run.
            .capture_any_mouse_down(
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.keys_may_move(window, cx)),
            )
            .capture_key_down(
                cx.listener(|this, _: &KeyDownEvent, window, cx| this.keys_may_move(window, cx)),
            )
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::middle_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::middle_up))
            .w(px(width))
            .h(px(height))
            .flex()
            .flex_col()
            .bg(theme::chrome())
            .font(theme::ui_font(gpui::FontWeight::NORMAL))
            .text_color(theme::ink())
            .relative()
            .child(head)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(canvas)
                    .child(inspector),
            )
            .children(over)
            .children(menu)
    }
}

// ------------------------------------------------------------------ starting on a graph

/// A shot's graph to start on, and what is chosen in it: a node's id, or
/// `workflow:<line>` for the workflow of the model on that line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphStart {
    pub media: String,
    pub node: Option<String>,
}

/// Takes `--graph <media>` and `--graph-node <id>` (or `--graph=…`,
/// `--graph-node=…`) out of `args`: the graph the editor starts on, once
/// its project has said that media. A `--graph-node` without `--graph`, or
/// a flag with nothing after it, is refused.
pub fn graph_start(args: &mut Vec<String>) -> Result<Option<GraphStart>, String> {
    let mut media = None;
    let mut node = None;
    let mut kept = Vec::with_capacity(args.len());
    let mut words = std::mem::take(args).into_iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag == "--graph" || flag == "--graph-node" => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (word.clone(), None),
        };
        let slot = match flag.as_str() {
            "--graph" => &mut media,
            "--graph-node" => &mut node,
            _ => {
                kept.push(word);
                continue;
            }
        };
        let value = inline
            .or_else(|| words.next())
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("{flag} wants a value"))?;
        *slot = Some(value);
    }
    *args = kept;
    match (media, node) {
        (Some(media), node) => Ok(Some(GraphStart { media, node })),
        (None, Some(_)) => Err("--graph-node wants --graph".to_owned()),
        (None, None) => Ok(None),
    }
}

/// The node the next graph opened on its media chooses, from `--graph-node`.
static START: std::sync::Mutex<Option<GraphStart>> = std::sync::Mutex::new(None);

/// Opens the graph `start` names (with `open`, which puts it over its
/// window) once `session`'s project has said its media, choosing its node,
/// or opening its model's workflow, once the bridge has said the graph.
/// `app::open_graph_when_up` calls it for `--graph` as the window opens.
pub fn open_when_ready(
    start: GraphStart,
    session: &Entity<Session>,
    open: impl Fn(SharedString, &mut App) + 'static,
    cx: &mut App,
) {
    if let Ok(mut pending) = START.lock() {
        *pending = Some(start.clone());
    }
    let opened = std::rc::Rc::new(std::cell::Cell::new(false));
    cx.observe(session, move |session, cx| {
        if opened.get() || session.read(cx).timeline().source(&start.media).is_none() {
            return;
        }
        opened.set(true);
        open(start.media.clone().into(), cx);
    })
    .detach();
}

/// The node `--graph-node` chose for `media`'s graph, taken once.
fn start_node(media: &str) -> Option<String> {
    let mut pending = START.lock().ok()?;
    if pending.as_ref().is_some_and(|start| start.media == media) {
        return pending.take().and_then(|start| start.node);
    }
    None
}
