//! Where the graph's cards go and how the canvas looks at them: each
//! card's size, laid out left to right by what it reads, where its edges
//! leave and come in, the camera that pans and zooms the canvas and settles
//! it on the graph, the card an arrow goes to, and the places a person put
//! cards, kept with the editor's settings.

use crate::making::{GraphEdge, GraphNode, Recipe};
use crate::session::Session;
use crate::ui;
use gpui::{px, App, Entity};
use std::collections::HashMap;
use std::path::Path;

/// A node's card, and the room between the cards for the edges to turn in.
pub(super) const CARD_W: f32 = 208.0;
pub(super) const CARD_H: f32 = 56.0;
/// What a model's line of its seed and size adds to its card.
pub(super) const CARD_MODEL: f32 = 18.0;
/// What a signal's curve or a clip's path adds to its card.
pub(super) const CARD_CURVE: f32 = 36.0;
const COLUMN_GAP: f32 = 112.0;
const ROW_GAP: f32 = 20.0;
/// A node of a model's workflow: its name over a row an input; and between
/// its columns, as its rows name what comes in.
pub(super) const PART_W: f32 = 232.0;
pub(super) const PART_HEAD: f32 = 28.0;
pub(super) const PART_ROW: f32 = 20.0;
const PART_GAP: f32 = 64.0;
/// What floats over the canvas keeps this far from its edges.
pub(super) const PAD: f32 = 12.0;
/// The panel at most, less where the window is smaller; the column that
/// says what the chosen node holds.
const GRAPH_MOST: (f32, f32) = (1480.0, 900.0);
pub(super) const INSPECTOR: f32 = 300.0;
/// The legend of what the edges carry, at the canvas's foot.
pub(super) const LEGEND: f32 = 28.0;
/// What the canvas's zoom goes up or down by a press.
pub const ZOOM_STEP: f32 = 1.2;
/// The least and the most the canvas is drawn at.
pub const ZOOM_LEAST: f32 = 0.35;
pub const ZOOM_MOST: f32 = 2.0;
/// The smallest a graph is shown at by itself: smaller than this its words
/// are not read, so a graph too large for the canvas starts at its left at
/// this size, to be moved along, and Fit takes all of it in. Edges' names
/// are not drawn under it either.
pub const ZOOM_READ: f32 = 0.7;

/// The graph panel's size in a window `viewport` big: [`GRAPH_MOST`],
/// less where the window is smaller, with the
/// modal's margin all round (`ui::modal_bounds`), so the whole panel, its
/// foot among it, shows in the box the window gives it however small the
/// window. Its inspector column scrolls what does not fit.
pub(super) fn graph_size(viewport: (f32, f32)) -> (f32, f32) {
    let placed = ui::modal_bounds(
        gpui::size(px(GRAPH_MOST.0), px(GRAPH_MOST.1)),
        gpui::size(px(viewport.0), px(viewport.1)),
    );
    (f32::from(placed.size.width), f32::from(placed.size.height))
}

/// Whether a card draws something under its lines: a signal's number over
/// its time, or the path a clip is led along.
pub fn draws_curve(node: &GraphNode) -> bool {
    match node.kind.as_str() {
        "signal" => node.holds["values"]
            .as_array()
            .is_some_and(|values| !values.is_empty()),
        "moves" => node.holds["path"]
            .as_str()
            .is_some_and(|path| !path.is_empty()),
        _ => false,
    }
}

/// A card's size, before what it reads makes it taller: a workflow's node
/// is its name and a row for each input; a model says a third thing; a
/// signal or a clip's way draws under its lines.
pub fn card_size(node: &GraphNode) -> (f32, f32) {
    if node.is_part() {
        let rows = node.holds["inputs"].as_array().map_or(0, Vec::len);
        return (PART_W, PART_HEAD + PART_ROW * rows as f32 + 8.0);
    }
    let mut height = CARD_H;
    if node.kind == "model" {
        height += CARD_MODEL;
    }
    if draws_curve(node) {
        height += CARD_CURVE;
    }
    (CARD_W, height)
}

