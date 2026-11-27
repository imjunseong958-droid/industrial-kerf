//! What the drawings of several passes of one sequence share ([`crate::Compositor::passes`]), so
//! that the work of a frame that is the same in every pass is done once for all of them: each
//! clip's picture before a pass is made of it ([`Drawing::ready`]), and what of that picture the
//! camera sees at each pixel ([`three_d::Seen`]), which each pass then reads its own picture by.
//!
//! Whichever drawing asks for a thing first makes it, and one that asks while it is being made
//! waits for it, so that drawings of the same frame go on together. Making a thing asks for no
//! other, so no two wait for each other. A thing is let go once every drawing has taken it or is
//! done with the frame it is of, however that drawing drew the frame: on the GPU, or from frames
//! kept on disk, it asks for none of it. Past [`MOST_HELD`] bytes the oldest go first, for a
//! drawing that never draws a frame the others do, as one does whose stretch of a render was kept.

use super::*;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, PoisonError};

/// How many drawings can share their work at most: each has a bit of a `u64` for what it took.
pub(crate) const MOST_SHARING: usize = 64;

/// The most bytes of things held for drawings that have not taken them yet.
const MOST_HELD: usize = 512 << 20;

/// A clip's picture as [`Drawing::ready`] makes it before a pass is made of it: which clip of
/// which sequence, when, how large, on what canvas, and how far past its frame. That is all it is
/// made from, for a clip with no effects that does not nest a sequence, whose pictures are drawn
/// in each pass as that pass has the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ReadyKey {
    sequence: usize,
    track: usize,
    place: usize,
    at: i64,
    scale: u64,
    canvas: (usize, usize),
    reach: Option<[u64; 3]>,
}

/// What the camera of a scene sees of the picture of a [`ReadyKey`], placed on `plane`, on an
/// output picture of `size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct SeenKey {
    pub(super) ready: ReadyKey,
    pub(super) size: (usize, usize),
    pub(super) camera: [u64; 16],
    pub(super) plane: [u64; 15],
    /// What share of its size the picture is read at.
    pub(super) by: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Ready(ReadyKey),
    Seen(Box<SeenKey>),
}

impl Key {
    /// The frame of the program's sequence it is of, by its time: none for one of a sequence it
    /// nests, which may be of more than one.
    fn frame(&self) -> Option<i64> {
        let ready = match self {
            Self::Ready(ready) => ready,
            Self::Seen(seen) => &seen.ready,
        };
        (ready.sequence == 0).then_some(ready.at)
    }
}

#[derive(Clone)]
enum Thing {
    /// None where the clip draws nothing then.
    Ready(Option<Ready>),
    Seen(Arc<three_d::Seen>),
}

impl Thing {
    fn bytes(&self) -> usize {
        match self {
            Self::Ready(ready) => ready.as_ref().map_or(0, |ready| ready.picture.px.len() * 4),
            Self::Seen(seen) => seen.bytes(),
        }
    }
}

enum Slot {
    Making,
    Made {
        thing: Box<Thing>,
        /// Which drawings have taken it, a bit each.
        taken: u64,
        bytes: usize,
        /// When it was made, to let go of the oldest first.
        made: u64,
    },
}

#[derive(Default)]
struct Held {
    things: HashMap<Key, Slot>,
    /// What was made, oldest first, by when; some let go of already.
    order: VecDeque<(u64, Key)>,
    bytes: usize,
    made: u64,
    /// The frames of the program's sequence, by their times: which drawings are done with each,
    /// and what was made of it.
    frames: HashMap<i64, (u64, Vec<Key>)>,
}

/// The most frames [`Held::frames`] keeps: those some drawing never draws, beyond.
const MOST_FRAMES: usize = 256;

impl Held {
    /// The thing of `key`, if it is held, taken by `taking`: let go of once all of `all` have.
    fn take(&mut self, key: &Key, taking: u64, all: u64) -> Option<Thing> {
        let thing = match self.things.get(key) {
            Some(Slot::Made { thing, .. }) => Thing::clone(thing),
            _ => return None,
        };
        self.mark(key, taking, all);
        Some(thing)
    }

