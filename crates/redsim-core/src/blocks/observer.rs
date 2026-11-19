//! Observers (`ObserverBlock`).

use crate::data::State;
use crate::gen::{blk, prop};
use crate::pos::BlockPos;
use crate::world::World;

impl World {
    pub(crate) fn observer_start_signal(&mut self, pos: BlockPos) {
        if !self.has_scheduled_tick(pos, blk::OBSERVER) {
            self.schedule_tick(pos, blk::OBSERVER, 2, 0);
        }
    }

    fn observer_update_neighbors_in_front(&mut self, pos: BlockPos, s: State) {
        let dir = self.d.get_dir(s, prop::FACING);
        let op = pos.rel(dir.opposite());
        self.neighbor_changed(op, blk::OBSERVER);
        self.update_neighbors_at_except_from_facing(op, blk::OBSERVER, dir);
    }

    pub(crate) fn observer_tick(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) {
            let ns = self.d.with_bool(s, prop::POWERED, false);
            self.set_block(pos, ns, 2);
        } else {
            let ns = self.d.with_bool(s, prop::POWERED, true);
            self.set_block(pos, ns, 2);
            self.schedule_tick(pos, blk::OBSERVER, 2, 0);
        }
        self.observer_update_neighbors_in_front(pos, s);
    }

    pub(crate) fn observer_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if !self.is(old, blk::OBSERVER) && self.d.get_bool(s, prop::POWERED) && !self.has_scheduled_tick(pos, blk::OBSERVER) {
            let ns = self.d.with_bool(s, prop::POWERED, false);
            self.set_block(pos, ns, 18);
            self.observer_update_neighbors_in_front(pos, ns);
        }
    }

    pub(crate) fn observer_affect_neighbors_after_removal(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) && self.has_scheduled_tick(pos, blk::OBSERVER) {
            let ns = self.d.with_bool(s, prop::POWERED, false);
            self.observer_update_neighbors_in_front(pos, ns);
        }
    }
}
