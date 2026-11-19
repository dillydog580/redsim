//! Repeaters and comparators: `DiodeBlock`, `RepeaterBlock`, `ComparatorBlock`.

use crate::data::{Kind, State};
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::world::{BeData, World};

// TickPriority values
pub const EXTREMELY_HIGH: i8 = -3;
pub const VERY_HIGH: i8 = -2;
pub const HIGH: i8 = -1;
pub const NORMAL: i8 = 0;

impl World {
    #[inline]
    fn diode_facing(&self, s: State) -> Dir {
        self.d.get_dir(s, prop::HORIZONTAL_FACING)
    }
    #[inline]
    fn is_diode(&self, s: State) -> bool {
        matches!(self.kind(s), Kind::Repeater | Kind::Comparator)
    }

    fn diode_delay(&self, s: State) -> i32 {
        match self.kind(s) {
            Kind::Repeater => self.d.get_int(s, prop::DELAY) * 2,
            _ => 2,
        }
    }

    pub(crate) fn comparator_output(&self, pos: BlockPos) -> i32 {
        match self.bes().get(&pos) {
            Some(be) => match be.data {
                BeData::Comparator { output } => output,
                _ => 0,
            },
            None => 0,
        }
    }

    fn diode_output_signal(&self, s: State, pos: BlockPos) -> i32 {
        match self.kind(s) {
            Kind::Comparator => self.comparator_output(pos),
            _ => 15,
        }
    }

    /// `DiodeBlock.getSignal`
    pub(crate) fn diode_get_signal(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if self.diode_facing(s) == dir && self.d.get_bool(s, prop::POWERED) {
            self.diode_output_signal(s, pos)
        } else {
            0
        }
    }

    /// `DiodeBlock.getInputSignal` (+ ComparatorBlock override)
    fn diode_input_signal(&mut self, s: State, pos: BlockPos) -> i32 {
        let dir = self.diode_facing(s);
        let tp = pos.rel(dir);
        let mut input = self.get_signal(tp, dir);
        if input < 15 {
            let ts = self.get(tp);
            input = input.max(if self.is(ts, blk::REDSTONE_WIRE) { self.d.get_int(ts, prop::POWER) } else { 0 });
        }
        if self.kind(s) != Kind::Comparator {
            return input;
        }
        let mut result = input;
        let ts = self.get(tp);
        if self.has_analog_output(ts) {
            result = self.analog_output(ts, tp, dir.opposite());
        } else if result < 15 && self.is_conductor(ts) {
            let tp2 = tp.rel(dir);
            let ts2 = self.get(tp2);
            // item frames are entities: not simulated (treated as absent)
            if self.has_analog_output(ts2) {
                result = self.analog_output(ts2, tp2, dir.opposite());
            }
        }
        result
    }

    /// `DiodeBlock.getAlternateSignal`
    /// `mode == compare` (the only other mode is subtract)
    #[inline]
    fn comparator_compare_mode(&self, s: State) -> bool {
        self.d.get_idx(s, prop::MODE_COMPARATOR) == Some(self.d.comparator_compare_idx)
    }

    fn diode_alternate_signal(&self, s: State, pos: BlockPos) -> i32 {
        let dir = self.diode_facing(s);
        let cw = dir.cw();
        let ccw = dir.ccw();
        let only_diodes = self.kind(s) == Kind::Repeater;
        self.get_control_input_signal(pos.rel(cw), cw, only_diodes)
            .max(self.get_control_input_signal(pos.rel(ccw), ccw, only_diodes))
    }

    pub(crate) fn repeater_is_locked(&self, pos: BlockPos, s: State) -> bool {
        self.diode_alternate_signal(s, pos) > 0
    }

    fn diode_is_locked(&self, pos: BlockPos, s: State) -> bool {
        match self.kind(s) {
            Kind::Repeater => self.repeater_is_locked(pos, s),
            _ => false,
        }
    }

    fn diode_should_turn_on(&mut self, pos: BlockPos, s: State) -> bool {
        match self.kind(s) {
            Kind::Comparator => {
                let input = self.diode_input_signal(s, pos);
                if input == 0 {
                    return false;
                }
                let side = self.diode_alternate_signal(s, pos);
                if input > side {
                    true
                } else {
                    input == side && self.comparator_compare_mode(s)
                }
            }
            _ => self.diode_input_signal(s, pos) > 0,
        }
    }

    /// `DiodeBlock.shouldPrioritize`
    fn diode_should_prioritize(&self, pos: BlockPos, s: State) -> bool {
        let d = self.diode_facing(s).opposite();
        let os = self.get(pos.rel(d));
        self.is_diode(os) && self.diode_facing(os) != d
    }

    /// `DiodeBlock.tick` (repeater)
    pub(crate) fn diode_tick(&mut self, s: State, pos: BlockPos) {
        if !self.diode_is_locked(pos, s) {
            let on = self.d.get_bool(s, prop::POWERED);
            let should = self.diode_should_turn_on(pos, s);
            if on && !should {
                let ns = self.d.with_bool(s, prop::POWERED, false);
                self.set_block(pos, ns, 2);
            } else if !on {
                let ns = self.d.with_bool(s, prop::POWERED, true);
                self.set_block(pos, ns, 2);
                if !should {
                    let delay = self.diode_delay(s);
                    self.schedule_tick(pos, blk::REPEATER, delay, VERY_HIGH);
                }
            }
        }
    }

