//! A game's own state for each of the renderer's slots, kept while the slot
//! holds the same model.

/// What a game keeps for one slot: its simulations, which start over when
/// the slot takes another model, and whatever else it keeps by geometry.
pub(in crate::app::model_preview) trait GameSlot: Default {
    /// Start the simulations over, for the slot's next model.
    fn restart(&mut self);
}

/// A game's state for each slot, with the slot's upload it is for: the core
/// counts the models each slot takes, and another count restarts it, as the
/// core's own simulations restart.
pub(in crate::app::model_preview) struct GameSlots<S> {
    slots: Vec<(Option<u64>, S)>,
}

impl<S> Default for GameSlots<S> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<S: GameSlot> GameSlots<S> {
    /// `slot`'s state for its `uploads`th model, started afresh if it was for
    /// another.
    pub(in crate::app::model_preview) fn of(&mut self, slot: usize, uploads: u64) -> &mut S {
        if self.slots.len() <= slot {
            self.slots.resize_with(slot + 1, || (None, S::default()));
        }
        let (upload, state) = &mut self.slots[slot];
        if *upload != Some(uploads) {
            state.restart();
            *upload = Some(uploads);
        }
        state
    }

    /// `slot`'s state, if it is for the slot's `uploads`th model.
    #[cfg(test)]
    pub(in crate::app::model_preview) fn current(&self, slot: usize, uploads: u64) -> Option<&S> {
        let (upload, state) = self.slots.get(slot)?;
        (*upload == Some(uploads)).then_some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many times a slot's state has started over.
    #[derive(Default)]
    struct Counted(u32);

    impl GameSlot for Counted {
        fn restart(&mut self) {
            self.0 += 1;
        }
    }

    #[test]
    fn a_slots_state_starts_over_for_each_model_it_takes() {
        let mut slots = GameSlots::<Counted>::default();
        assert_eq!(slots.of(1, 1).0, 1, "a slot's first model starts it");
        assert_eq!(slots.of(1, 1).0, 1, "the same model keeps it");
        assert_eq!(slots.of(0, 1).0, 1, "each slot keeps its own");
        assert_eq!(slots.of(1, 2).0, 2, "the slot's next model starts it over");
        assert!(slots.current(1, 1).is_none(), "nothing is kept for a model the slot let go");
        assert_eq!(slots.current(1, 2).map(|state| state.0), Some(2));
        assert!(slots.current(5, 1).is_none());
    }
}
