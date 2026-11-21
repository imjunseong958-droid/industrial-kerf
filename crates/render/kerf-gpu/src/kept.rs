//! What a frame need not make again because the frame before it made the same thing, up to a budget.
//!
//! Only what is asked for a second time by the same key is held, so a title that changes on every frame
//! holds nothing, and what has not been asked for in a while goes, oldest first when there is no room.

/// How many frames something stays without being asked for: ten seconds at 24 frames a second.
const IDLE_FRAMES: u64 = 240;

/// How many keys asked for once are remembered.
const SEEN: usize = 32;

struct Entry<T> {
    key: String,
    bytes: usize,
    last: u64,
    value: T,
}

/// A store of what cost a frame to make, by a key saying all that went into it.
pub struct Store<T> {
    budget: usize,
    bytes: usize,
    now: u64,
    entries: Vec<Entry<T>>,
    seen: Vec<String>,
}

impl<T: Clone> Store<T> {
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            bytes: 0,
            now: 0,
            entries: Vec::new(),
            seen: Vec::new(),
        }
    }

    /// Another frame has begun: let go of what has gone unused.
    pub fn tick(&mut self) {
        self.now += 1;
        let (now, mut freed) = (self.now, 0);
        self.entries.retain(|entry| {
            let keep = entry.last + IDLE_FRAMES >= now;
            if !keep {
                freed += entry.bytes;
            }
            keep
        });
        self.bytes -= freed;
    }

    /// What is kept under `key`.
    pub fn get(&mut self, key: &str) -> Option<T> {
        let now = self.now;
        let entry = self.entries.iter_mut().find(|entry| entry.key == key)?;
        entry.last = now;
        Some(entry.value.clone())
    }

    /// Whether what `key` makes, of `bytes`, should be kept: it has been asked for before, and fits.
    pub fn wants(&mut self, key: &str, bytes: usize) -> bool {
        if let Some(place) = self.seen.iter().position(|seen| seen == key) {
            self.seen.remove(place);
            return bytes <= self.budget;
        }
        if self.seen.len() == SEEN {
            self.seen.remove(0);
        }
        self.seen.push(key.to_owned());
        false
    }

    /// Keep `value`, making room for it by letting go of what was asked for longest ago.
    pub fn put(&mut self, key: &str, bytes: usize, value: T) {
        while self.bytes + bytes > self.budget {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last)
                .map(|(place, _)| place);
            let Some(oldest) = oldest else {
                return;
            };
            self.bytes -= self.entries.swap_remove(oldest).bytes;
        }
        self.bytes += bytes;
        self.entries.push(Entry {
            key: key.to_owned(),
            bytes,
            last: self.now,
            value,
        });
    }

    /// The bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_never_holds_more_than_its_budget() {
        let mut store = Store::new(100);
        for index in 0..50 {
            store.tick();
            let key = format!("{index}");
            assert!(!store.wants(&key, 30));
            assert!(store.wants(&key, 30));
            store.put(&key, 30, index);
            assert!(store.bytes() <= 100, "{}", store.bytes());
        }
        // The newest are what is left.
        assert_eq!(store.bytes(), 90);
        assert_eq!(store.get("49"), Some(49));
        assert_eq!(store.get("47"), Some(47));
        assert_eq!(store.get("46"), None);
        // What cannot fit at all is never wanted.
        assert!(!store.wants("huge", 101));
        assert!(!store.wants("huge", 101));
    }
}