    /// Mark the thing of `key` taken by `taking`, and let go of it once all of `all` have: whether
    /// it is held still.
    fn mark(&mut self, key: &Key, taking: u64, all: u64) -> bool {
        let Some(Slot::Made { taken, .. }) = self.things.get_mut(key) else {
            return false;
        };
        *taken |= taking;
        if *taken != all {
            return true;
        }
        if let Some(Slot::Made { bytes, .. }) = self.things.remove(key) {
            self.bytes -= bytes;
        }
        false
    }

    /// Whether the thing of `key` is the one made at `made`.
    fn is(&self, key: &Key, made: u64) -> bool {
        matches!(self.things.get(key), Some(Slot::Made { made: when, .. }) if *when == made)
    }

    /// Let go of the oldest things made until no more than [`MOST_HELD`] bytes are held.
    fn trim(&mut self) {
        if self.order.len() > 2 * self.things.len() + 64 {
            let mut order = std::mem::take(&mut self.order);
            order.retain(|(made, key)| self.is(key, *made));
            self.order = order;
        }
        if self.frames.len() > MOST_FRAMES
            && let Some(oldest) = self.frames.keys().min().copied()
        {
            self.frames.remove(&oldest);
        }
        while let Some((made, key)) = self.order.front() {
            let current = self.is(key, *made);
            if current && self.bytes <= MOST_HELD {
                return;
            }
            let Some((_, key)) = self.order.pop_front() else {
                return;
            };
            if current && let Some(Slot::Made { bytes, .. }) = self.things.remove(&key) {
                self.bytes -= bytes;
            }
        }
    }
}

/// The work several drawings share, for [`MOST_SHARING`] at most.
pub(crate) struct Shared {
    sharers: usize,
    held: Mutex<Held>,
    /// Told whenever a thing is made, or given up on.
    made: Condvar,
}

impl Shared {
    pub(crate) fn new(sharers: usize) -> Self {
        assert!(
            (1..=MOST_SHARING).contains(&sharers),
            "{sharers} drawings cannot share their work"
        );
        Self {
            sharers,
            held: Mutex::default(),
            made: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The bits of every drawing.
    fn all(&self) -> u64 {
        u64::MAX >> (64 - self.sharers)
    }

    /// The thing of `key` for drawing `me`: taken as another made it, or made by `make`.
    fn thing(&self, me: usize, key: Key, make: impl FnOnce() -> Thing) -> Thing {
        let mine = 1u64 << me;
        {
            let mut held = self.lock();
            loop {
                match held.things.get_mut(&key) {
                    None => {
                        held.things.insert(key.clone(), Slot::Making);
                        break;
                    }
                    Some(Slot::Making) => {
                        held = self.made.wait(held).unwrap_or_else(PoisonError::into_inner);
                    }
                    Some(Slot::Made { .. }) => {
                        if let Some(thing) = held.take(&key, mine, self.all()) {
                            return thing;
                        }
                    }
                }
            }
        }
        // Were making it to panic, those waiting for it make it themselves.
        let mut making = Making {
            shared: self,
            key: Some(key),
        };
        let thing = make();
        let key = making
            .key
            .take()
            .expect("only a thing made gives its key back");
        let mut held = self.lock();
        // Those done with its frame already will not ask for it.
        let done = key
            .frame()
            .and_then(|frame| held.frames.get(&frame))
            .map_or(0, |(done, _)| *done);
        let taken = mine | done;
        if taken == self.all() {
            held.things.remove(&key);
        } else {
            let bytes = thing.bytes();
            held.made += 1;
            let made = held.made;
            held.things.insert(
                key.clone(),
                Slot::Made {
                    thing: Box::new(thing.clone()),
                    taken,
                    bytes,
                    made,
                },
            );
            if let Some(frame) = key.frame() {
                held.frames.entry(frame).or_default().1.push(key.clone());
            }
            held.order.push_back((made, key));
            held.bytes += bytes;
            held.trim();
        }
        drop(held);
        self.made.notify_all();
        thing
    }

    /// Drawing `me` is done with the frame of the program's sequence at `at`: what was made of it
    /// is let go once every drawing is done with it or has taken it.
    fn done(&self, me: usize, at: Time) {
        let mut held = self.lock();
        let frame = at.flicks();
        let (done, made) = {
            let (done, made) = held.frames.entry(frame).or_default();
            *done |= 1 << me;
            (*done, std::mem::take(made))
        };
        let left: Vec<Key> = made
            .into_iter()
            .filter(|key| held.mark(key, done, self.all()))
            .collect();
        if done == self.all() {
            held.frames.remove(&frame);
        } else if let Some((_, made)) = held.frames.get_mut(&frame) {
            made.extend(left);
        }
    }
}

/// A thing being made, given up on if it is dropped while it still holds its key.
struct Making<'a> {
    shared: &'a Shared,
    key: Option<Key>,
}

impl Drop for Making<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.shared.lock().things.remove(&key);
            self.shared.made.notify_all();
        }
    }
}

