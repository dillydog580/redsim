//! Rails: `BaseRailBlock`, `RailState` (shape/connection solver), `RailBlock`,
//! `PoweredRailBlock` (powered and activator rails) and `DetectorRailBlock` (no minecarts are
//! simulated, so a detector rail is only ever un-pressed).

use crate::data::{Kind, State};
use crate::gen::{blk, prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::shape::Support;
use crate::world::World;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Shape {
    NS,
    EW,
    AscE,
    AscW,
    AscN,
    AscS,
    SE,
    SW,
    NW,
    NE,
}

/// Shapes in the order the game lists the `shape` values of rails (both properties start with
/// the six straight shapes); `rail_shapes_follow_property_order` checks it.
const SHAPES: [Shape; 10] =
    [Shape::NS, Shape::EW, Shape::AscE, Shape::AscW, Shape::AscN, Shape::AscS, Shape::SE, Shape::SW, Shape::NW, Shape::NE];

impl Shape {
    fn name(self) -> &'static str {
        match self {
            Shape::NS => "north_south",
            Shape::EW => "east_west",
            Shape::AscE => "ascending_east",
            Shape::AscW => "ascending_west",
            Shape::AscN => "ascending_north",
            Shape::AscS => "ascending_south",
            Shape::SE => "south_east",
            Shape::SW => "south_west",
            Shape::NW => "north_west",
            Shape::NE => "north_east",
        }
    }
    #[cfg_attr(not(test), allow(dead_code))]
    fn parse(s: &str) -> Shape {
        match s {
            "north_south" => Shape::NS,
            "east_west" => Shape::EW,
            "ascending_east" => Shape::AscE,
            "ascending_west" => Shape::AscW,
            "ascending_north" => Shape::AscN,
            "ascending_south" => Shape::AscS,
            "south_east" => Shape::SE,
            "south_west" => Shape::SW,
            "north_west" => Shape::NW,
            _ => Shape::NE,
        }
    }
    fn is_slope(self) -> bool {
        matches!(self, Shape::AscE | Shape::AscW | Shape::AscN | Shape::AscS)
    }
}

#[derive(Clone, Debug)]
struct RailState {
    pos: BlockPos,
    state: State,
    straight: bool,
    conns: Vec<BlockPos>,
}

impl World {
    pub(crate) fn is_rail_state(&self, s: State) -> bool {
        let b = self.d.block_of(s);
        self.d.has_tag(b, tag::RAILS) && self.d.bflags(b) & crate::data::bflag::RAIL != 0
    }

    fn shape_prop(&self, s: State) -> u16 {
        if self.d.has_prop(s, prop::RAIL_SHAPE) {
            prop::RAIL_SHAPE
        } else {
            prop::RAIL_SHAPE_STRAIGHT
        }
    }

    fn rail_shape(&self, s: State) -> Shape {
        SHAPES[self.d.get_idx(s, self.shape_prop(s)).expect("rail shape") as usize]
    }

    fn with_shape(&self, s: State, sh: Shape) -> State {
        let p = self.shape_prop(s);
        let idx = SHAPES.iter().position(|&x| x == sh).unwrap() as u32;
        assert!((idx as usize) < self.d.props[p as usize].values.len(), "shape {} not allowed", sh.name());
        self.d.with_idx(s, p, idx)
    }

    fn rs_new(&self, pos: BlockPos, state: State) -> RailState {
        let mut rs = RailState { pos, state, straight: self.d.block_of(state) != blk::RAIL, conns: Vec::new() };
        let sh = self.rail_shape(state);
        Self::rs_update_connections(&mut rs, sh);
        rs
    }

    fn rs_update_connections(rs: &mut RailState, sh: Shape) {
        let p = rs.pos;
        rs.conns.clear();
        let (a, b) = match sh {
            Shape::NS => (p.rel(Dir::North), p.rel(Dir::South)),
            Shape::EW => (p.rel(Dir::West), p.rel(Dir::East)),
            Shape::AscE => (p.rel(Dir::West), p.rel(Dir::East).above()),
            Shape::AscW => (p.rel(Dir::West).above(), p.rel(Dir::East)),
            Shape::AscN => (p.rel(Dir::North).above(), p.rel(Dir::South)),
            Shape::AscS => (p.rel(Dir::North), p.rel(Dir::South).above()),
            Shape::SE => (p.rel(Dir::East), p.rel(Dir::South)),
            Shape::SW => (p.rel(Dir::West), p.rel(Dir::South)),
            Shape::NW => (p.rel(Dir::West), p.rel(Dir::North)),
            Shape::NE => (p.rel(Dir::East), p.rel(Dir::North)),
        };
        rs.conns.push(a);
        rs.conns.push(b);
    }