/// A card's height: room for its lines, and for each thing it reads to
/// come in apart (a workflow's node has a row for each already).
pub fn card_height(node: &GraphNode, inputs: usize) -> f32 {
    let (_, height) = card_size(node);
    if node.is_part() {
        return height;
    }
    height.max(16.0 + 24.0 * inputs as f32)
}

/// Where each card goes, its top left, left to right by what it reads: a
/// node a column after the furthest thing it reads, what nothing comes into
/// beside the first that reads it, and each column centred on the middle,
/// top to bottom in the order of what each feeds.
pub fn arrange(recipe: &Recipe) -> HashMap<String, (f32, f32)> {
    let count = recipe.nodes.len();
    let index: HashMap<&str, usize> = recipe
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.id.as_str(), at))
        .collect();
    let wired: Vec<(usize, usize)> = recipe
        .edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index.get(edge.from.as_str())?,
                *index.get(edge.to.as_str())?,
            ))
        })
        .collect();
    let mut column = vec![0usize; count];
    // How far each is from what it reads: the longest way to it. A loop,
    // which steps never make, stops after as many passes as there are nodes.
    for _ in 0..count {
        let mut moved = false;
        for &(from, to) in &wired {
            if column[to] < column[from] + 1 {
                column[to] = column[from] + 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    for node in 0..count {
        if wired.iter().any(|&(_, to)| to == node) {
            continue;
        }
        let nearest = wired
            .iter()
            .filter(|&&(from, _)| from == node)
            .map(|&(_, to)| column[to])
            .min();
        if let Some(nearest) = nearest.filter(|nearest| *nearest > 0) {
            column[node] = nearest - 1;
        }
    }
    let inputs = |node: usize| wired.iter().filter(|&&(_, to)| to == node).count();
    let last = column.iter().copied().max().unwrap_or(0);
    let mut row: HashMap<usize, usize> = HashMap::new();
    let mut places = HashMap::new();
    for at in (0..=last).rev() {
        let mut here: Vec<usize> = (0..count).filter(|node| column[*node] == at).collect();
        let order = |node: usize| {
            wired
                .iter()
                .filter(|&&(from, _)| from == node)
                .map(|&(_, to)| {
                    let into: Vec<usize> = wired
                        .iter()
                        .filter(|&&(_, reader)| reader == to)
                        .map(|&(from, _)| from)
                        .collect();
                    let input = into.iter().rposition(|from| *from == node).unwrap_or(0);
                    row.get(&to).copied().unwrap_or(0) * 64 + input
                })
                .min()
                .unwrap_or(usize::MAX)
        };
        here.sort_by_key(|node| (order(*node), *node));
        let heights: Vec<f32> = here
            .iter()
            .map(|node| card_height(&recipe.nodes[*node], inputs(*node)))
            .collect();
        let total: f32 =
            heights.iter().sum::<f32>() + ROW_GAP * here.len().saturating_sub(1) as f32;
        let mut y = -total / 2.0;
        for (place, (node, height)) in here.iter().zip(heights).enumerate() {
            row.insert(*node, place);
            let card = &recipe.nodes[*node];
            let pitch = card_size(card).0 + if card.is_part() { PART_GAP } else { COLUMN_GAP };
            places.insert(card.id.clone(), (at as f32 * pitch, y.round()));
            y += height + ROW_GAP;
        }
    }
    places
}

/// Where each card is: as laid out, or where the person put it.
pub fn placed_over(
    recipe: &Recipe,
    placed: &HashMap<String, (f32, f32)>,
) -> HashMap<String, (f32, f32)> {
    let mut places = arrange(recipe);
    for (id, place) in places.iter_mut() {
        if let Some(put) = placed.get(id) {
            *place = *put;
        }
    }
    places
}

/// A card's rectangle on the scene: left, top, width, height.
pub fn card_rect(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    id: &str,
) -> Option<[f32; 4]> {
    let node = recipe.node(id)?;
    let (x, y) = places.get(id)?;
    let (width, _) = card_size(node);
    let height = card_height(node, recipe.reads_into(id).len());
    Some([*x, *y, width, height])
}

/// Where an edge leaves `id`, on the scene: its right edge, at its middle;
/// a workflow's node beside its name, over its rows.
pub fn out_port(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    id: &str,
) -> Option<(f32, f32)> {
    let [x, y, width, height] = card_rect(recipe, places, id)?;
    if recipe.node(id).is_some_and(GraphNode::is_part) {
        return Some((x + width, y + PART_HEAD / 2.0 + 2.0));
    }
    Some((x + width, y + height / 2.0))
}

/// Where `edge` comes into its card, on the scene: at the row of the input
/// it is, in a workflow; else spread down its left edge in the order the
/// steps give what it reads.
pub fn in_port(
    recipe: &Recipe,
    places: &HashMap<String, (f32, f32)>,
    edge: &GraphEdge,
) -> Option<(f32, f32)> {
    let [x, y, _, height] = card_rect(recipe, places, &edge.to)?;
    let to = recipe.node(&edge.to)?;
    if to.is_part() {
        let row = to.holds["inputs"]
            .as_array()
            .and_then(|inputs| inputs.iter().rposition(|input| input["name"] == edge.port))
            .unwrap_or(0);
        return Some((x, y + PART_HEAD + PART_ROW * (row as f32 + 0.5)));
    }
    let into = recipe.reads_into(&edge.to);
    let index = into
        .iter()
        .rposition(|other| {
            other.from == edge.from && other.port == edge.port && other.nth == edge.nth
        })
        .unwrap_or(0);
    Some((
        x,
        y + height * (index as f32 + 0.5) / into.len().max(1) as f32,
    ))
}

/// How the canvas is looked at: where on it a point of the scene is drawn
/// is `offset + point * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub zoom: f32,
    pub offset: (f32, f32),
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: (0.0, 0.0),
        }
    }
}

