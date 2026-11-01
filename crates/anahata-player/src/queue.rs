use rand::seq::SliceRandom;
use rand::thread_rng;

use crate::types::{QueueItem, RepeatMode};

/// What `Queue::remove_item` did, so the caller can react (for example when the playing track was removed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoveOutcome {
    /// The id existed and was removed.
    pub removed: bool,
    /// The removed item was the current one. `current_item()` now points at the item
    /// that followed it (if any); nothing has been started.
    pub removed_current: bool,
}

pub struct Queue {
    items: Vec<QueueItem>,
    play_order: Vec<usize>,
    current_index: Option<usize>, // index into play_order
    shuffle: bool,
    repeat: RepeatMode,
    revision: u64,
    next_id: u64,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            play_order: Vec::new(),
            current_index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            revision: 0,
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn current_item(&self) -> Option<&QueueItem> {
        self.current_index
            .and_then(|idx| self.play_order.get(idx))
            .and_then(|&item_idx| self.items.get(item_idx))
    }

    pub fn current_item_id(&self) -> Option<u64> {
        self.current_item().map(|it| it.id)
    }

    pub fn current_queue_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub fn is_shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }

        self.shuffle = enabled;
        if self.items.is_empty() {
            return;
        }

        let current_item_idx = self
            .current_index
            .and_then(|idx| self.play_order.get(idx).copied());

        if self.shuffle {
            // Build shuffled play_order
            let mut order: Vec<usize> = (0..self.items.len()).collect();
            let mut rng = thread_rng();
            order.shuffle(&mut rng);

            // Keep current playing track at position 0 in play_order
            if let Some(cur) = current_item_idx {
                if let Some(pos) = order.iter().position(|&x| x == cur) {
                    order.remove(pos);
                    order.insert(0, cur);
                }
                self.current_index = Some(0);
            }
            self.play_order = order;
        } else {
            // Unshuffled: sequential order 0..N-1
            self.play_order = (0..self.items.len()).collect();
            if let Some(cur) = current_item_idx {
                self.current_index = Some(cur);
            }
        }
        self.revision += 1;
    }

    pub fn load(&mut self, mut items: Vec<QueueItem>, start_index: Option<usize>) {
        for item in &mut items {
            if item.id == 0 {
                item.id = self.next_id;
                self.next_id += 1;
            } else if item.id >= self.next_id {
                self.next_id = item.id + 1;
            }
        }

        self.items = items;
        self.rebuild_order();

        if let Some(idx) = start_index {
            if idx < self.items.len() {
                if self.shuffle {
                    // Find where this item landed in play_order
                    self.current_index = self.play_order.iter().position(|&x| x == idx);
                } else {
                    self.current_index = Some(idx);
                }
            } else {
                self.current_index = None;
            }
        } else if !self.items.is_empty() {
            self.current_index = Some(0);
        } else {
            self.current_index = None;
        }

        self.revision += 1;
    }

    pub fn play_item(&mut self, queue_item_id: u64) -> Option<&QueueItem> {
        let item_idx = self.items.iter().position(|it| it.id == queue_item_id)?;
        let order_idx = self.play_order.iter().position(|&x| x == item_idx)?;
        self.current_index = Some(order_idx);
        self.current_item()
    }

    /// Natural advance, used when a track finishes by itself. Repeat One replays the same track.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&QueueItem> {
        if self.items.is_empty() {
            return None;
        }

        match self.repeat {
            RepeatMode::One => self.current_item(),
            RepeatMode::All => {
                let cur = self.current_index.unwrap_or(0);
                self.current_index = Some((cur + 1) % self.play_order.len());
                self.current_item()
            }
            RepeatMode::Off => {
                let cur = self.current_index.unwrap_or(0);
                if cur + 1 < self.play_order.len() {
                    self.current_index = Some(cur + 1);
                    self.current_item()
                } else {
                    // Reached end of queue
                    self.current_index = None;
                    None
                }
            }
        }
    }

    /// Advance requested by the user (Next button). Unlike `next()`, Repeat One does not trap
    /// the user on the same track: it behaves like Repeat All. At the end of the queue with
    /// repeat off it returns `None` and leaves nothing current.
    pub fn skip_next(&mut self) -> Option<&QueueItem> {
        if self.items.is_empty() {
            return None;
        }
        let cur = self.current_index.unwrap_or(0);
        let last = self.play_order.len() - 1;
        if cur < last {
            self.current_index = Some(cur + 1);
        } else if self.repeat == RepeatMode::Off {
            self.current_index = None;
            return None;
        } else {
            self.current_index = Some(0);
        }
        self.current_item()
    }

    /// Moves to the following item without wrapping or repeating. Returns `None` (and does
    /// not move) at the end. Used to hop over unplayable tracks without looping forever.
    pub fn step_forward(&mut self) -> Option<&QueueItem> {
        let cur = self.current_index?;
        if cur + 1 < self.play_order.len() {
            self.current_index = Some(cur + 1);
            self.current_item()
        } else {
            None
        }
    }

    pub fn previous(&mut self) -> Option<&QueueItem> {
        if self.items.is_empty() {
            return None;
        }

        let cur = self.current_index.unwrap_or(0);
        if cur > 0 {
            self.current_index = Some(cur - 1);
        } else if self.repeat == RepeatMode::All {
            self.current_index = Some(self.play_order.len() - 1);
        }
        self.current_item()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.play_order.clear();
        self.current_index = None;
        self.revision += 1;
    }

    /// Removes an item without disturbing what is playing: `current` follows the item's
    /// identity, and the shuffle order of the remaining items is preserved.
    pub fn remove_item(&mut self, queue_item_id: u64) -> RemoveOutcome {
        let Some(item_idx) = self.items.iter().position(|it| it.id == queue_item_id) else {
            return RemoveOutcome::default();
        };
        let order_pos = self
            .play_order
            .iter()
            .position(|&x| x == item_idx)
            .expect("every item appears in play_order");

        self.items.remove(item_idx);
        self.play_order.remove(order_pos);
        for entry in &mut self.play_order {
            if *entry > item_idx {
                *entry -= 1;
            }
        }

        let mut removed_current = false;
        if let Some(cur) = self.current_index {
            if order_pos < cur {
                self.current_index = Some(cur - 1);
            } else if order_pos == cur {
                removed_current = true;
                // The next item slid into this slot; if there is none, nothing is current.
                self.current_index = if cur < self.play_order.len() { Some(cur) } else { None };
            }
        }

        self.revision += 1;
        RemoveOutcome { removed: true, removed_current }
    }

    fn rebuild_order(&mut self) {
        if self.shuffle {
            let mut order: Vec<usize> = (0..self.items.len()).collect();
            let mut rng = thread_rng();
            order.shuffle(&mut rng);
            self.play_order = order;
        } else {
            self.play_order = (0..self.items.len()).collect();
        }

        if let Some(idx) = self.current_index {
            if idx >= self.play_order.len() {
                self.current_index = if self.play_order.is_empty() {
                    None
                } else {
                    Some(self.play_order.len() - 1)
                };
            }
        }
    }
}