    fn rs_get_rail(&self, pos: BlockPos) -> Option<RailState> {
        for p in [pos, pos.above(), pos.below()] {
            let s = self.get(p);
            if self.is_rail_state(s) {
                return Some(self.rs_new(p, s));
            }
        }
        None
    }

    fn rs_has_rail(&self, pos: BlockPos) -> bool {
        self.is_rail_state(self.get(pos)) || self.is_rail_state(self.get(pos.above())) || self.is_rail_state(self.get(pos.below()))
    }

    fn rs_has_connection(rs: &RailState, p: BlockPos) -> bool {
        rs.conns.iter().any(|c| c.x == p.x && c.z == p.z)
    }

    fn rs_remove_soft(&self, rs: &mut RailState) {
        let mut i = 0;
        while i < rs.conns.len() {
            match self.rs_get_rail(rs.conns[i]) {
                Some(r) if Self::rs_has_connection(&r, rs.pos) => {
                    rs.conns[i] = r.pos;
                    i += 1;
                }
                _ => {
                    rs.conns.remove(i);
                }
            }
        }
    }

    fn rs_can_connect_to(rs: &RailState, other: &RailState) -> bool {
        Self::rs_has_connection(rs, other.pos) || rs.conns.len() != 2
    }

    fn rs_count_potential(&self, rs: &RailState) -> i32 {
        Dir::HORIZONTAL.iter().filter(|&&d| self.rs_has_rail(rs.pos.rel(d))).count() as i32
    }

    fn rs_connect_to(&mut self, rs: &mut RailState, other: &RailState) {
        rs.conns.push(other.pos);
        let p = rs.pos;
        let (north, south, west, east) = (p.rel(Dir::North), p.rel(Dir::South), p.rel(Dir::West), p.rel(Dir::East));
        let n = Self::rs_has_connection(rs, north);
        let s = Self::rs_has_connection(rs, south);
        let w = Self::rs_has_connection(rs, west);
        let e = Self::rs_has_connection(rs, east);
        let mut shape: Option<Shape> = None;
        if n || s {
            shape = Some(Shape::NS);
        }
        if w || e {
            shape = Some(Shape::EW);
        }
        if !rs.straight {
            if s && e && !n && !w {
                shape = Some(Shape::SE);
            }
            if s && w && !n && !e {
                shape = Some(Shape::SW);
            }
            if n && w && !s && !e {
                shape = Some(Shape::NW);
            }
            if n && e && !s && !w {
                shape = Some(Shape::NE);
            }
        }
        if shape == Some(Shape::NS) {
            if self.is_rail_state(self.get(north.above())) {
                shape = Some(Shape::AscN);
            }
            if self.is_rail_state(self.get(south.above())) {
                shape = Some(Shape::AscS);
            }
        }
        if shape == Some(Shape::EW) {
            if self.is_rail_state(self.get(east.above())) {
                shape = Some(Shape::AscE);
            }
            if self.is_rail_state(self.get(west.above())) {
                shape = Some(Shape::AscW);
            }
        }
        let sh = shape.unwrap_or(Shape::NS);
        rs.state = self.with_shape(rs.state, sh);
        self.set_block_and_update(rs.pos, rs.state);
    }

    fn rs_has_neighbor_rail(&self, rs: &RailState, pos: BlockPos) -> bool {
        match self.rs_get_rail(pos) {
            None => false,
            Some(mut n) => {
                self.rs_remove_soft(&mut n);
                Self::rs_can_connect_to(&n, rs)
            }
        }
    }

