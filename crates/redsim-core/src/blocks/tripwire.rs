//! Tripwire hooks and tripwire (`TripWireHookBlock`, `TripWireBlock`). No entities are simulated,
//! so a wire is only ever released (a wire placed or pasted with `powered=true` clears itself).

use crate::data::State;
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::world::World;

const WIRE_DIST_MAX: i32 = 42;

impl World {
    fn tw_dir_prop(d: Dir) -> u16 {
        match d {
            Dir::North => prop::NORTH,
            Dir::East => prop::EAST,
            Dir::South => prop::SOUTH,
            _ => prop::WEST,
        }
    }

    /// `TripWireBlock.shouldConnectTo`
    pub(crate) fn tripwire_should_connect(&self, ns: State, d: Dir) -> bool {
        let b = self.d.block_of(ns);
        if b == blk::TRIPWIRE_HOOK {
            self.d.get_dir(ns, prop::HORIZONTAL_FACING) == d.opposite()
        } else {
            b == blk::TRIPWIRE
        }
    }

    /// `TripWireBlock.getStateForPlacement` (connections to neighbouring wire and hooks).
    pub fn tripwire_placement_state(&self, pos: BlockPos, s: State) -> State {
        let mut st = self.d.default_state(blk::TRIPWIRE);
        let _ = s;
        for d in [Dir::North, Dir::East, Dir::South, Dir::West] {
            let c = self.tripwire_should_connect(self.get(pos.rel(d)), d);
            st = self.d.with_bool(st, Self::tw_dir_prop(d), c);
        }
        st
    }

    pub(crate) fn tripwire_update_shape(&mut self, s: State, dir: Dir, ns: State) -> State {
        if dir.is_horizontal() {
            let c = self.tripwire_should_connect(ns, dir);
            self.d.with_bool(s, Self::tw_dir_prop(dir), c)
        } else {
            s
        }
    }

