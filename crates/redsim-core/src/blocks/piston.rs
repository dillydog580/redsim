//! Pistons: `PistonBaseBlock`, `PistonHeadBlock`, `MovingPistonBlock`,
//! `PistonStructureResolver` and `PistonMovingBlockEntity` (block 36).

use crate::data::{Push, State, AIR};
use crate::gen::{blk, prop, st};
use crate::javahash::JavaHashSet;
use crate::pos::{BlockPos, Dir};
use crate::trace::TraceKind;
use crate::world::{BeData, MovingPiston, World, MAX_Y, MIN_Y};

pub const MAX_PUSH: usize = 12;

/// Why a piston could not move (for explanations).
#[derive(Clone, Debug, PartialEq)]
pub enum ResolveFail {
    Immovable(BlockPos),
    TooManyBlocks,
}

struct Resolver {
    piston_pos: BlockPos,
    extending: bool,
    start_pos: BlockPos,
    push_dir: Dir,
    piston_dir: Dir,
    to_push: Vec<BlockPos>,
    to_destroy: Vec<BlockPos>,
    fail: Option<ResolveFail>,
}

#[inline]
fn is_sticky(w: &World, s: State) -> bool {
    w.is(s, blk::SLIME_BLOCK) || w.is(s, blk::HONEY_BLOCK)
}

fn can_stick_to_each_other(w: &World, a: State, b: State) -> bool {
    if w.is(a, blk::HONEY_BLOCK) && w.is(b, blk::SLIME_BLOCK) {
        false
    } else if w.is(a, blk::SLIME_BLOCK) && w.is(b, blk::HONEY_BLOCK) {
        false
    } else {
        is_sticky(w, a) || is_sticky(w, b)
    }
}

impl World {
    #[inline]
    fn piston_facing(&self, s: State) -> Dir {
        self.d.get_dir(s, prop::FACING)
    }
    #[inline]
    fn is_piston_base(&self, s: State) -> bool {
        self.is(s, blk::PISTON) || self.is(s, blk::STICKY_PISTON)
    }
    #[inline]
    fn piston_sticky(&self, s: State) -> bool {
        self.is(s, blk::STICKY_PISTON)
    }

    /// `PistonBaseBlock.isPushable`
    pub fn piston_is_pushable(&self, s: State, pos: BlockPos, dir: Dir, allow_destroyable: bool, connection_dir: Dir) -> bool {
        if pos.y < MIN_Y || pos.y > MAX_Y {
            return false;
        }
        if self.is_air(s) {
            return true;
        }
        if dir == Dir::Down && pos.y == MIN_Y {
            return false;
        }
        if dir == Dir::Up && pos.y == MAX_Y {
            return false;
        }
        if !self.is_piston_base(s) {
            if self.d.info(s).destroy_speed == -1.0 {
                return false;
            }
            match self.d.info(s).push {
                Push::Immoveable => return false,
                Push::Popped => return allow_destroyable,
                Push::PushOnly => return dir == connection_dir,
                _ => {}
            }
        } else if self.d.get_bool(s, prop::EXTENDED) {
            return false;
        }
        !self.has_be_flag(s)
    }

    /// `PistonBaseBlock.getNeighborSignal` (includes quasi-connectivity).
    fn piston_neighbor_signal(&self, pos: BlockPos, push_dir: Dir) -> bool {
        for d in Dir::ALL {
            if d != push_dir && self.has_signal(pos.rel(d), d) {
                return true;
            }
        }
        if self.has_signal(pos, Dir::Down) {
            return true;
        }
        let above = pos.above();
        for d in Dir::ALL {
            if d != Dir::Down && self.has_signal(above.rel(d), d) {
                return true;
            }
        }
        false
    }

    pub(crate) fn piston_set_placed_by(&mut self, pos: BlockPos, s: State) {
        self.piston_check_if_extend(pos, s);
    }

