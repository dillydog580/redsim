//! Redstone torches (floor and wall), including burnout (`RECENT_TOGGLES`).

use crate::data::{Kind, State};
use crate::gen::prop;
use crate::pos::{BlockPos, Dir};
use crate::world::World;

impl World {
    pub(crate) fn torch_own_signal(&self, s: State) -> i32 {
        if self.d.get_bool(s, prop::LIT) {
            15
        } else {
            0
        }
    }

    /// `RedstoneTorchBlock.hasNeighborSignal` (+ wall torch override)
    fn torch_has_neighbor_signal(&self, s: State, pos: BlockPos) -> bool {
        if self.kind(s) == Kind::WallTorch {
            let opp = self.d.get_dir(s, prop::HORIZONTAL_FACING).opposite();
            self.has_signal(pos.rel(opp), opp)
        } else {
            self.has_signal(pos.below(), Dir::Down)
        }
    }

    pub(crate) fn torch_notify_neighbors(&mut self, pos: BlockPos, s: State) {
        let block = self.d.block_of(s);
        for d in Dir::ALL {
            self.update_neighbors_at(pos.rel(d), block);
        }
    }

    /// Returns true if nothing changed because the torch is already right for its power (not when
    /// it only skipped because a tick is due: that depends on the tick queue).
    pub(crate) fn torch_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let block = self.d.block_of(s);
        if self.d.get_bool(s, prop::LIT) != self.torch_has_neighbor_signal(s, pos) {
            return true;
        }
        if !self.will_tick_this_tick(pos, block) {
            self.schedule_tick(pos, block, 2, 0);
        }
        false
    }

    fn torch_toggled_too_frequently(&mut self, pos: BlockPos, add: bool) -> bool {
        if add {
            self.torch_toggles.push((pos, self.game_time));
        }
        let mut count = 0;
        for &(p, _) in &self.torch_toggles {
            if p == pos {
                count += 1;
                if count >= 8 {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn torch_tick(&mut self, s: State, pos: BlockPos) {
        let neighbor_signal = self.torch_has_neighbor_signal(s, pos);
        while let Some(&(_, when)) = self.torch_toggles.first() {
            if self.game_time - when > 60 {
                self.torch_toggles.remove(0);
            } else {
                break;
            }
        }
        if self.d.get_bool(s, prop::LIT) {
            if neighbor_signal {
                let ns = self.d.with_bool(s, prop::LIT, false);
                self.set_block_and_update(pos, ns);
                if self.torch_toggled_too_frequently(pos, true) {
                    let b = self.block(pos);
                    self.schedule_tick(pos, b, 160, 0);
                }
            }
        } else if !neighbor_signal && !self.torch_toggled_too_frequently(pos, false) {
            let ns = self.d.with_bool(s, prop::LIT, true);
            self.set_block_and_update(pos, ns);
        }
    }
}
