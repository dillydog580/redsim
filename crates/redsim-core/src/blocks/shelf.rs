//! Shelves (`ShelfBlock` + `SideChainPartBlock`), copper golem statues (comparator output from the
//! pose) and big dripleaf (redstone resets the tilt) with its stem.

use crate::data::{State, AIR};
use crate::gen::{blk, prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::vibration::GameEvent;
use crate::world::World;
use rustc_hash::FxHashMap;

const MAX_CHAIN: usize = 3;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Part {
    Unconnected,
    Right,
    Center,
    Left,
}

impl Part {
    fn parse(s: &str) -> Part {
        match s {
            "right" => Part::Right,
            "center" => Part::Center,
            "left" => Part::Left,
            _ => Part::Unconnected,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Part::Unconnected => "unconnected",
            Part::Right => "right",
            Part::Center => "center",
            Part::Left => "left",
        }
    }
    fn is_connected(self) -> bool {
        self != Part::Unconnected
    }
    fn is_connection_towards(self, end: Part) -> bool {
        self == Part::Center || self == end
    }
    fn is_chain_end(self) -> bool {
        self != Part::Center
    }
    fn connected_right(self) -> Part {
        match self {
            Part::Unconnected | Part::Left => Part::Left,
            Part::Right | Part::Center => Part::Center,
        }
    }
    fn connected_left(self) -> Part {
        match self {
            Part::Unconnected | Part::Right => Part::Right,
            Part::Center | Part::Left => Part::Center,
        }
    }
    fn disconnected_right(self) -> Part {
        match self {
            Part::Unconnected | Part::Left => Part::Unconnected,
            Part::Right | Part::Center => Part::Right,
        }
    }
    fn disconnected_left(self) -> Part {
        match self {
            Part::Unconnected | Part::Right => Part::Unconnected,
            Part::Center | Part::Left => Part::Left,
        }
    }
}

/// `SideChainPartBlock.Neighbors`: neighbours along a shelf's sides, cached at first look.
struct Neighbors {
    facing: Dir,
    center: BlockPos,
    cache: FxHashMap<BlockPos, Option<Part>>,
}

impl World {
    fn shelf_part(&self, s: State) -> Part {
        Part::parse(self.d.get_str(s, prop::SIDE_CHAIN_PART))
    }

    fn shelf_connectable(&self, s: State) -> bool {
        self.d.has_tag(self.d.block_of(s), tag::WOODEN_SHELVES) && self.d.has_prop(s, prop::POWERED) && self.d.get_bool(s, prop::POWERED)
    }

    fn shelf_facing(&self, s: State) -> Dir {
        self.d.get_dir(s, prop::HORIZONTAL_FACING)
    }

    /// (pos, part) of the neighbour `steps` to the left (clockwise) or right of the centre.
    fn chain_neighbor(&self, n: &mut Neighbors, left: bool, steps: i32) -> (BlockPos, Option<Part>) {
        let dir = if left { n.facing.cw() } else { n.facing.ccw() };
        let p = n.center.rel_n(dir, steps);
        if let Some(&part) = n.cache.get(&p) {
            return (p, part);
        }
        let s = self.get(p);
        let part = if self.shelf_connectable(s) && self.shelf_facing(s) == n.facing { Some(self.shelf_part(s)) } else { None };
        n.cache.insert(p, part);
        (p, part)
    }

    fn shelf_set_part(&mut self, pos: BlockPos, part: Part) {
        let s = self.get(pos);
        if self.shelf_part(s) != part {
            let ns = self.d.with_str(s, prop::SIDE_CHAIN_PART, part.name());
            self.set_block_and_update(pos, ns);
        }
    }

    /// `SideChainPartBlock.getAllBlocksConnectedTo`
    fn shelf_connected_to(&self, pos: BlockPos) -> Vec<BlockPos> {
        let s = self.get(pos);
        if !self.shelf_connectable(s) {
            return Vec::new();
        }
        let mut n = Neighbors { facing: self.shelf_facing(s), center: pos, cache: FxHashMap::default() };
        let mut out = std::collections::VecDeque::new();
        out.push_back(pos);
        for (left, end) in [(true, Part::Left), (false, Part::Right)] {
            for steps in 1..MAX_CHAIN as i32 {
                let (p, part) = self.chain_neighbor(&mut n, left, steps);
                if part.map_or(false, |pt| pt.is_connection_towards(end)) {
                    if left {
                        out.push_front(p);
                    } else {
                        out.push_back(p);
                    }
                }
                if part.map_or(true, |pt| pt.is_chain_end()) {
                    break;
                }
            }
        }
        out.into_iter().collect()
    }

    /// `SideChainPartBlock.updateNeighborsAfterPoweringDown`
    fn shelf_power_down_neighbors(&mut self, pos: BlockPos, s: State) {
        let mut n = Neighbors { facing: self.shelf_facing(s), center: pos, cache: FxHashMap::default() };
        let (lp, l) = self.chain_neighbor(&mut n, true, 1);
        if let Some(part) = l {
            self.shelf_set_part(lp, part.disconnected_right());
        }
        let (rp, r) = self.chain_neighbor(&mut n, false, 1);
        if let Some(part) = r {
            self.shelf_set_part(rp, part.disconnected_left());
        }
    }

    /// `SideChainPartBlock.updateSelfAndNeighborsOnPoweringUp`
    fn shelf_power_up(&mut self, pos: BlockPos, s: State, old: State) {
        if !self.shelf_connectable(s) {
            return;
        }
        let being_updated = self.shelf_part(s).is_connected() || (self.shelf_connectable(old) && self.shelf_part(old).is_connected());
        if being_updated {
            return;
        }
        let mut n = Neighbors { facing: self.shelf_facing(s), center: pos, cache: FxHashMap::default() };
        let (lp, l) = self.chain_neighbor(&mut n, true, 1);
        let (rp, r) = self.chain_neighbor(&mut n, false, 1);
        let left_len = if l.is_some() { self.shelf_connected_to(lp).len() } else { 0 };
        let right_len = if r.is_some() { self.shelf_connected_to(rp).len() } else { 0 };
        let can = |n: usize, cur: usize| n > 0 && cur + n <= MAX_CHAIN;
        let mut part = Part::Unconnected;
        let mut cur = 1;
        if can(left_len, cur) {
            part = part.connected_left();
            if let Some(lpart) = l {
                self.shelf_set_part(lp, lpart.connected_right());
            }
            cur += left_len;
        }
        if can(right_len, cur) {
            part = part.connected_right();
            if let Some(rpart) = r {
                self.shelf_set_part(rp, rpart.connected_left());
            }
        }
        self.shelf_set_part(pos, part);
    }

    pub(crate) fn shelf_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if self.d.get_bool(s, prop::POWERED) {
            self.shelf_power_up(pos, s, old);
        } else {
            self.shelf_power_down_neighbors(pos, s);
        }
    }

    pub(crate) fn shelf_neighbor_changed(&mut self, s: State, pos: BlockPos) {
        let signal = self.has_neighbor_signal(pos);
        if self.d.get_bool(s, prop::POWERED) != signal {
            let mut ns = self.d.with_bool(s, prop::POWERED, signal);
            if !signal {
                ns = self.d.with_str(ns, prop::SIDE_CHAIN_PART, Part::Unconnected.name());
            }
            self.set_block_and_update(pos, ns);
            let ev = if signal { GameEvent::BlockActivate } else { GameEvent::BlockDeactivate };
            self.game_event(ev, pos, Some(ns));
        }
    }

    pub(crate) fn shelf_affect_after_removal(&mut self, s: State, pos: BlockPos) {
        // Containers.updateNeighboursAfterDestroy, then the chain lets go
        let b = self.d.block_of(s);
        self.update_neighbour_for_output_signal(pos, b);
        self.shelf_power_down_neighbors(pos, s);
    }

    /// Comparator reading a shelf: a bit per filled slot, only from the front.
    pub(crate) fn shelf_analog(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if dir != self.shelf_facing(s).opposite() {
            return 0;
        }
        match self.bes().get(&pos) {
            Some(crate::world::BlockEntity { data: crate::world::BeData::Container(c), .. }) => {
                let filled = |slot: u32| c.items.iter().any(|i| i.slot == slot && i.count > 0) as i32;
                filled(0) | filled(1) << 1 | filled(2) << 2
            }
            _ => 0,
        }
    }

    // ----- copper golem statue ----------------------------------------------------------

    pub(crate) fn statue_analog(&self, s: State) -> i32 {
        // CopperGolemStatueBlock: POSE.ordinal() + 1 (property values are in enum order)
        self.d.get_idx(s, prop::COPPER_GOLEM_POSE).unwrap_or(0) as i32 + 1
    }

    // ----- big dripleaf -------------------------------------------------------------------

    pub(crate) fn dripleaf_can_survive(&self, pos: BlockPos) -> bool {
        let b = self.d.block_of(self.get(pos.below()));
        b == blk::BIG_DRIPLEAF || b == blk::BIG_DRIPLEAF_STEM || self.d.has_tag(b, tag::SUPPORTS_BIG_DRIPLEAF)
    }

    pub(crate) fn dripleaf_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, ns: State) -> State {
        if dir == Dir::Down && !self.dripleaf_can_survive(pos) {
            return AIR;
        }
        self.warn_waterlogged_pub(s, pos);
        if dir == Dir::Up && self.d.block_of(ns) == blk::BIG_DRIPLEAF {
            // BIG_DRIPLEAF_STEM.withPropertiesOf(state)
            let stem = self.d.default_state(blk::BIG_DRIPLEAF_STEM);
            let stem = self.d.with_dir(stem, prop::HORIZONTAL_FACING, self.d.get_dir(s, prop::HORIZONTAL_FACING));
            return self.d.with_bool(stem, prop::WATERLOGGED, self.d.get_bool(s, prop::WATERLOGGED));
        }
        s
    }

    fn dripleaf_set_tilt(&mut self, s: State, pos: BlockPos, tilt: &str) {
        let prev = self.d.get_str(s, prop::TILT).to_string();
        let ns = self.d.with_str(s, prop::TILT, tilt);
        self.set_block(pos, ns, 2);
        // Tilt.causesVibration: every tilt but "unstable"
        if tilt != "unstable" && tilt != prev {
            self.game_event(GameEvent::BlockChange, pos, None);
        }
    }

    fn dripleaf_tilt_and_schedule(&mut self, s: State, pos: BlockPos, tilt: &str) {
        self.dripleaf_set_tilt(s, pos, tilt);
        let delay = match tilt {
            "unstable" | "partial" => 10,
            "full" => 100,
            _ => return,
        };
        self.schedule_tick(pos, blk::BIG_DRIPLEAF, delay, 0);
    }

    pub(crate) fn dripleaf_neighbor_changed(&mut self, s: State, pos: BlockPos) {
        if self.has_neighbor_signal(pos) {
            self.dripleaf_set_tilt(s, pos, "none");
        }
    }

    pub(crate) fn dripleaf_tick(&mut self, s: State, pos: BlockPos) {
        if self.has_neighbor_signal(pos) {
            self.dripleaf_set_tilt(s, pos, "none");
            return;
        }
        match self.d.get_str(s, prop::TILT) {
            "unstable" => self.dripleaf_tilt_and_schedule(s, pos, "partial"),
            "partial" => self.dripleaf_tilt_and_schedule(s, pos, "full"),
            "full" => self.dripleaf_set_tilt(s, pos, "none"),
            _ => {}
        }
    }

    pub(crate) fn dripleaf_stem_can_survive(&self, pos: BlockPos) -> bool {
        let below = self.d.block_of(self.get(pos.below()));
        let above = self.d.block_of(self.get(pos.above()));
        (below == blk::BIG_DRIPLEAF_STEM || self.d.has_tag(below, tag::SUPPORTS_BIG_DRIPLEAF))
            && (above == blk::BIG_DRIPLEAF_STEM || above == blk::BIG_DRIPLEAF)
    }

    pub(crate) fn dripleaf_stem_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir) -> State {
        if (dir == Dir::Down || dir == Dir::Up) && !self.dripleaf_stem_can_survive(pos) {
            self.schedule_tick(pos, blk::BIG_DRIPLEAF_STEM, 1, 0);
        }
        self.warn_waterlogged_pub(s, pos);
        s
    }

    pub(crate) fn dripleaf_stem_tick(&mut self, pos: BlockPos) {
        if !self.dripleaf_stem_can_survive(pos) {
            self.destroy_block(pos, true, 512);
        }
    }
}
