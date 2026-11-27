//! Ids both sides added for different things: theirs get fresh ids, and
//! what theirs names by them follows.

use super::elements::{params_of, track_head, Flat, Placed};
use kerf_model::{
    Catalog, Clip, Content, Id, IdGen, Marker, Media, Param, ParamDef, ParamType, Params, Project,
    Sequence, Transition, Value,
};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What an id names, compared to tell whether both sides added the same thing.
#[derive(PartialEq)]
enum Thing<'a> {
    Media(&'a Media),
    Sequence(&'a Sequence),
    Marker(&'a Id, &'a Marker),
    Clip(&'a Placed<Clip>),
    Transition(&'a Placed<Transition>),
    Link(BTreeSet<&'a Id>),
}

impl Thing<'_> {
    /// The pass that renames it: what it names is renamed in earlier passes.
    /// Tracks, named per sequence, are pass 1.
    fn pass(&self) -> usize {
        match self {
            Self::Media(_) | Self::Sequence(_) => 0,
            Self::Marker(..) => 2,
            Self::Clip(_) => 3,
            Self::Transition(_) | Self::Link(_) => 4,
        }
    }

    /// Whether both sides added the same thing under one id. Link groups
    /// that share a clip are one group that each side linked its own clips
    /// to: renaming theirs would pull the shared clip apart.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Link(mine), Self::Link(theirs)) => !mine.is_disjoint(theirs),
            _ => self == other,
        }
    }
}

/// Everything in the project-wide namespace, in document order.
fn things(flat: &Flat) -> Vec<(&Id, Thing<'_>)> {
    let mut things: Vec<(&Id, Thing)> = Vec::new();
    things.extend(
        flat.media
            .iter()
            .map(|(id, media)| (id, Thing::Media(media))),
    );
    things.extend(
        flat.sequences
            .iter()
            .map(|(id, s)| (id, Thing::Sequence(s))),
    );
    things.extend(
        flat.markers
            .iter()
            .map(|((sequence, id), marker)| (id, Thing::Marker(sequence, marker))),
    );
    things.extend(flat.clips.iter().map(|(id, clip)| (id, Thing::Clip(clip))));
    things.extend(
        flat.transitions
            .iter()
            .map(|(id, transition)| (id, Thing::Transition(transition))),
    );
    let mut links: Vec<(&Id, BTreeSet<&Id>)> = Vec::new();
    let mut groups: HashMap<&Id, usize> = HashMap::new();
    for (id, placed) in &flat.clips {
        let Some(link) = &placed.item.link else {
            continue;
        };
        let group = *groups.entry(link).or_insert_with(|| {
            links.push((link, BTreeSet::new()));
            links.len() - 1
        });
        links[group].1.insert(id);
    }
    things.extend(
        links
            .into_iter()
            .map(|(id, members)| (id, Thing::Link(members))),
    );
    things
}

/// Theirs' renames, applied wherever theirs names the ids.
#[derive(Default)]
struct Renames {
    ids: HashMap<Id, Id>,
    /// Track renames by sequence.
    tracks: HashMap<Id, HashMap<Id, Id>>,
}

impl Renames {
    fn is_empty(&self) -> bool {
        self.ids.is_empty() && self.tracks.is_empty()
    }

    fn apply(&self, project: &mut Project, catalog: &Catalog) {
        let rename = |id: &mut Id| {
            if let Some(new) = self.ids.get(id) {
                *id = new.clone();
            }
        };
        for media in &mut project.media {
            rename(&mut media.id);
        }
        for sequence in &mut project.sequences {
            rename(&mut sequence.id);
            let tracks = self.tracks.get(&sequence.id);
            let params = |params: &mut Params, defs: &[ParamDef]| self.params(params, defs, tracks);
            for marker in &mut sequence.markers {
                rename(&mut marker.id);
            }
            for track in &mut sequence.tracks {
                if let Some(new) = tracks.and_then(|tracks| tracks.get(&track.id)) {
                    track.id = new.clone();
                }
                for clip in &mut track.clips {
                    rename(&mut clip.id);
                    if let Some(link) = &mut clip.link {
                        rename(link);
                    }
                    if let Some(followed) = &mut clip.follows {
                        rename(followed);
                    }
                    match &mut clip.content {
                        Content::Media { media, .. } => rename(media),
                        Content::Sequence { sequence, .. } => rename(sequence),
                        Content::Generator {
                            fx, params: values, ..
                        } => {
                            params(values, params_of(catalog, fx));
                        }
                        Content::Caption { .. } => {}
                    }
                    if let Some(domain) = track.kind.domain() {
                        params(&mut clip.props, catalog.props(domain));
                    }
                    for effect in &mut clip.effects {
                        params(&mut effect.params, params_of(catalog, &effect.fx));
                    }
                }
                for transition in &mut track.transitions {
                    rename(&mut transition.id);
                    rename(&mut transition.from);
                    rename(&mut transition.to);
                    params(&mut transition.params, params_of(catalog, &transition.fx));
                }
            }
        }
    }

    /// Rewrite parameters that name a renamed marker or track.
    fn params(&self, params: &mut Params, defs: &[ParamDef], tracks: Option<&HashMap<Id, Id>>) {
        for (name, param) in params.iter_mut() {
            let (Some(def), Param::Static(Value::Text(text))) =
                (defs.iter().find(|def| def.name == *name), param)
            else {
                continue;
            };
            let new = match def.ty {
                ParamType::Time => self.ids.get(text.as_str()),
                ParamType::Track | ParamType::Sound => {
                    tracks.and_then(|tracks| tracks.get(text.as_str()))
                }
                _ => None,
            };
            if let Some(new) = new {
                *text = new.to_string();
            }
        }
    }
}

/// What a fresh id in place of `id` starts with: `id` without its number.
pub(super) fn prefix(id: &Id) -> &str {
    let text = id.as_str().trim_end_matches(|c: char| c.is_ascii_digit());
    &text[..text.len().min(Id::MAX_LEN - 8)]
}

/// Give theirs' additions that collide with ours' different additions fresh
/// ids, and rewrite theirs' references to them. Returns the renames.
pub(super) fn rename_collisions(
    base: &Project,
    ours: &Project,
    theirs: &mut Project,
    catalog: &Catalog,
) -> Vec<(Id, Id)> {
    let in_base: HashSet<&Id> = base.ids().collect();
    let ours_flat = Flat::of(ours);
    let ours_things: HashMap<&Id, Thing> = things(&ours_flat).into_iter().collect();
    let mut ids = IdGen::new(base.ids().chain(ours.ids()).chain(theirs.ids()));
    for id in base
        .retired
        .iter()
        .chain(&ours.retired)
        .chain(&theirs.retired)
    {
        ids.retire(id);
    }
    let mut renamed = Vec::new();
    for pass in 0..5 {
        let mut renames = Renames::default();
        if pass == 1 {
            renames.tracks = track_collisions(base, ours, theirs, &mut renamed);
        } else {
            let flat = Flat::of(theirs);
            for (id, thing) in things(&flat) {
                if thing.pass() != pass || in_base.contains(id) {
                    continue;
                }
                let collides = match ours_things.get(id) {
                    Some(other) => !other.same(&thing),
                    None => ours.retired.contains(id),
                };
                if collides {
                    let new = ids.next(prefix(id));
                    renamed.push((id.clone(), new.clone()));
                    renames.ids.insert(id.clone(), new);
                }
            }
        }
        if !renames.is_empty() {
            renames.apply(theirs, catalog);
        }
    }
    renamed
}

/// Fresh track ids for tracks both sides added to a sequence under one id
/// but made differently.
fn track_collisions(
    base: &Project,
    ours: &Project,
    theirs: &Project,
    renamed: &mut Vec<(Id, Id)>,
) -> HashMap<Id, HashMap<Id, Id>> {
    let mut renames: HashMap<Id, HashMap<Id, Id>> = HashMap::new();
    for sequence in &theirs.sequences {
        let Some(other) = ours.sequence(sequence.id.as_str()) else {
            continue;
        };
        let before = base.sequence(sequence.id.as_str());
        let mut ids = IdGen::new(
            [before, Some(other), Some(sequence)]
                .into_iter()
                .flatten()
                .flat_map(|s| s.tracks.iter().map(|track| &track.id)),
        );
        for track in &sequence.tracks {
            let added = before.is_none_or(|s| s.track(track.id.as_str()).is_none());
            let collides = other
                .track(track.id.as_str())
                .is_some_and(|mine| track_head(mine) != track_head(track));
            if added && collides {
                let new = ids.next(prefix(&track.id));
                renamed.push((track.id.clone(), new.clone()));
                renames
                    .entry(sequence.id.clone())
                    .or_default()
                    .insert(track.id.clone(), new);
            }
        }
    }
    renames
}