    /// Why is this piston (not) moving? Human-readable diagnosis for explanations.
    pub fn piston_diagnose(&self, pos: BlockPos) -> String {
        let s = self.get(pos);
        if !self.is_piston_base(s) {
            return "not a piston".into();
        }
        let dir = self.piston_facing(s);
        let mut lines = Vec::new();
        let mut powered = false;
        for d in Dir::ALL {
            if d != dir && self.has_signal(pos.rel(d), d) {
                lines.push(format!("powered from {} ({})", pos.rel(d), d.name()));
                powered = true;
            }
        }
        if self.has_signal(pos, Dir::Down) {
            lines.push("powered (self signal)".into());
            powered = true;
        }
        let above = pos.above();
        for d in Dir::ALL {
            if d != Dir::Down && self.has_signal(above.rel(d), d) {
                lines.push(format!("quasi-connected: block above the piston is powered via {} ({})", above.rel(d), d.name()));
                powered = true;
            }
        }
        if !powered {
            lines.push("not powered (checked 5 sides + quasi-connectivity above)".into());
        }
        let extended = self.d.get_bool(s, prop::EXTENDED);
        lines.push(format!("extended={}", extended));
        if powered && !extended {
            let mut r = Resolver::new(pos, dir, true);
            if self.resolve(&mut r) {
                lines.push(format!("can extend: would push {} block(s), destroy {}", r.to_push.len(), r.to_destroy.len()));
            } else {
                lines.push(match r.fail {
                    Some(ResolveFail::Immovable(p)) => format!("CANNOT extend: immovable block {} at {}", self.d.format_state(self.get(p)), p),
                    Some(ResolveFail::TooManyBlocks) => "CANNOT extend: more than 12 blocks to push".into(),
                    None => "CANNOT extend".into(),
                });
            }
        }
        lines.join("\n")
    }