    /// `RailState.place`
    fn rs_place(&mut self, rs: &mut RailState, has_signal: bool, first: bool, default: Shape) {
        let p = rs.pos;
        let (north, south, west, east) = (p.rel(Dir::North), p.rel(Dir::South), p.rel(Dir::West), p.rel(Dir::East));
        let n = self.rs_has_neighbor_rail(rs, north);
        let s = self.rs_has_neighbor_rail(rs, south);
        let w = self.rs_has_neighbor_rail(rs, west);
        let e = self.rs_has_neighbor_rail(rs, east);
        let mut shape: Option<Shape> = None;
        let ns = n || s;
        let we = w || e;
        if ns && !we {
            shape = Some(Shape::NS);
        }
        if we && !ns {
            shape = Some(Shape::EW);
        }
        let (se, sw, ne, nw) = (s && e, s && w, n && e, n && w);
        if !rs.straight {
            if se && !n && !w {
                shape = Some(Shape::SE);
            }
            if sw && !n && !e {
                shape = Some(Shape::SW);
            }
            if nw && !s && !e {
                shape = Some(Shape::NW);
            }
            if ne && !s && !w {
                shape = Some(Shape::NE);
            }
        }
        if shape.is_none() {
            if ns && we {
                shape = Some(default);
            } else if ns {
                shape = Some(Shape::NS);
            } else if we {
                shape = Some(Shape::EW);
            }
            if !rs.straight {
                if has_signal {
                    if se {
                        shape = Some(Shape::SE);
                    }
                    if sw {
                        shape = Some(Shape::SW);
                    }
                    if ne {
                        shape = Some(Shape::NE);
                    }
                    if nw {
                        shape = Some(Shape::NW);
                    }
                } else {
                    if nw {
                        shape = Some(Shape::NW);
                    }
                    if ne {
                        shape = Some(Shape::NE);
                    }
                    if sw {
                        shape = Some(Shape::SW);
                    }
                    if se {
                        shape = Some(Shape::SE);
                    }
                }
            }
        }
        if shape == Some(Shape::NS) {
            if self.is_rail_state(self.get(north.above())) {
                shape = Some(Shape::AscN);
            }
            if self.is_rail_state(self.get(south.above())) {
                shape = Some(Shape::AscS);
            }
        }
        if shape == Some(Shape::EW) {
            if self.is_rail_state(self.get(east.above())) {
                shape = Some(Shape::AscE);
            }
            if self.is_rail_state(self.get(west.above())) {
                shape = Some(Shape::AscW);
            }
        }
        let sh = shape.unwrap_or(default);
        Self::rs_update_connections(rs, sh);
        rs.state = self.with_shape(rs.state, sh);
        if first || self.get(p) != rs.state {
            self.set_block_and_update(p, rs.state);
            let conns = rs.conns.clone();
            for c in conns {
                if let Some(mut nb) = self.rs_get_rail(c) {
                    self.rs_remove_soft(&mut nb);
                    if Self::rs_can_connect_to(&nb, rs) {
                        self.rs_connect_to(&mut nb, rs);
                    }
                }
            }
        }
    }

    /// `BaseRailBlock.updateDir`
    fn rail_update_dir(&mut self, pos: BlockPos, s: State, first: bool) -> State {
        let mut rs = self.rs_new(pos, s);
        let sig = self.has_neighbor_signal(pos);
        let current = self.rail_shape(s);
        self.rs_place(&mut rs, sig, first, current);
        rs.state
    }

    /// `BaseRailBlock.updateState(state, level, pos, movedByPiston)` (used by onPlace)
    fn rail_update_state_placed(&mut self, s: State, pos: BlockPos, moved: bool) -> State {
        let ns = self.rail_update_dir(pos, s, true);
        if self.d.block_of(s) != blk::RAIL {
            let b = self.d.block_of(s);
            self.neighbor_changed_full(ns, pos, b, moved);
        }
        ns
    }

    pub(crate) fn rail_can_survive(&self, pos: BlockPos) -> bool {
        let b = pos.below();
        self.sturdy(self.get(b), b, Dir::Up, Support::Rigid)
    }

    fn rail_should_be_removed(&self, pos: BlockPos, sh: Shape) -> bool {
        if !self.rail_can_survive(pos) {
            return true;
        }
        let side = match sh {
            Shape::AscE => Some(Dir::East),
            Shape::AscW => Some(Dir::West),
            Shape::AscN => Some(Dir::North),
            Shape::AscS => Some(Dir::South),
            _ => None,
        };
        match side {
            Some(d) => {
                let p = pos.rel(d);
                !self.sturdy(self.get(p), p, Dir::Up, Support::Rigid)
            }
            None => false,
        }
    }

    pub(crate) fn rail_on_place(&mut self, s: State, pos: BlockPos, old: State, moved: bool) {
        if self.d.block_of(old) == self.d.block_of(s) {
            return;
        }
        let ns = self.rail_update_state_placed(s, pos, moved);
        if self.kind(s) == Kind::DetectorRail {
            self.detector_check_pressed(pos, ns);
        }
    }

    /// Returns true (meaningful for powered and activator rails) if nothing changed for a reason
    /// that depends only on the world's blocks: the rail is gone, or its power is already right.
    pub(crate) fn rail_neighbor_changed(&mut self, s: State, pos: BlockPos, block: crate::data::BlockId, moved: bool) -> bool {
        if self.d.block_of(self.get(pos)) != self.d.block_of(s) {
            return true;
        }
        let sh = self.rail_shape(s);
        if self.rail_should_be_removed(pos, sh) {
            self.remove_block(pos, moved);
            return false;
        }
        match self.kind(s) {
            Kind::Rail => {
                // RailBlock.updateState: re-solve the shape at junctions when a signal source updates it
                let src_default = self.d.default_state(block);
                if self.is_signal_source(src_default) {
                    let rs = self.rs_new(pos, s);
                    if self.rs_count_potential(&rs) == 3 {
                        self.rail_update_dir(pos, s, false);
                    }
                }
            }
            Kind::PoweredRail => return self.powered_rail_update(s, pos),
            _ => {}
        }
        false
    }

