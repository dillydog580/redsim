//! Redstone dust: `RedstoneWireBlock` + `DefaultRedstoneWireEvaluator` (26.3, no
//! Redstone Experiments). Power changes notify neighbours in Java HashSet order,
//! which makes dust behaviour location dependent; that order is reproduced exactly.

use crate::data::{BlockId, Kind, State, AIR};
use crate::gen::{blk, prop};
use crate::javahash::hashset_order7;
use crate::pos::{BlockPos, Dir};
use crate::world::World;

/// RedstoneSide values in property order: up, side, none
const UP: u32 = 0;
const SIDE: u32 = 1;
const NONE: u32 = 2;

#[inline]
fn side_prop(d: Dir) -> u16 {
    match d {
        Dir::North => prop::NORTH_REDSTONE,
        Dir::East => prop::EAST_REDSTONE,
        Dir::South => prop::SOUTH_REDSTONE,
        Dir::West => prop::WEST_REDSTONE,
        _ => unreachable!(),
    }
}

impl World {
    #[inline]
    fn side(&self, s: State, d: Dir) -> u32 {
        self.d.get_idx(s, side_prop(d)).unwrap()
    }
    #[inline]
    fn set_side(&self, s: State, d: Dir, v: u32) -> State {
        self.d.with_idx(s, side_prop(d), v)
    }
    #[inline]
    fn connected(&self, s: State, d: Dir) -> bool {
        self.side(s, d) != NONE
    }
    fn is_cross(&self, s: State) -> bool {
        Dir::HORIZONTAL.iter().all(|&d| self.connected(s, d))
    }
    fn is_dot(&self, s: State) -> bool {
        Dir::HORIZONTAL.iter().all(|&d| !self.connected(s, d))
    }
    fn wire_cross_state(&self) -> State {
        let mut s = self.d.default_state(blk::REDSTONE_WIRE);
        for d in Dir::HORIZONTAL {
            s = self.set_side(s, d, SIDE);
        }
        s
    }

    /// Dust positions that feed power into the dust at `pos`, by the exact geometry of
    /// `RedstoneWireEvaluator.getIncomingWireSignal` (same level; up a step only when the
    /// side block conducts and the block above `pos` does not; down a step when the side block
    /// does not conduct). Edges can be one-way (e.g. dust stairs on glass).
    pub fn dust_inputs(&self, pos: BlockPos) -> Vec<BlockPos> {
        let mut out = Vec::new();
        let above_conducts = self.is_conductor(self.get(pos.above()));
        for d in Dir::HORIZONTAL {
            let np = pos.rel(d);
            let ns = self.get(np);
            if self.is(ns, blk::REDSTONE_WIRE) {
                out.push(np);
            }
            if self.is_conductor(ns) && !above_conducts {
                if self.is(self.get(np.above()), blk::REDSTONE_WIRE) {
                    out.push(np.above());
                }
            } else if !self.is_conductor(ns) && self.is(self.get(np.below()), blk::REDSTONE_WIRE) {
                out.push(np.below());
            }
        }
        out
    }

    /// Positions the dust at `pos` sends power to (ignoring its current power level): the block
    /// below it, and each side its connection shape points into.
    pub fn dust_outputs(&self, pos: BlockPos) -> Vec<BlockPos> {
        let s = self.get(pos);
        if !self.is(s, blk::REDSTONE_WIRE) {
            return Vec::new();
        }
        let cs = self.wire_connection_state(s, pos);
        let mut out = vec![pos.below()];
        for d in Dir::HORIZONTAL {
            if self.connected(cs, d) {
                out.push(pos.rel(d));
            }
        }
        out
    }

    /// `RedstoneWireBlock.getStateForPlacement`: a player-placed dust starts as a cross and takes
    /// the connections its neighbours give it (a lone dust stays a cross).
    pub fn dust_placement_state(&self, pos: BlockPos, power: i32) -> State {
        let cross = self.d.with_int(self.wire_cross_state(), prop::POWER, power);
        self.wire_connection_state(cross, pos)
    }

    pub(crate) fn wire_can_survive_on(&self, below: State, below_pos: BlockPos) -> bool {
        self.sturdy_full(below, below_pos, Dir::Up) || self.is(below, blk::HOPPER)
    }

