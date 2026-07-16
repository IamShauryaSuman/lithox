pub type FrameId = usize;

#[derive(Debug, Clone, PartialEq)]
enum FrameState {
    /// Ignored: The frame is actively in use, or empty. It cannot be evicted.
    EmptyOrPinned,

    /// Accessed (Ref = 1): Unpinned recently. Given a second chance.
    Accessed,

    /// Evictable (Ref = 0): Unpinned and untouched for a while. Prime for eviction.
    Evictable,
}

pub struct ClockReplacer {
    /// The "clock face": an array holding the state of every frame.
    frames: Vec<FrameState>,

    /// The "clock hand": the index we are currently evaluating.
    clock_hand: usize,

    /// The total number of frames this replacer manages.
    capacity: usize,
}

impl ClockReplacer {
    /// Creates a new Clock Replacer managing up to `capacity` frames.
    pub fn new(capacity: usize) -> Self {
        Self {
            frames: vec![FrameState::EmptyOrPinned; capacity],
            clock_hand: 0,
            capacity,
        }
    }

    /// Finds a victim frame to evict according to the clock algorithm.
    pub fn victim(&mut self) -> Option<FrameId> {
        let mut steps = 0;
        let max_steps = self.capacity * 2;

        while steps < max_steps {
            match self.frames[self.clock_hand] {
                FrameState::EmptyOrPinned => {}
                FrameState::Accessed => {
                    self.frames[self.clock_hand] = FrameState::Evictable;
                }
                FrameState::Evictable => {
                    let victim_id = self.clock_hand;
                    self.frames[victim_id] = FrameState::EmptyOrPinned;
                    self.clock_hand = (self.clock_hand + 1) % self.capacity;

                    return Some(victim_id);
                }
            }

            self.clock_hand = (self.clock_hand + 1) % self.capacity;
            steps += 1;
        }

        None
    }

    /// Removes a frame from eviction consideration (it is actively being used).
    pub fn pin(&mut self, frame_id: FrameId) {
        if frame_id < self.capacity {
            self.frames[frame_id] = FrameState::EmptyOrPinned;
        }
    }

    /// Adds a frame for eviction consideration (it is no longer being used).
    pub fn unpin(&mut self, frame_id: FrameId) {
        if frame_id < self.capacity && self.frames[frame_id] == FrameState::EmptyOrPinned {
            self.frames[frame_id] = FrameState::Accessed;
        }
    }

    /// Returns the number of frames currently eligible for eviction.
    pub fn size(&self) -> usize {
        self.frames
            .iter()
            .filter(|&state| *state != FrameState::EmptyOrPinned)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clock_replacer() {
        let mut replacer = ClockReplacer::new(5);

        // All frames start empty, so no victims available
        assert_eq!(replacer.victim(), None);
        assert_eq!(replacer.size(), 0);

        // Unpin frames 1, 2, 3. They are now "Accessed"
        replacer.unpin(1);
        replacer.unpin(2);
        replacer.unpin(3);
        assert_eq!(replacer.size(), 3);

        // Frame 1 gets pinned again. It should be removed from eviction consideration.
        replacer.pin(1);
        assert_eq!(replacer.size(), 2);

        // Ask for a victim.
        // Hand starts at 0 (Pinned). Moves to 1 (Pinned).
        // Moves to 2 (Accessed -> flips to Evictable).
        // Moves to 3 (Accessed -> flips to Evictable).
        // Moves to 4 (Pinned).
        // Moves to 0 (Pinned). Moves to 1 (Pinned).
        // Moves to 2 (Evictable -> JACKPOT).
        assert_eq!(replacer.victim(), Some(2));
        assert_eq!(replacer.size(), 1);

        // Next victim should be 3.
        assert_eq!(replacer.victim(), Some(3));
        assert_eq!(replacer.size(), 0);

        // No more victims
        assert_eq!(replacer.victim(), None);
    }
}