    pub(crate) fn rail_affect_after_removal(&mut self, s: State, pos: BlockPos, moved: bool) {
        if moved {
            return;
        }
        let b = self.d.block_of(s);
        if self.rail_shape(s).is_slope() {
            self.update_neighbors_at(pos.above(), b);
        }
        if b != blk::RAIL {
            self.update_neighbors_at(pos, b);
            self.update_neighbors_at(pos.below(), b);
        }
    }

    // ----- powered / activator rails -----------------------------------------------------

    fn find_powered_rail_signal(&self, pos: BlockPos, s: State, forward: bool, depth: i32) -> bool {
        if depth >= 8 {
            return false;
        }
        let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
        let mut check_below = true;
        let mut sh = self.rail_shape(s);
        match sh {
            Shape::NS => z += if forward { 1 } else { -1 },
            Shape::EW => x += if forward { -1 } else { 1 },
            Shape::AscE => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                    y += 1;
                    check_below = false;
                }
                sh = Shape::EW;
            }
            Shape::AscW => {
                if forward {
                    x -= 1;
                    y += 1;
                    check_below = false;
                } else {
                    x += 1;
                }
                sh = Shape::EW;
            }
            Shape::AscN => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                    y += 1;
                    check_below = false;
                }
                sh = Shape::NS;
            }
            Shape::AscS => {
                if forward {
                    z += 1;
                    y += 1;
                    check_below = false;
                } else {
                    z -= 1;
                }
                sh = Shape::NS;
            }
            _ => {}
        }
        let block = self.d.block_of(s);
        self.same_rail_with_power(BlockPos::new(x, y, z), block, forward, depth, sh)
            || (check_below && self.same_rail_with_power(BlockPos::new(x, y - 1, z), block, forward, depth, sh))
    }

    fn same_rail_with_power(&self, pos: BlockPos, block: crate::data::BlockId, forward: bool, depth: i32, dir: Shape) -> bool {
        let s = self.get(pos);
        if self.d.block_of(s) != block {
            return false;
        }
        let my = self.rail_shape(s);
        if dir == Shape::EW && matches!(my, Shape::NS | Shape::AscN | Shape::AscS) {
            return false;
        }
        if dir == Shape::NS && matches!(my, Shape::EW | Shape::AscE | Shape::AscW) {
            return false;
        }
        if !self.d.get_bool(s, prop::POWERED) {
            return false;
        }
        self.has_neighbor_signal(pos) || self.find_powered_rail_signal(pos, s, forward, depth + 1)
    }

    /// Returns true if the power was already right (nothing done).
    fn powered_rail_update(&mut self, s: State, pos: BlockPos) -> bool {
        let is_powered = self.d.get_bool(s, prop::POWERED);
        let should = self.has_neighbor_signal(pos)
            || self.find_powered_rail_signal(pos, s, true, 0)
            || self.find_powered_rail_signal(pos, s, false, 0);
        if should == is_powered {
            return true;
        }
        {
            let ns = self.d.with_bool(s, prop::POWERED, should);
            self.set_block_and_update(pos, ns);
            let b = self.d.block_of(s);
            self.update_neighbors_at(pos.below(), b);
            if self.rail_shape(s).is_slope() {
                self.update_neighbors_at(pos.above(), b);
            }
        }
        false
    }

    // ----- detector rail (no minecarts: it can only switch off) ----------------------------

    fn detector_check_pressed(&mut self, pos: BlockPos, s: State) {
        if !(self.rail_can_survive(pos) && self.d.block_of(self.get(pos)) == blk::DETECTOR_RAIL) {
            return;
        }
        let was = self.d.get_bool(s, prop::POWERED);
        if was {
            let ns = self.d.with_bool(s, prop::POWERED, false);
            self.set_block_and_update(pos, ns);
            // updatePowerToConnected
            let rs = self.rs_new(pos, ns);
            for c in rs.conns.clone() {
                let cs = self.get(c);
                let cb = self.d.block_of(cs);
                self.neighbor_changed_full(cs, c, cb, false);
            }
            self.update_neighbors_at(pos, blk::DETECTOR_RAIL);
            self.update_neighbors_at(pos.below(), blk::DETECTOR_RAIL);
        }
        self.update_neighbour_for_output_signal(pos, blk::DETECTOR_RAIL);
    }

    pub(crate) fn detector_tick(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) {
            self.detector_check_pressed(pos, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_shapes_follow_property_order() {
        let d = crate::data::data();
        for p in [prop::RAIL_SHAPE, prop::RAIL_SHAPE_STRAIGHT] {
            for (i, v) in d.props[p as usize].values.iter().enumerate() {
                assert_eq!(SHAPES[i], Shape::parse(v));
                assert_eq!(SHAPES[i].name(), v);
            }
        }
    }
}