    /// `RedstoneWireBlock.getConnectionState`
    pub(crate) fn wire_connection_state(&self, state: State, pos: BlockPos) -> State {
        let was_dot = self.is_dot(state);
        let power = self.d.get_int(state, prop::POWER);
        let base = self.d.with_int(self.d.default_state(blk::REDSTONE_WIRE), prop::POWER, power);
        let mut s = self.wire_missing_connections(base, pos);
        if was_dot && self.is_dot(s) {
            return s;
        }
        let north = self.connected(s, Dir::North);
        let south = self.connected(s, Dir::South);
        let east = self.connected(s, Dir::East);
        let west = self.connected(s, Dir::West);
        let ns_empty = !north && !south;
        let ew_empty = !east && !west;
        if !west && ns_empty {
            s = self.set_side(s, Dir::West, SIDE);
        }
        if !east && ns_empty {
            s = self.set_side(s, Dir::East, SIDE);
        }
        if !north && ew_empty {
            s = self.set_side(s, Dir::North, SIDE);
        }
        if !south && ew_empty {
            s = self.set_side(s, Dir::South, SIDE);
        }
        s
    }

    fn wire_missing_connections(&self, mut s: State, pos: BlockPos) -> State {
        let can_connect_up = !self.is_conductor(self.get(pos.above()));
        for d in Dir::HORIZONTAL {
            if !self.connected(s, d) {
                let side = self.wire_connecting_side_up(pos, d, can_connect_up);
                s = self.set_side(s, d, side);
            }
        }
        s
    }

    fn wire_connecting_side(&self, pos: BlockPos, d: Dir) -> u32 {
        let can_up = !self.is_conductor(self.get(pos.above()));
        self.wire_connecting_side_up(pos, d, can_up)
    }

    fn wire_connecting_side_up(&self, pos: BlockPos, d: Dir, can_connect_up: bool) -> u32 {
        let rp = pos.rel(d);
        let rs = self.get(rp);
        if can_connect_up {
            let placeable_above = self.d.bflags(self.d.block_of(rs)) & crate::data::bflag::TRAPDOOR != 0
                || self.wire_can_survive_on(rs, rp);
            if placeable_above && self.should_connect_to_pos(rp.above()) {
                if self.sturdy_full(rs, rp, d.opposite()) {
                    return UP;
                }
                return SIDE;
            }
        }
        if !self.should_wire_connect_to(rs, rp, Some(d)) && (self.is_conductor(rs) || !self.should_connect_to_pos(rp.below())) {
            NONE
        } else {
            SIDE
        }
    }

    /// `wire_connecting_side_up(pos, d, can_connect_up) != NONE`, without telling UP from SIDE.
    #[inline]
    fn wire_side_connected(&self, pos: BlockPos, d: Dir, can_connect_up: bool) -> bool {
        let rp = pos.rel(d);
        let rs = self.get(rp);
        if can_connect_up {
            let placeable_above = self.d.bflags(self.d.block_of(rs)) & crate::data::bflag::TRAPDOOR != 0
                || self.wire_can_survive_on(rs, rp);
            if placeable_above && self.should_connect_to_pos(rp.above()) {
                return true;
            }
        }
        self.should_wire_connect_to(rs, rp, Some(d)) || (!self.is_conductor(rs) && self.should_connect_to_pos(rp.below()))
    }

    /// Whether `wire_connection_state(s, pos)` is connected towards `t`. Same answer, but it
    /// only looks at the sides that decide it: the connection state is a pure function of the
    /// world, and side `t` ends up connected iff it connects itself, or neither perpendicular
    /// side does (the dust is then a line through `t`, or a cross unless it was a dot).
    fn wire_connects_towards(&self, s: State, pos: BlockPos, t: Dir) -> bool {
        let can_up = !self.is_conductor(self.get(pos.above()));
        if self.wire_side_connected(pos, t, can_up) {
            return true;
        }
        let p = t.cw();
        if self.wire_side_connected(pos, p, can_up) || self.wire_side_connected(pos, p.opposite(), can_up) {
            return false;
        }
        // nothing connects across t's axis: a dot stays a dot only if t's opposite is free too
        !self.is_dot(s) || self.wire_side_connected(pos, t.opposite(), can_up)
    }

    #[inline]
    fn should_connect_to_pos(&self, p: BlockPos) -> bool {
        let s = self.get(p);
        self.should_wire_connect_to(s, p, None)
    }