impl Camera {
    pub fn to_view(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (self.offset.0 + x * self.zoom, self.offset.1 + y * self.zoom)
    }

    pub fn to_scene(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let zoom = self.zoom.max(f32::EPSILON);
        ((x - self.offset.0) / zoom, (y - self.offset.1) / zoom)
    }
}

/// What every card covers of the scene: left, top, right, bottom.
pub fn extent(recipe: &Recipe, places: &HashMap<String, (f32, f32)>) -> Option<[f32; 4]> {
    let mut all: Option<[f32; 4]> = None;
    for node in &recipe.nodes {
        let Some([x, y, width, height]) = card_rect(recipe, places, &node.id) else {
            continue;
        };
        all = Some(match all {
            None => [x, y, x + width, y + height],
            Some([left, top, right, bottom]) => [
                left.min(x),
                top.min(y),
                right.max(x + width),
                bottom.max(y + height),
            ],
        });
    }
    all
}

/// The canvas `room` big, looking at cards covering `extent`: all of them,
/// in the middle of the room the legend leaves, no larger than they are; or,
/// short of the `whole` of them (Fit), a graph too large to read whole at
/// its left, at a size that is read ([`ZOOM_READ`]).
pub fn settle(extent: Option<[f32; 4]>, room: (f32, f32), whole: bool) -> Camera {
    let Some([left, top, right, bottom]) = extent else {
        return Camera {
            zoom: 1.0,
            offset: ((room.0 / 2.0).round(), (room.1 / 2.0).round()),
        };
    };
    // Room round the cards, and the legend under them.
    let area = [
        32.0,
        24.0,
        room.0 - 32.0,
        room.1 - (LEGEND + 2.0 * PAD + 8.0),
    ];
    let (width, height) = ((area[2] - area[0]).max(1.0), (area[3] - area[1]).max(1.0));
    let (all_w, all_h) = ((right - left).max(1.0), (bottom - top).max(1.0));
    let zoom = 1.0f32
        .min(width / all_w)
        .min(height / all_h)
        .clamp(ZOOM_LEAST, ZOOM_MOST);
    let centre = ((area[0] + area[2]) / 2.0, (area[1] + area[3]) / 2.0);
    let middle = ((left + right) / 2.0, (top + bottom) / 2.0);
    let mut offset = (centre.0 - middle.0 * zoom, centre.1 - middle.1 * zoom);
    let mut zoom = zoom;
    if !whole && zoom < ZOOM_READ {
        // Too wide or too tall to be read whole: where it starts, at a size that is.
        zoom = ZOOM_READ;
        let wide = all_w * zoom > width;
        let tall = all_h * zoom > height;
        offset = (
            if wide {
                area[0] - left * zoom
            } else {
                centre.0 - middle.0 * zoom
            },
            if tall {
                area[1] - top * zoom
            } else {
                centre.1 - middle.1 * zoom
            },
        );
    }
    Camera {
        zoom,
        offset: (offset.0.round(), offset.1.round()),
    }
}

