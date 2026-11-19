//! Doors, trapdoors and fence gates (`DoorBlock`, `TrapDoorBlock`, `FenceGateBlock`).

use crate::data::{State, AIR};
use crate::gen::{prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::shape::Support;
use crate::world::World;

/// Direction a player using a block is facing in the oracle and API (yaw 0 = south).
pub const PLAYER_FACING: Dir = Dir::South;

impl World {
    #[inline]
    fn door_lower(&self, s: State) -> bool {
        self.d.get_str(s, prop::DOUBLE_BLOCK_HALF) == "lower"
    }

    pub(crate) fn door_can_survive(&self, s: State, pos: BlockPos) -> bool {
        let below = pos.below();
        let bs = self.get(below);
        if self.door_lower(s) {
            self.sturdy(bs, below, Dir::Up, Support::Full)
        } else {
            self.d.block_of(bs) == self.d.block_of(s)
        }
    }

    pub(crate) fn door_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, ns: State) -> State {
        let lower = self.door_lower(s);
        if dir.is_horizontal() || lower != (dir == Dir::Up) {
            if lower && dir == Dir::Down && !self.door_can_survive(s, pos) {
                AIR
            } else {
                s
            }
        } else if self.d.kind(ns) == crate::data::Kind::Door && self.door_lower(ns) != lower {
            self.d.with_str(ns, prop::DOUBLE_BLOCK_HALF, if lower { "lower" } else { "upper" })
        } else {
            AIR
        }
    }

    /// Returns true if the door's power already matches (nothing to do, whatever block updated it).
    pub(crate) fn door_neighbor_changed(&mut self, s: State, pos: BlockPos, block: crate::data::BlockId) -> bool {
        let other = pos.rel(if self.door_lower(s) { Dir::Up } else { Dir::Down });
        let signal = self.has_neighbor_signal(pos) || self.has_neighbor_signal(other);
        if signal == self.d.get_bool(s, prop::POWERED) {
            return true;
        }
        if self.d.block_of(s) != block {
            if signal != self.d.get_bool(s, prop::OPEN) {
                self.game_event(if signal { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose }, pos, None);
            }
            let ns = self.d.with_bool(self.d.with_bool(s, prop::POWERED, signal), prop::OPEN, signal);
            self.set_block(pos, ns, 2);
        }
        false
    }

    fn can_open_by_hand(&self, s: State, key: &str) -> bool {
        self.d.block_field_bool(self.d.block_of(s), key, "canOpenByHand").unwrap_or(true)
    }

    pub(crate) fn door_use(&mut self, s: State, pos: BlockPos) -> bool {
        if !self.can_open_by_hand(s, "DoorBlock.type") {
            return false;
        }
        let ns = self.d.cycle(s, prop::OPEN);
        self.set_block(pos, ns, 10);
        let ev = if self.d.get_bool(ns, prop::OPEN) { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose };
        self.game_event(ev, pos, None);
        true
    }

    pub(crate) fn warn_waterlogged_pub(&mut self, s: State, pos: BlockPos) {
        self.warn_waterlogged(s, pos)
    }

    fn warn_waterlogged(&mut self, s: State, pos: BlockPos) {
        if self.d.has_prop(s, prop::WATERLOGGED) && self.d.get_bool(s, prop::WATERLOGGED) {
            self.warn(pos, "waterlogged block schedules a fluid tick: water flow is not simulated");
        }
    }

    /// Returns true if the power already matches (nothing to do).
    pub(crate) fn trapdoor_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let signal = self.has_neighbor_signal(pos);
        if signal == self.d.get_bool(s, prop::POWERED) {
            return true;
        }
        {
            let mut ns = s;
            if self.d.get_bool(ns, prop::OPEN) != signal {
                ns = self.d.with_bool(ns, prop::OPEN, signal);
                self.game_event(if signal { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose }, pos, None);
            }
            ns = self.d.with_bool(ns, prop::POWERED, signal);
            self.set_block(pos, ns, 2);
            self.warn_waterlogged(ns, pos);
        }
        false
    }

    pub(crate) fn trapdoor_use(&mut self, s: State, pos: BlockPos) -> bool {
        if !self.can_open_by_hand(s, "TrapDoorBlock.type") {
            return false;
        }
        let ns = self.d.cycle(s, prop::OPEN);
        self.set_block(pos, ns, 2);
        self.warn_waterlogged(ns, pos);
        let ev = if self.d.get_bool(ns, prop::OPEN) { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose };
        self.game_event(ev, pos, None);
        true
    }

    pub(crate) fn waterlog_update_shape(&mut self, s: State, pos: BlockPos) -> State {
        self.warn_waterlogged(s, pos);
        s
    }

    fn is_wall(&self, s: State) -> bool {
        self.d.has_tag(self.d.block_of(s), tag::WALLS)
    }

    pub(crate) fn fence_gate_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, ns: State) -> State {
        let facing = self.d.get_dir(s, prop::HORIZONTAL_FACING);
        if facing.cw().axis() != dir.axis() {
            return s;
        }
        let in_wall = self.is_wall(ns) || self.is_wall(self.get(pos.rel(dir.opposite())));
        self.d.with_bool(s, prop::IN_WALL, in_wall)
    }

    /// Returns true if the power already matches (nothing to do).
    pub(crate) fn fence_gate_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let power = self.has_neighbor_signal(pos);
        if self.d.get_bool(s, prop::POWERED) == power {
            return true;
        }
        {
            let ns = self.d.with_bool(self.d.with_bool(s, prop::POWERED, power), prop::OPEN, power);
            self.set_block(pos, ns, 2);
            if self.d.get_bool(s, prop::OPEN) != power {
                self.game_event(if power { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose }, pos, None);
            }
        }
        false
    }

    pub(crate) fn fence_gate_use(&mut self, s: State, pos: BlockPos) -> bool {
        let ns = if self.d.get_bool(s, prop::OPEN) {
            self.d.with_bool(s, prop::OPEN, false)
        } else {
            let mut t = s;
            if self.d.get_dir(t, prop::HORIZONTAL_FACING) == PLAYER_FACING.opposite() {
                t = self.d.with_dir(t, prop::HORIZONTAL_FACING, PLAYER_FACING);
            }
            self.d.with_bool(t, prop::OPEN, true)
        };
        self.set_block(pos, ns, 10);
        let ev = if self.d.get_bool(ns, prop::OPEN) { crate::vibration::GameEvent::BlockOpen } else { crate::vibration::GameEvent::BlockClose };
        self.game_event(ev, pos, None);
        true
    }
}