    pub(crate) fn piston_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if self.d.block_of(old) != self.d.block_of(s) && !self.bes().contains_key(&pos) {
            self.piston_check_if_extend(pos, s);
        }
    }

    /// `PistonBaseBlock.checkIfExtend`; returns true if the piston was already in the state its
    /// power asks for (nothing done).
    pub(crate) fn piston_check_if_extend(&mut self, pos: BlockPos, s: State) -> bool {
        let dir = self.piston_facing(s);
        let extend = self.piston_neighbor_signal(pos, dir);
        let block = self.d.block_of(s);
        if extend && !self.d.get_bool(s, prop::EXTENDED) {
            let mut r = Resolver::new(pos, dir, true);
            if self.resolve(&mut r) {
                self.block_event(pos, block, 0, dir.ordinal() as i32);
            } else if self.trace.is_some() {
                let c = self.cause;
                self.trace_event(
                    TraceKind::Note { text: format!("piston powered but cannot extend: {:?}", r.fail) },
                    pos,
                    c,
                );
            }
        } else if !extend && self.d.get_bool(s, prop::EXTENDED) {
            let pushed = pos.rel_n(dir, 2);
            let ps = self.get(pushed);
            let mut event = 1;
            if self.is(ps, blk::MOVING_PISTON) && self.piston_facing(ps) == dir {
                if let Some(m) = self.moving_be(pushed) {
                    if m.extending && (m.progress_o < 0.5 || self.game_time == m.last_ticked || self.handling_tick) {
                        event = 2;
                    }
                }
            }
            self.block_event(pos, block, event, dir.ordinal() as i32);
        } else {
            return true;
        }
        false
    }

    /// `PistonBaseBlock.triggerEvent`
    pub(crate) fn piston_trigger_event(&mut self, s: State, pos: BlockPos, a: i32, b: i32) -> bool {
        let dir = self.piston_facing(s);
        let extended_state = self.d.with_bool(s, prop::EXTENDED, true);
        let sticky = self.piston_sticky(s);
        let extend = self.piston_neighbor_signal(pos, dir);
        if extend && (a == 1 || a == 2) {
            self.set_block(pos, extended_state, 2);
            return false;
        }
        if !extend && a == 0 {
            return false;
        }
        if a == 0 {
            if !self.piston_move_blocks(pos, dir, true, sticky) {
                return false;
            }
            self.set_block(pos, extended_state, 67);
            self.game_event(crate::vibration::GameEvent::BlockActivate, pos, Some(extended_state));
        } else if a == 1 || a == 2 {
            let front = pos.rel(dir);
            if self.moving_be(front).is_some() {
                self.moving_piston_final_tick(front);
            }
            let mut moving = self.d.with_dir(st::MOVING_PISTON, prop::FACING, dir);
            moving = self.d.with_str(moving, prop::PISTON_TYPE, if sticky { "sticky" } else { "normal" });
            self.set_block(pos, moving, 276);
            let base_default = self.d.default_state(self.d.block_of(s));
            let moved_state = self.d.with_dir(base_default, prop::FACING, Dir::from_3d(b & 7));
            self.add_block_entity(
                pos,
                moving,
                BeData::Moving(MovingPiston {
                    moved_state,
                    direction: dir,
                    extending: false,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                }),
            );
            self.update_neighbors_at(pos, blk::MOVING_PISTON);
            self.update_neighbour_shapes(moving, pos, 2, 512);
            if sticky {
                let two = pos.rel_n(dir, 2);
                let ms = self.get(two);
                let mut piston_piece = false;
                if self.is(ms, blk::MOVING_PISTON) {
                    if let Some(m) = self.moving_be(two) {
                        if m.direction == dir && m.extending {
                            self.moving_piston_final_tick(two);
                            piston_piece = true;
                        }
                    }
                }
                if !piston_piece {
                    let ms = self.get(two);
                    if a != 1
                        || self.is_air(ms)
                        || !self.piston_is_pushable(ms, two, dir.opposite(), false, dir)
                        || (self.d.info(ms).push != Push::PushPull && !self.is_piston_base(ms))
                    {
                        self.remove_block(pos.rel(dir), false);
                    } else {
                        self.piston_move_blocks(pos, dir, false, sticky);
                    }
                }
            } else {
                self.remove_block(pos.rel(dir), false);
            }
            self.game_event(crate::vibration::GameEvent::BlockDeactivate, pos, Some(moving));
        }
        true
    }

    fn resolve(&self, r: &mut Resolver) -> bool {
        r.to_push.clear();
        r.to_destroy.clear();
        let ns = self.get(r.start_pos);
        if !self.piston_is_pushable(ns, r.start_pos, r.push_dir, false, r.piston_dir) {
            if r.extending && self.d.info(ns).push == Push::Popped {
                r.to_destroy.push(r.start_pos);
                return true;
            }
            r.fail = Some(ResolveFail::Immovable(r.start_pos));
            return false;
        }
        if !self.add_block_line(r, r.start_pos, r.push_dir) {
            return false;
        }
        let mut i = 0;
        while i < r.to_push.len() {
            let p = r.to_push[i];
            if is_sticky(self, self.get(p)) && !self.add_branching_blocks(r, p) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn add_block_line(&self, r: &mut Resolver, start: BlockPos, direction: Dir) -> bool {
        let mut ns = self.get(start);
        if self.is_air(ns) {
            return true;
        }
        if !self.piston_is_pushable(ns, start, r.push_dir, false, direction) {
            return true;
        }
        if start == r.piston_pos {
            return true;
        }
        if r.to_push.contains(&start) {
            return true;
        }
        let mut block_count = 1usize;
        if block_count + r.to_push.len() > MAX_PUSH {
            r.fail = Some(ResolveFail::TooManyBlocks);
            return false;
        }
        while is_sticky(self, ns) {
            let pos = start.rel_n(r.push_dir.opposite(), block_count as i32);
            let prev = ns;
            ns = self.get(pos);
            if self.is_air(ns)
                || !can_stick_to_each_other(self, prev, ns)
                || !self.piston_is_pushable(ns, pos, r.push_dir, false, r.push_dir.opposite())
                || pos == r.piston_pos
            {
                break;
            }
            block_count += 1;
            if block_count + r.to_push.len() > MAX_PUSH {
                r.fail = Some(ResolveFail::TooManyBlocks);
                return false;
            }
        }
        let mut blocks_added = 0usize;
        for i in (0..block_count).rev() {
            r.to_push.push(start.rel_n(r.push_dir.opposite(), i as i32));
            blocks_added += 1;
        }
        let mut i = 1;
        loop {
            let pos = start.rel_n(r.push_dir, i);
            if let Some(collision) = r.to_push.iter().position(|&p| p == pos) {
                Self::reorder_list_at_collision(r, blocks_added, collision);
                for j in 0..=(collision + blocks_added) {
                    let bp = r.to_push[j];
                    if is_sticky(self, self.get(bp)) && !self.add_branching_blocks(r, bp) {
                        return false;
                    }
                }
                return true;
            }
            ns = self.get(pos);
            if self.is_air(ns) {
                return true;
            }
            if !self.piston_is_pushable(ns, pos, r.push_dir, true, r.push_dir) || pos == r.piston_pos {
                r.fail = Some(ResolveFail::Immovable(pos));
                return false;
            }
            if self.d.info(ns).push == Push::Popped {
                r.to_destroy.push(pos);
                return true;
            }
            if r.to_push.len() >= MAX_PUSH {
                r.fail = Some(ResolveFail::TooManyBlocks);
                return false;
            }
            r.to_push.push(pos);
            blocks_added += 1;
            i += 1;
        }
    }

    fn reorder_list_at_collision(r: &mut Resolver, blocks_added: usize, collision: usize) {
        let n = r.to_push.len();
        let head: Vec<BlockPos> = r.to_push[0..collision].to_vec();
        let last_line: Vec<BlockPos> = r.to_push[n - blocks_added..n].to_vec();
        let collision_to_line: Vec<BlockPos> = r.to_push[collision..n - blocks_added].to_vec();
        r.to_push.clear();
        r.to_push.extend(head);
        r.to_push.extend(last_line);
        r.to_push.extend(collision_to_line);
    }

    fn add_branching_blocks(&self, r: &mut Resolver, from: BlockPos) -> bool {
        let fs = self.get(from);
        for d in Dir::ALL {
            if d.axis() != r.push_dir.axis() {
                let np = from.rel(d);
                let ns = self.get(np);
                if can_stick_to_each_other(self, ns, fs) && !self.add_block_line(r, np, d) {
                    return false;
                }
            }
        }
        true
    }

    /// `PistonBaseBlock.moveBlocks`
    fn piston_move_blocks(&mut self, piston_pos: BlockPos, dir: Dir, extending: bool, sticky: bool) -> bool {
        let arm = piston_pos.rel(dir);
        if !extending && self.is(self.get(arm), blk::PISTON_HEAD) {
            self.set_block(arm, AIR, 276);
        }
        let mut r = Resolver::new(piston_pos, dir, extending);
        if !self.resolve(&mut r) {
            return false;
        }
        let to_push = r.to_push.clone();
        let to_destroy = r.to_destroy.clone();
        let mut delete_after_move: JavaHashSet<State> = JavaHashSet::new();
        let mut to_push_shapes: Vec<State> = Vec::with_capacity(to_push.len());
        for &p in &to_push {
            let s = self.get(p);
            to_push_shapes.push(s);
            delete_after_move.insert(p, s);
        }
        let mut to_update: Vec<State> = Vec::with_capacity(to_push.len() + to_destroy.len());
        let push_dir = if extending { dir } else { dir.opposite() };
        for i in (0..to_destroy.len()).rev() {
            let p = to_destroy[i];
            let s = self.get(p);
            // dropResources: not simulated
            self.set_block(p, AIR, 18);
            self.game_event(crate::vibration::GameEvent::BlockDestroy, p, Some(s));
            to_update.push(s);
        }
        for i in (0..to_push.len()).rev() {
            let p0 = to_push[i];
            let bs = self.get(p0);
            let p = p0.rel(push_dir);
            delete_after_move.remove(p);
            let actual = self.d.with_dir(st::MOVING_PISTON, prop::FACING, dir);
            self.set_block(p, actual, 324);
            self.add_block_entity(
                p,
                actual,
                BeData::Moving(MovingPiston {
                    moved_state: to_push_shapes[i],
                    direction: dir,
                    extending,
                    source: false,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                }),
            );
            to_update.push(bs);
        }
        if extending {
            let head_type = if sticky { "sticky" } else { "normal" };
            let mut head = self.d.with_dir(st::PISTON_HEAD, prop::FACING, dir);
            head = self.d.with_str(head, prop::PISTON_TYPE, head_type);
            let mut moving = self.d.with_dir(st::MOVING_PISTON, prop::FACING, dir);
            moving = self.d.with_str(moving, prop::PISTON_TYPE, head_type);
            delete_after_move.remove(arm);
            self.set_block(arm, moving, 324);
            self.add_block_entity(
                arm,
                moving,
                BeData::Moving(MovingPiston {
                    moved_state: head,
                    direction: dir,
                    extending: true,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                }),
            );
        }
        let entries: Vec<(BlockPos, State)> = delete_after_move.iter().copied().collect();
        for &(p, _) in &entries {
            self.set_block(p, AIR, 82);
        }
        for &(p, old) in &entries {
            self.update_indirect_neighbour_shapes(old, p, 2, 512);
            self.update_neighbour_shapes(AIR, p, 2, 512);
            self.update_indirect_neighbour_shapes(AIR, p, 2, 512);
        }
        let mut idx = 0;
        for i in (0..to_destroy.len()).rev() {
            let s = to_update[idx];
            idx += 1;
            let p = to_destroy[i];
            self.affect_neighbors_after_removal(s, p, false);
            self.update_indirect_neighbour_shapes(s, p, 2, 512);
            self.update_neighbors_at(p, self.d.block_of(s));
        }
        for i in (0..to_push.len()).rev() {
            let b = self.d.block_of(to_update[idx]);
            idx += 1;
            self.update_neighbors_at(to_push[i], b);
        }
        if extending {
            self.update_neighbors_at(arm, blk::PISTON_HEAD);
        }
        true
    }

    // ----- piston head ------------------------------------------------------------

    fn piston_head_fitting_base(&self, head: State, base: State) -> bool {
        let base_block = if self.d.get_str(head, prop::PISTON_TYPE) == "normal" { blk::PISTON } else { blk::STICKY_PISTON };
        self.is(base, base_block) && self.d.get_bool(base, prop::EXTENDED) && self.piston_facing(base) == self.piston_facing(head)
    }

    pub(crate) fn piston_head_can_survive(&self, s: State, pos: BlockPos) -> bool {
        let base = self.get(pos.rel(self.piston_facing(s).opposite()));
        self.piston_head_fitting_base(s, base)
            || (self.is(base, blk::MOVING_PISTON) && self.piston_facing(base) == self.piston_facing(s))
    }

    pub(crate) fn piston_head_neighbor_changed(&mut self, s: State, pos: BlockPos, block: crate::data::BlockId) {
        if self.can_survive(s, pos) {
            let bp = pos.rel(self.piston_facing(s).opposite());
            self.neighbor_changed(bp, block);
        }
    }

    pub(crate) fn piston_head_affect_neighbors_after_removal(&mut self, s: State, pos: BlockPos) {
        let bp = pos.rel(self.piston_facing(s).opposite());
        if self.piston_head_fitting_base(s, self.get(bp)) {
            self.destroy_block(bp, true, 512);
        }
    }

    // ----- moving piston block entity ------------------------------------------------

    /// `PistonMovingBlockEntity.finalTick`
    pub(crate) fn moving_piston_final_tick(&mut self, pos: BlockPos) {
        let Some(m) = self.moving_be(pos).cloned() else { return };
        if m.progress_o < 1.0 {
            self.remove_block_entity(pos);
            if self.is(self.get(pos), blk::MOVING_PISTON) {
                let ns = if m.source { AIR } else { self.update_from_neighbour_shapes(m.moved_state, pos) };
                self.set_block_and_update(pos, ns);
                self.neighbor_changed(pos, self.d.block_of(ns));
            }
        }
    }

    /// `PistonMovingBlockEntity.tick`
    pub(crate) fn moving_piston_tick(&mut self, pos: BlockPos, _state: State) {
        let gt = self.game_time;
        let Some(m) = self.moving_be_mut(pos) else { return };
        m.last_ticked = gt;
        m.progress_o = m.progress;
        if m.progress_o >= 1.0 {
            let moved = m.moved_state;
            self.remove_block_entity(pos);
            if self.is(self.get(pos), blk::MOVING_PISTON) {
                let mut ns = self.update_from_neighbour_shapes(moved, pos);
                if self.is_air(ns) {
                    self.set_block(pos, moved, 340);
                    self.update_or_destroy(moved, ns, pos, 3, 512);
                } else {
                    if self.d.has_prop(ns, prop::WATERLOGGED) && self.d.get_bool(ns, prop::WATERLOGGED) {
                        ns = self.d.with_bool(ns, prop::WATERLOGGED, false);
                    }
                    self.set_block(pos, ns, 67);
                    self.neighbor_changed(pos, self.d.block_of(ns));
                }
            }
        } else {
            let np = m.progress + 0.5;
            m.progress = if np >= 1.0 { 1.0 } else { np };
        }
    }
}

impl Resolver {
    fn new(piston_pos: BlockPos, dir: Dir, extending: bool) -> Self {
        let (push_dir, start_pos) = if extending { (dir, piston_pos.rel(dir)) } else { (dir.opposite(), piston_pos.rel_n(dir, 2)) };
        Resolver {
            piston_pos,
            extending,
            start_pos,
            push_dir,
            piston_dir: dir,
            to_push: Vec::new(),
            to_destroy: Vec::new(),
            fail: None,
        }
    }
}