/// `camera` zoomed by `factor` about `about`, a point of the canvas: what is
/// under it stays under it. The zoom stays between [`ZOOM_LEAST`] and
/// [`ZOOM_MOST`].
pub fn zoom_about(camera: Camera, factor: f32, about: (f32, f32)) -> Camera {
    let zoom = (camera.zoom * factor).clamp(ZOOM_LEAST, ZOOM_MOST);
    let scene = camera.to_scene(about);
    Camera {
        zoom,
        offset: (about.0 - scene.0 * zoom, about.1 - scene.1 * zoom),
    }
}

/// Which way an arrow key goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Left,
    Right,
    Up,
    Down,
}

/// The card an arrow goes to from `from`: the nearest that way, by the
/// middles of the cards (`centres`), one well off to the side counting for
/// more; the first card when none is chosen.
pub fn nearest_toward(
    centres: &[(String, (f32, f32))],
    from: Option<&str>,
    toward: Toward,
) -> Option<String> {
    let Some(start) = from.and_then(|from| centres.iter().find(|(id, _)| id == from)) else {
        return centres.first().map(|(id, _)| id.clone());
    };
    let across = matches!(toward, Toward::Left | Toward::Right);
    let sign = if matches!(toward, Toward::Left | Toward::Up) {
        -1.0
    } else {
        1.0
    };
    let mut best: Option<(&str, f32)> = None;
    for (id, centre) in centres {
        if id == &start.0 {
            continue;
        }
        let by = (centre.0 - start.1 .0, centre.1 - start.1 .1);
        let along = if across { by.0 } else { by.1 } * sign;
        let aside = if across { by.1 } else { by.0 }.abs();
        if along <= 1.0 || aside > along * 2.0 + 40.0 {
            continue;
        }
        let far = along + aside * 2.0;
        if best.is_none_or(|(_, nearest)| far < nearest) {
            best = Some((id, far));
        }
    }
    best.map(|(id, _)| id.to_owned())
}

// ------------------------------------------------------------------ where cards were put
//
// The cards' places are kept with the editor's own settings
// (`Prefs::graph_places`), never in the recipe.

/// The key a shot's cards are kept under: its project and its media.
pub fn placement_key(project: Option<&Path>, media: &str) -> String {
    let project = project.map(|project| project.display().to_string());
    format!("{}\nmedia {media}", project.unwrap_or_default())
}

/// The key a model's workflow's cards are kept under: another arrangement
/// than its shot's.
pub fn workflow_key(shot: &str, workflow: &str) -> String {
    format!("{shot}\nworkflow {workflow}")
}

pub(super) fn load_places(
    session: &Entity<Session>,
    key: &str,
    cx: &App,
) -> HashMap<String, (f32, f32)> {
    session.read(cx).prefs.graph_places_of(key)
}

pub(super) fn save_places(
    session: &Entity<Session>,
    key: &str,
    places: &HashMap<String, (f32, f32)>,
    cx: &mut App,
) {
    session.update(cx, |session, _| {
        session.prefs.keep_graph_places(key, places);
        session.save_prefs();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: kind.into(),
            ..GraphNode::default()
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            port: "guide".into(),
            ..GraphEdge::default()
        }
    }

    #[test]
    fn a_loop_does_not_keep_the_layout_from_ending() {
        let recipe = Recipe {
            nodes: vec![node("a", "render"), node("b", "model")],
            edges: vec![edge("a", "b"), edge("b", "a"), edge("a", "missing")],
            ..Recipe::default()
        };
        assert_eq!(arrange(&recipe).len(), 2);
        assert!(arrange(&Recipe::default()).is_empty());
    }
}