    pub(crate) fn wire_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        if dir == Dir::Down {
            return if !self.wire_can_survive_on(ns, npos) { AIR } else { s };
        }
        if dir == Dir::Up {
            return self.wire_connection_state(s, pos);
        }
        let side = self.wire_connecting_side(pos, dir);
        let side_connected = side != NONE;
        if side_connected == self.connected(s, dir) && !self.is_cross(s) {
            self.set_side(s, dir, side)
        } else {
            let power = self.d.get_int(s, prop::POWER);
            let cross = self.d.with_int(self.wire_cross_state(), prop::POWER, power);
            let st = self.set_side(cross, dir, side);
            self.wire_connection_state(st, pos)
        }
    }

    pub(crate) fn wire_update_indirect_neighbour_shapes(&mut self, s: State, pos: BlockPos, fl: i32, limit: i32) {
        for d in Dir::HORIZONTAL {
            let v = self.side(s, d);
            let p = pos.rel(d);
            if v != NONE && !self.is(self.get(p), blk::REDSTONE_WIRE) {
                let down = p.below();
                if self.is(self.get(down), blk::REDSTONE_WIRE) {
                    let np = down.rel(d.opposite());
                    let ns = self.get(np);
                    self.neighbor_shape_changed(d.opposite(), down, np, ns, fl, limit);
                }
                let up = p.above();
                if self.is(self.get(up), blk::REDSTONE_WIRE) {
                    let np = up.rel(d.opposite());
                    let ns = self.get(np);
                    self.neighbor_shape_changed(d.opposite(), up, np, ns, fl, limit);
                }
            }
        }
    }

    /// `RedstoneWireBlock.getSignal`
    pub(crate) fn wire_get_signal(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if self.wire_should_signal && dir != Dir::Down {
            let power = self.d.get_int(s, prop::POWER);
            if power == 0 {
                return 0;
            }
            if dir != Dir::Up && !self.wire_connects_towards(s, pos, dir.opposite()) {
                0
            } else {
                power
            }
        } else {
            0
        }
    }

    /// `RedstoneWireBlock.getBlockSignal`: best signal from non-wire neighbours.
    fn wire_block_signal(&mut self, pos: BlockPos) -> i32 {
        self.wire_should_signal = false;
        let s = self.get_best_neighbor_signal(pos);
        self.wire_should_signal = true;
        s
    }

    #[inline]
    fn wire_power_at(&self, p: BlockPos) -> i32 {
        let s = self.get(p);
        if self.is(s, blk::REDSTONE_WIRE) {
            self.d.get_int(s, prop::POWER)
        } else {
            0
        }
    }

    /// `RedstoneWireEvaluator.getIncomingWireSignal`
    fn wire_incoming_signal(&self, pos: BlockPos) -> i32 {
        let mut w = 0;
        // the game re-reads the block above for every side; it cannot change in between
        let above_conducts = self.is_conductor(self.get(pos.above()));
        for d in Dir::HORIZONTAL {
            let np = pos.rel(d);
            let ns = self.get(np);
            w = w.max(if self.is(ns, blk::REDSTONE_WIRE) { self.d.get_int(ns, prop::POWER) } else { 0 });
            if self.is_conductor(ns) && !above_conducts {
                w = w.max(self.wire_power_at(np.above()));
            } else if !self.is_conductor(ns) {
                w = w.max(self.wire_power_at(np.below()));
            }
        }
        (w - 1).max(0)
    }

    /// `DefaultRedstoneWireEvaluator.updatePowerStrength`; returns false if it did nothing
    /// (the power is already right).
    pub(crate) fn wire_update_power_strength(&mut self, pos: BlockPos, state: State) -> bool {
        let block_signal = self.wire_block_signal(pos);
        let target = if block_signal == 15 { 15 } else { block_signal.max(self.wire_incoming_signal(pos)) };
        if self.d.get_int(state, prop::POWER) == target {
            return false;
        }
        {
            if self.get(pos) == state {
                let ns = self.d.with_int(state, prop::POWER, target);
                self.set_block(pos, ns, 2);
            }
            let mut items = [pos; 7];
            for (i, d) in Dir::ALL.iter().enumerate() {
                items[i + 1] = pos.rel(*d);
            }
            for p in hashset_order7(&items) {
                self.update_neighbors_at(p, blk::REDSTONE_WIRE);
            }
        }
        true
    }

    fn wire_check_corner_change_at(&mut self, pos: BlockPos) {
        if self.is(self.get(pos), blk::REDSTONE_WIRE) {
            self.update_neighbors_at(pos, blk::REDSTONE_WIRE);
            for d in Dir::ALL {
                self.update_neighbors_at(pos.rel(d), blk::REDSTONE_WIRE);
            }
        }
    }

    pub(crate) fn wire_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if !self.is(old, blk::REDSTONE_WIRE) {
            self.wire_update_power_strength(pos, s);
            for d in Dir::VERTICAL {
                self.update_neighbors_at(pos.rel(d), blk::REDSTONE_WIRE);
            }
            self.wire_update_neighbors_of_neighboring_wires(pos);
        }
    }

    pub(crate) fn wire_affect_neighbors_after_removal(&mut self, s: State, pos: BlockPos, moved: bool) {
        if !moved {
            for d in Dir::ALL {
                self.update_neighbors_at(pos.rel(d), blk::REDSTONE_WIRE);
            }
            self.wire_update_power_strength(pos, s);
            self.wire_update_neighbors_of_neighboring_wires(pos);
        }
    }

    fn wire_update_neighbors_of_neighboring_wires(&mut self, pos: BlockPos) {
        for d in Dir::HORIZONTAL {
            self.wire_check_corner_change_at(pos.rel(d));
        }
        for d in Dir::HORIZONTAL {
            let t = pos.rel(d);
            if self.is_conductor(self.get(t)) {
                self.wire_check_corner_change_at(t.above());
            } else {
                self.wire_check_corner_change_at(t.below());
            }
        }
    }

    /// Returns true if nothing changed (the dust survives and its power is already right).
    pub(crate) fn wire_neighbor_changed(&mut self, s: State, pos: BlockPos, _block: BlockId, _moved: bool) -> bool {
        if self.can_survive(s, pos) {
            !self.wire_update_power_strength(pos, s)
        } else {
            // dropResources: item entities are not simulated
            self.remove_block(pos, false);
            false
        }
    }

    /// `RedstoneWireBlock.useWithoutItem`: toggle dot <-> cross.
    pub(crate) fn wire_use(&mut self, s: State, pos: BlockPos) -> bool {
        if self.is_cross(s) || self.is_dot(s) {
            let base = if self.is_cross(s) { self.d.default_state(blk::REDSTONE_WIRE) } else { self.wire_cross_state() };
            let power = self.d.get_int(s, prop::POWER);
            let ns = self.d.with_int(base, prop::POWER, power);
            let ns = self.wire_connection_state(ns, pos);
            if ns != s {
                self.set_block_and_update(pos, ns);
                // updatesOnShapeChange
                for d in Dir::HORIZONTAL {
                    let rp = pos.rel(d);
                    if self.connected(s, d) != self.connected(ns, d) && self.is_conductor(self.get(rp)) {
                        self.update_neighbors_at_except_from_facing(rp, blk::REDSTONE_WIRE, d.opposite());
                    }
                }
                return true;
            }
        }
        false
    }

    #[allow(dead_code)]
    fn _kind_check(&self, s: State) -> bool {
        self.kind(s) == Kind::Wire
    }
}