/// What a drawing shares, and which of the drawings it is.
pub(super) struct Sharing {
    pub(super) shared: Arc<Shared>,
    pub(super) me: usize,
}

/// A drawing's share of the work of a picture of a clip: what it is, and where it is shared.
#[derive(Clone, Copy)]
pub(super) struct Share<'a> {
    sharing: &'a Sharing,
    pub(super) ready: ReadyKey,
}

impl Share<'_> {
    /// What the camera sees of the picture, of `key`: as another drawing found it, or as `see`
    /// finds it.
    pub(super) fn seen(
        &self,
        key: SeenKey,
        see: impl FnOnce() -> three_d::Seen,
    ) -> Arc<three_d::Seen> {
        let Sharing { shared, me } = self.sharing;
        match shared.thing(*me, Key::Seen(Box::new(key)), || {
            Thing::Seen(Arc::new(see()))
        }) {
            Thing::Seen(seen) => seen,
            Thing::Ready(_) => unreachable!("a key of what is seen is made only into that"),
        }
    }
}

impl Drawing {
    /// Say that this drawing is done with the frame of the program's sequence at `at`, if it shares
    /// its work: it drew it, or found it kept.
    pub(super) fn done_with(&self, at: Time) {
        if let Some(Sharing { shared, me }) = &self.sharing {
            shared.done(*me, at);
        }
    }

    /// Where the work on the picture of `shown` of `sequence` at `at` is shared, on a canvas of
    /// `canvas` at `scale` of it, and drawn `reach` past its frame: none where this drawing shares
    /// none, or the picture may be another in another pass.
    pub(super) fn share_of(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        (at, scale): (Time, f64),
        canvas: (usize, usize),
        reach: Option<Reach>,
    ) -> Option<Share<'_>> {
        let sharing = self.sharing.as_ref()?;
        let plan = &shown.plan;
        // An effect may read the sequence's other tracks, and a nested sequence is drawn, as the
        // pass has them.
        if !plan.effects.is_empty() || matches!(plan.source, Source::Nested { .. }) {
            return None;
        }
        let index = self
            .sequences
            .iter()
            .position(|other| std::ptr::eq(other, sequence))?;
        let (track, place) = sequence
            .tracks
            .iter()
            .enumerate()
            .find_map(|(track, each)| {
                each.shown
                    .iter()
                    .position(|other| std::ptr::eq(other, shown))
                    .map(|place| (track, place))
            })?;
        Some(Share {
            sharing,
            ready: ReadyKey {
                sequence: index,
                track,
                place,
                at: at.flicks(),
                scale: scale.to_bits(),
                canvas,
                reach: reach.map(|reach| {
                    [
                        reach.beyond.0.to_bits(),
                        reach.beyond.1.to_bits(),
                        reach.per.to_bits(),
                    ]
                }),
            },
        })
    }

    /// The share of the picture of a clip that [`Ready::share`] says.
    pub(super) fn shared_by(&self, ready: Option<ReadyKey>) -> Option<Share<'_>> {
        Some(Share {
            sharing: self.sharing.as_ref()?,
            ready: ready?,
        })
    }

    /// The picture of a clip `share` says before a pass is made of it: as another drawing made
    /// it, or as `make` does.
    pub(super) fn shared_ready(
        &self,
        share: Share<'_>,
        make: impl FnOnce() -> Option<Ready>,
    ) -> Option<Ready> {
        let Sharing { shared, me } = share.sharing;
        match shared.thing(*me, Key::Ready(share.ready), || Thing::Ready(make())) {
            Thing::Ready(ready) => ready,
            Thing::Seen(_) => unreachable!("a key of a picture is made only into that"),
        }
    }
}