    pub(crate) fn tripwire_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if self.d.block_of(old) != blk::TRIPWIRE {
            self.tripwire_update_source(pos, s);
        }
    }

    pub(crate) fn tripwire_affect_after_removal(&mut self, s: State, pos: BlockPos, moved: bool) {
        if !moved {
            let ps = self.d.with_bool(s, prop::POWERED, true);
            self.tripwire_update_source(pos, ps);
        }
    }

    /// `TripWireBlock.updateSource`
    fn tripwire_update_source(&mut self, pos: BlockPos, s: State) {
        for d in [Dir::South, Dir::West] {
            for i in 1..WIRE_DIST_MAX {
                let tp = pos.rel_n(d, i);
                let b = self.get(tp);
                let bb = self.d.block_of(b);
                if bb == blk::TRIPWIRE_HOOK {
                    if self.d.get_dir(b, prop::HORIZONTAL_FACING) == d.opposite() {
                        self.hook_calculate_state(tp, b, false, true, i, Some(s));
                    }
                    break;
                }
                if bb != blk::TRIPWIRE {
                    break;
                }
            }
        }
    }

    /// `TripWireBlock.tick` -> `checkPressed` with no entities: a pressed wire is released.
    pub(crate) fn tripwire_tick(&mut self, pos: BlockPos) {
        let s = self.get(pos);
        if !self.d.get_bool(s, prop::POWERED) {
            return;
        }
        let ns = self.d.with_bool(s, prop::POWERED, false);
        self.set_block_and_update(pos, ns);
        self.tripwire_update_source(pos, ns);
        // wasPressed -> recheck in 1 tick
        self.schedule_tick(pos, blk::TRIPWIRE, 1, 0);
    }

    // ----- hook ---------------------------------------------------------------------------

    pub(crate) fn hook_can_survive(&self, s: State, pos: BlockPos) -> bool {
        let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
        let rp = pos.rel(f.opposite());
        f.is_horizontal() && self.sturdy_full(self.get(rp), rp, f)
    }

    fn hook_notify_neighbors(&mut self, pos: BlockPos, d: Dir) {
        let front = d.opposite();
        self.update_neighbors_at(pos, blk::TRIPWIRE_HOOK);
        self.update_neighbors_at(pos.rel(front), blk::TRIPWIRE_HOOK);
    }

    /// `TripWireHookBlock.calculateState`
    pub(crate) fn hook_calculate_state(
        &mut self,
        pos: BlockPos,
        state: State,
        being_destroyed: bool,
        can_update: bool,
        wire_source: i32,
        wire_source_state: Option<State>,
    ) {
        if !self.d.has_prop(state, prop::HORIZONTAL_FACING) {
            return;
        }
        let direction = self.d.get_dir(state, prop::HORIZONTAL_FACING);
        let was_attached = self.d.has_prop(state, prop::ATTACHED) && self.d.get_bool(state, prop::ATTACHED);
        let was_powered = self.d.has_prop(state, prop::POWERED) && self.d.get_bool(state, prop::POWERED);
        let block = self.d.block_of(state);
        let mut attached = !being_destroyed;
        let mut powered = false;
        let mut receiver = 0;
        let mut wire_states: [Option<State>; WIRE_DIST_MAX as usize] = [None; WIRE_DIST_MAX as usize];
        for i in 1..WIRE_DIST_MAX {
            let tp = pos.rel_n(direction, i);
            let mut ws = self.get(tp);
            let wb = self.d.block_of(ws);
            if wb == blk::TRIPWIRE_HOOK {
                if self.d.get_dir(ws, prop::HORIZONTAL_FACING) == direction.opposite() {
                    receiver = i;
                }
                break;
            }
            if wb != blk::TRIPWIRE && i != wire_source {
                wire_states[i as usize] = None;
                attached = false;
            } else {
                if i == wire_source {
                    ws = wire_source_state.unwrap_or(ws);
                }
                let armed = !self.d.get_bool(ws, prop::DISARMED);
                let wp = self.d.get_bool(ws, prop::POWERED);
                powered |= armed && wp;
                wire_states[i as usize] = Some(ws);
                if i == wire_source {
                    self.schedule_tick(pos, block, 10, 0);
                    attached &= armed;
                }
            }
        }
        attached &= receiver > 1;
        powered &= attached;
        let base = self.d.default_state(block);
        let new_state = self.d.with_bool(self.d.with_bool(base, prop::ATTACHED, attached), prop::POWERED, powered);
        if receiver > 0 {
            let tp = pos.rel_n(direction, receiver);
            let opp = direction.opposite();
            let rs = self.d.with_dir(new_state, prop::HORIZONTAL_FACING, opp);
            self.set_block_and_update(tp, rs);
            self.hook_notify_neighbors(tp, opp);
            if self.d.block_of(self.get(pos)) != blk::TRIPWIRE_HOOK {
                // onRemoved(newState): note newState still has the default facing (north)
                self.hook_on_removed(new_state, pos);
                return;
            }
            self.hook_emit_state(tp, attached, powered, was_attached, was_powered);
        }
        self.hook_emit_state(pos, attached, powered, was_attached, was_powered);
        if !being_destroyed {
            let ms = self.d.with_dir(new_state, prop::HORIZONTAL_FACING, direction);
            self.set_block_and_update(pos, ms);
            if can_update {
                self.hook_notify_neighbors(pos, direction);
            }
        }
        if was_attached != attached {
            for i in 1..receiver {
                let tp = pos.rel_n(direction, i);
                if let Some(wd) = wire_states[i as usize] {
                    let cur = self.d.block_of(self.get(tp));
                    if cur == blk::TRIPWIRE || cur == blk::TRIPWIRE_HOOK {
                        let ns = if self.d.has_prop(wd, prop::ATTACHED) { self.d.with_bool(wd, prop::ATTACHED, attached) } else { wd };
                        self.set_block_and_update(tp, ns);
                    }
                }
            }
        }
    }

    /// `TripWireHookBlock.emitState` (sounds and game events)
    fn hook_emit_state(&mut self, pos: BlockPos, attached: bool, powered: bool, was_attached: bool, was_powered: bool) {
        use crate::vibration::GameEvent;
        let ev = if powered && !was_powered {
            GameEvent::BlockActivate
        } else if !powered && was_powered {
            GameEvent::BlockDeactivate
        } else if attached && !was_attached {
            GameEvent::BlockAttach
        } else if !attached && was_attached {
            GameEvent::BlockDetach
        } else {
            return;
        };
        self.game_event(ev, pos, None);
    }

    fn hook_on_removed(&mut self, s: State, pos: BlockPos) {
        let attached = self.d.get_bool(s, prop::ATTACHED);
        let powered = self.d.get_bool(s, prop::POWERED);
        if attached || powered {
            self.hook_calculate_state(pos, s, true, false, -1, None);
        }
        if powered {
            let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
            self.hook_notify_neighbors(pos, f);
        }
    }

    pub(crate) fn hook_affect_after_removal(&mut self, s: State, pos: BlockPos, moved: bool) {
        if !moved {
            self.hook_on_removed(s, pos);
        }
    }

    pub(crate) fn hook_tick(&mut self, s: State, pos: BlockPos) {
        self.hook_calculate_state(pos, s, false, true, -1, None);
    }

    /// `TripWireHookBlock.setPlacedBy`
    pub(crate) fn hook_set_placed_by(&mut self, pos: BlockPos, s: State) {
        self.hook_calculate_state(pos, s, false, false, -1, None);
    }
}