#[cfg(test)]
mod tests {
    use crate::data::{data, State};
    use crate::gen::blk;
    use crate::pos::{BlockPos, Dir};
    use crate::world::World;

    /// The lazy connection check used by `wire_get_signal` must agree with the full
    /// `getConnectionState` on every neighbourhood: random states of every block around dust.
    #[test]
    fn connects_towards_matches_connection_state() {
        let d = data();
        assert!(World::new().is_dot(d.default_state(blk::REDSTONE_WIRE)), "default dust must be a dot");
        let n = d.states.len() as u64;
        let wire = d.binfo(blk::REDSTONE_WIRE);
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        // common neighbours often, any state sometimes
        let common: Vec<State> = ["stone", "glass", "redstone_wire", "redstone_block", "oak_trapdoor[half=top]", "hopper",
            "smooth_stone_slab[type=top]", "observer", "repeater", "air", "air", "air", "lever", "redstone_torch",
            "comparator", "target", "piston[facing=up]"]
            .iter()
            .map(|t| d.parse_state(t).unwrap())
            .collect();
        let c = BlockPos::new(3, 64, -5);
        let mut checked = 0u64;
        for _ in 0..40_000 {
            let mut w = World::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let p = c.offset(dx, dy, dz);
                        let r = rnd();
                        let s = if r % 3 == 0 { (rnd() % n) as State } else { common[(rnd() % common.len() as u64) as usize] };
                        w.place_raw(p, s);
                    }
                }
            }
            let s = wire.first + (rnd() % wire.n as u64) as u32;
            w.place_raw(c, s);
            let full = w.wire_connection_state(s, c);
            for t in Dir::HORIZONTAL {
                assert_eq!(w.wire_connects_towards(s, c, t), w.connected(full, t), "state {} towards {:?}", d.format_state(s), t);
                checked += 1;
            }
        }
        assert!(checked > 100_000);
    }
}