    /// `DiodeBlock.neighborChanged`
    /// Returns true (repeaters only) if nothing changed for a reason that depends only on the
    /// world's blocks: the repeater is not there any more, is locked, or is already right.
    pub(crate) fn diode_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let block = self.d.block_of(s);
        if self.is(self.get(pos), block) {
            if self.can_survive(s, pos) {
                self.diode_check_tick_on_neighbor(pos, s)
            } else {
                self.remove_block(pos, false);
                for d in Dir::ALL {
                    self.update_neighbors_at(pos.rel(d), block);
                }
                false
            }
        } else {
            self.kind(s) == Kind::Repeater
        }
    }

    /// `DiodeBlock.checkTickOnNeighbor`; returns true for a repeater that is locked or already
    /// right for its input (never for comparators: their check reads block entities with side effects).
    fn diode_check_tick_on_neighbor(&mut self, pos: BlockPos, s: State) -> bool {
        let block = self.d.block_of(s);
        if self.kind(s) == Kind::Comparator {
            if !self.will_tick_this_tick(pos, block) {
                let out = self.comparator_calculate_output(pos, s);
                let old = self.comparator_output(pos);
                if out != old || self.d.get_bool(s, prop::POWERED) != self.diode_should_turn_on(pos, s) {
                    let pri = if self.diode_should_prioritize(pos, s) { HIGH } else { NORMAL };
                    self.schedule_tick(pos, block, 2, pri);
                }
            }
            return false;
        }
        if self.diode_is_locked(pos, s) {
            return true;
        }
        {
            let on = self.d.get_bool(s, prop::POWERED);
            let should = self.diode_should_turn_on(pos, s);
            if on == should {
                return true;
            }
            if !self.will_tick_this_tick(pos, block) {
                let pri = if self.diode_should_prioritize(pos, s) {
                    EXTREMELY_HIGH
                } else if on {
                    VERY_HIGH
                } else {
                    HIGH
                };
                let delay = self.diode_delay(s);
                self.schedule_tick(pos, block, delay, pri);
            }
        }
        false
    }

    /// `DiodeBlock.updateNeighborsInFront`
    pub(crate) fn diode_update_neighbors_in_front(&mut self, pos: BlockPos, s: State) {
        let dir = self.diode_facing(s);
        let op = pos.rel(dir.opposite());
        let block = self.d.block_of(s);
        self.neighbor_changed(op, block);
        self.update_neighbors_at_except_from_facing(op, block, dir);
    }

    fn comparator_calculate_output(&mut self, pos: BlockPos, s: State) -> i32 {
        let input = self.diode_input_signal(s, pos);
        if input == 0 {
            return 0;
        }
        let alt = self.diode_alternate_signal(s, pos);
        if alt > input {
            0
        } else if !self.comparator_compare_mode(s) {
            input - alt
        } else {
            input
        }
    }

    /// `ComparatorBlock.refreshOutputState` (its scheduled tick)
    pub(crate) fn comparator_refresh_output(&mut self, s: State, pos: BlockPos) {
        let out = self.comparator_calculate_output(pos, s);
        let mut old = 0;
        if let Some(be) = self.bes_mut().get_mut(&pos) {
            if let BeData::Comparator { output } = &mut be.data {
                old = *output;
                *output = out;
            }
        }
        if old != out || self.comparator_compare_mode(s) {
            let source_on = self.diode_should_turn_on(pos, s);
            let is_on = self.d.get_bool(s, prop::POWERED);
            if is_on && !source_on {
                let ns = self.d.with_bool(s, prop::POWERED, false);
                self.set_block(pos, ns, 2);
            } else if !is_on && source_on {
                let ns = self.d.with_bool(s, prop::POWERED, true);
                self.set_block(pos, ns, 2);
            }
            self.diode_update_neighbors_in_front(pos, s);
        }
    }

    /// `DiodeBlock.setPlacedBy`
    pub(crate) fn diode_set_placed_by(&mut self, pos: BlockPos, s: State) {
        if self.diode_should_turn_on(pos, s) {
            let b = self.d.block_of(s);
            self.schedule_tick(pos, b, 1, 0);
        }
    }

    /// Player use: repeater cycles delay; comparator toggles mode.
    pub(crate) fn diode_use(&mut self, s: State, pos: BlockPos) -> bool {
        match self.kind(s) {
            Kind::Repeater => {
                let ns = self.d.cycle(s, prop::DELAY);
                self.set_block_and_update(pos, ns);
                true
            }
            Kind::Comparator => {
                let ns = self.d.cycle(s, prop::MODE_COMPARATOR);
                self.set_block(pos, ns, 2);
                if self.is(self.get(pos), blk::COMPARATOR) {
                    self.comparator_refresh_output(ns, pos);
                }
                true
            }
            _ => false,
        }
    }
}
