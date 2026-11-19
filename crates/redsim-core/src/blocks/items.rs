//! Item transfer between block containers: `HopperBlockEntity` (push/pull with cooldowns),
//! dropper insertion, and the comparator notifications (`setChanged`) each step causes.
//! Item entities are not simulated: anything that would create one produces a warning.

use crate::data::{Kind, State};
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::world::{BeData, BlockEntity, ContainerData, ItemStack, World};

/// A resolved container: one block entity, or a double chest (first, second).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum CRef {
    Single(BlockPos),
    Double(BlockPos, BlockPos),
}

impl World {
    fn cdata(&self, p: BlockPos) -> Option<&ContainerData> {
        match self.bes().get(&p) {
            Some(BlockEntity { data: BeData::Container(c), .. }) => Some(c),
            _ => None,
        }
    }
    fn cdata_mut(&mut self, p: BlockPos) -> Option<&mut ContainerData> {
        match self.bes_mut().get_mut(&p) {
            Some(BlockEntity { data: BeData::Container(c), .. }) => Some(c),
            _ => None,
        }
    }
    fn is_hopper_be(&self, p: BlockPos) -> bool {
        self.cdata(p).map_or(false, |c| c.hopper.is_some())
    }

    pub(crate) fn item_max(&self, id: &str) -> i32 {
        *self.d.item_max_stack.get(id).unwrap_or(&64)
    }

    /// `HopperBlockEntity.getContainerAt` for blocks (entity containers are not simulated).
    pub(crate) fn container_at(&mut self, p: BlockPos) -> Option<CRef> {
        let s = self.get(p);
        if matches!(self.kind(s), Kind::Composter | Kind::Jukebox | Kind::ChiseledBookshelf | Kind::Crafter) {
            self.warn(p, "hopper/dropper interacts with a composter, jukebox, bookshelf or crafter: not simulated");
            return None;
        }
        self.cdata(p)?;
        if self.d.bflags(self.d.block_of(s)) & crate::data::bflag::CHEST != 0 {
            // ChestBlock.getContainer(.., ignoreBeingBlocked = true)
            let ty = self.d.get_str(s, prop::CHEST_TYPE);
            if ty == "single" {
                return Some(CRef::Single(p));
            }
            let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
            let np = p.rel(if ty == "left" { f.cw() } else { f.ccw() });
            let ns = self.get(np);
            if self.d.block_of(ns) == self.d.block_of(s) {
                let nt = self.d.get_str(ns, prop::CHEST_TYPE);
                if nt != "single" && nt != ty && self.d.get_dir(ns, prop::HORIZONTAL_FACING) == f && self.cdata(np).is_some() {
                    return Some(if ty == "right" { CRef::Double(p, np) } else { CRef::Double(np, p) });
                }
            }
            return Some(CRef::Single(p));
        }
        Some(CRef::Single(p))
    }

    fn c_size(&self, c: CRef) -> u32 {
        match c {
            CRef::Single(p) => self.cdata(p).map_or(0, |d| d.size),
            CRef::Double(a, b) => self.cdata(a).map_or(0, |d| d.size) + self.cdata(b).map_or(0, |d| d.size),
        }
    }

    fn c_locate(&self, c: CRef, slot: u32) -> (BlockPos, u32) {
        match c {
            CRef::Single(p) => (p, slot),
            CRef::Double(a, b) => {
                let n = self.cdata(a).map_or(0, |d| d.size);
                if slot < n {
                    (a, slot)
                } else {
                    (b, slot - n)
                }
            }
        }
    }

    fn c_get(&self, c: CRef, slot: u32) -> Option<ItemStack> {
        self.c_peek(c, slot).cloned()
    }

    /// The stack `c_get` returns, without copying it.
    fn c_peek(&self, c: CRef, slot: u32) -> Option<&ItemStack> {
        let (p, s) = self.c_locate(c, slot);
        self.cdata(p).and_then(|d| d.items.iter().find(|i| i.slot == s && i.count > 0))
    }

    /// True if every slot of `c` holds a stack and `full(stack)` holds for each (`c_get` order).
    fn c_all_slots(&self, c: CRef, full: impl Fn(&ItemStack) -> bool) -> bool {
        match c {
            CRef::Single(p) => match self.cdata(p) {
                Some(d) => (0..d.size).all(|i| d.items.iter().find(|it| it.slot == i && it.count > 0).map_or(false, |st| full(st))),
                None => true,
            },
            CRef::Double(..) => (0..self.c_size(c)).all(|i| self.c_peek(c, i).map_or(false, |st| full(st))),
        }
    }

    fn raw_set(&mut self, p: BlockPos, slot: u32, stack: Option<ItemStack>) {
        if let Some(d) = self.cdata_mut(p) {
            d.items.retain(|i| i.slot != slot);
            if let Some(mut st) = stack {
                if st.count > 0 {
                    st.slot = slot;
                    d.items.push(st);
                    d.items.sort_by_key(|i| i.slot);
                }
            }
        }
    }

    /// `BlockEntity.setChanged`: comparators next to the container re-check.
    pub(crate) fn be_set_changed(&mut self, p: BlockPos) {
        let s = self.get(p);
        let b = match self.bes().get(&p) {
            Some(be) => self.d.block_of(be.state),
            None => return,
        };
        if !self.is_air(s) {
            self.update_neighbour_for_output_signal(p, b);
        }
        // ShelfBlockEntity.setChanged() -> setChanged(BLOCK_ACTIVATE)
        if let Some(be) = self.bes().get(&p) {
            let st = be.state;
            if self.kind(st) == crate::data::Kind::Shelf {
                self.game_event(crate::vibration::GameEvent::BlockActivate, p, Some(st));
            }
        }
    }

    fn c_set_changed(&mut self, c: CRef) {
        match c {
            CRef::Single(p) => self.be_set_changed(p),
            CRef::Double(a, b) => {
                self.be_set_changed(a);
                self.be_set_changed(b);
            }
        }
    }

    /// `Container.setItem` (non-hopper containers call setChanged).
    fn c_set_item(&mut self, c: CRef, slot: u32, stack: Option<ItemStack>) {
        let (p, s) = self.c_locate(c, slot);
        let limited = stack.map(|mut st| {
            let m = self.c_max_for(p, &st.id);
            st.count = st.count.min(m);
            st
        });
        self.raw_set(p, s, limited);
        if !self.is_hopper_be(p) && !self.is_single_item_be(p) {
            self.be_set_changed(p);
        }
    }

    /// `Container.removeItem(slot, n)` -> removed stack.
    fn c_remove_item(&mut self, c: CRef, slot: u32, n: i32) -> Option<ItemStack> {
        let (p, s) = self.c_locate(c, slot);
        let mut cur = self.cdata(p).and_then(|d| d.items.iter().find(|i| i.slot == s && i.count > 0).cloned())?;
        if n <= 0 {
            return None;
        }
        let take = n.min(cur.count);
        cur.count -= take;
        let removed = ItemStack { slot: 0, id: cur.id.clone(), count: take };
        let id = cur.id.clone();
        self.raw_set(p, s, if cur.count > 0 { Some(cur) } else { None });
        if !self.is_hopper_be(p) && !self.is_single_item_be(p) {
            self.be_set_changed(p);
        }
        let _ = id;
        Some(removed)
    }

    /// `ContainerSingleItem` (decorated pots): setItem/removeItem don't call setChanged.
    fn is_single_item_be(&self, p: BlockPos) -> bool {
        self.d.block_of(self.get(p)) == crate::gen::blk::DECORATED_POT
    }

    fn c_max_for(&self, p: BlockPos, id: &str) -> i32 {
        let cm = self.cdata(p).map_or(99, |d| d.max_stack);
        cm.min(self.item_max(id))
    }

    fn c_is_empty(&self, c: CRef) -> bool {
        match c {
            // no slot below the size holds a stack
            CRef::Single(p) => self.cdata(p).map_or(true, |d| !d.items.iter().any(|it| it.count > 0 && it.slot < d.size)),
            CRef::Double(..) => (0..self.c_size(c)).all(|i| self.c_peek(c, i).is_none()),
        }
    }

    fn c_is_shulker(&self, c: CRef) -> bool {
        match c {
            CRef::Single(p) => self.d.has_tag(self.d.block_of(self.get(p)), crate::gen::tag::SHULKER_BOXES),
            CRef::Double(..) => false,
        }
    }

    /// `canPlaceItem` + WorldlyContainer face check (shulker boxes refuse shulker boxes).
    fn c_can_place(&self, c: CRef, id: &str) -> bool {
        !(self.c_is_shulker(c) && id.ends_with("shulker_box"))
    }

    /// `HopperBlockEntity.isFullContainer`
    fn c_is_full(&self, c: CRef) -> bool {
        self.c_all_slots(c, |st| st.count >= self.item_max(&st.id))
    }

    /// `HopperBlockEntity.addItem(from, container, stack, direction)` -> remaining (None = all moved).
    fn c_add_item(&mut self, from: Option<BlockPos>, c: CRef, mut stack: ItemStack) -> Option<ItemStack> {
        let n = self.c_size(c);
        for i in 0..n {
            if stack.count <= 0 {
                break;
            }
            stack = match self.c_try_move_in(from, c, stack, i) {
                Some(s) => s,
                None => return None,
            };
        }
        if stack.count > 0 {
            Some(stack)
        } else {
            None
        }
    }

    /// `HopperBlockEntity.tryMoveInItem`
    fn c_try_move_in(&mut self, from: Option<BlockPos>, c: CRef, mut stack: ItemStack, slot: u32) -> Option<ItemStack> {
        if !self.c_can_place(c, &stack.id) {
            return Some(stack);
        }
        let cur = self.c_get(c, slot);
        let was_empty = self.c_is_empty(c);
        let mut success = false;
        match cur {
            None => {
                self.c_set_item(c, slot, Some(stack.clone()));
                stack.count = 0;
                success = true;
            }
            Some(mut cs) => {
                let max = self.item_max(&cs.id);
                if cs.count <= max && cs.id == stack.id {
                    let space = self.item_max(&stack.id) - cs.count;
                    let count = stack.count.min(space);
                    stack.count -= count;
                    cs.count += count;
                    let (p, s) = self.c_locate(c, slot);
                    self.raw_set(p, s, Some(cs));
                    success = count > 0;
                }
            }
        }
        if success {
            if let CRef::Single(hp) = c {
                if was_empty {
                    let gt_from = from.and_then(|f| self.cdata(f).and_then(|d| d.hopper.map(|h| h.ticked)));
                    if let Some(d) = self.cdata_mut(hp) {
                        if let Some(h) = &mut d.hopper {
                            if h.cooldown <= 8 {
                                let skip = match gt_from {
                                    Some(ft) if h.ticked >= ft => 1,
                                    _ => 0,
                                };
                                h.cooldown = 8 - skip;
                            }
                        }
                    }
                }
            }
            self.c_set_changed(c);
        }
        if stack.count > 0 {
            Some(stack)
        } else {
            None
        }
    }

    // ----- hopper ---------------------------------------------------------------------

    /// `HopperBlockEntity.pushItemsTick`
    pub(crate) fn hopper_tick(&mut self, pos: BlockPos, state: State) {
        let gt = self.game_time;
        let on_cooldown = {
            let Some(d) = self.cdata_mut(pos) else { return };
            let Some(h) = &mut d.hopper else { return };
            h.cooldown -= 1;
            h.ticked = gt;
            h.cooldown > 0
        };
        if !on_cooldown {
            if let Some(h) = self.cdata_mut(pos).and_then(|d| d.hopper.as_mut()) {
                h.cooldown = 0;
            }
            self.hopper_try_move_items(pos, state);
        }
    }

    fn hopper_try_move_items(&mut self, pos: BlockPos, state: State) -> bool {
        // cooldown and emptiness from one lookup (nothing changes between the two in the game)
        let (cooldown, empty) = match self.cdata(pos) {
            Some(d) => (d.hopper.map_or(0, |h| h.cooldown), !d.items.iter().any(|it| it.count > 0 && it.slot < d.size)),
            None => (0, true),
        };
        if cooldown > 0 || !self.d.get_bool(state, prop::ENABLED) {
            return false;
        }
        let me = CRef::Single(pos);
        let mut changed = false;
        if !empty {
            changed = self.hopper_eject(pos, state);
        }
        let full = self.c_all_slots(me, |st| st.count == self.item_max(&st.id));
        if !full {
            changed |= self.hopper_suck(pos);
        }
        if changed {
            if let Some(h) = self.cdata_mut(pos).and_then(|d| d.hopper.as_mut()) {
                h.cooldown = 8;
            }
            let b = self.d.block_of(state);
            self.update_neighbour_for_output_signal(pos, b);
            return true;
        }
        false
    }

    fn hopper_eject(&mut self, pos: BlockPos, state: State) -> bool {
        let facing = self.d.get_dir(state, prop::FACING_HOPPER);
        let Some(target) = self.container_at(pos.rel(facing)) else { return false };
        if self.c_is_full(target) {
            return false;
        }
        let me = CRef::Single(pos);
        for slot in 0..self.c_size(me) {
            if let Some(st) = self.c_get(me, slot) {
                let original = st.count;
                let one = self.c_remove_item(me, slot, 1).unwrap();
                match self.c_add_item(Some(pos), target, one) {
                    None => {
                        self.c_set_changed(target);
                        return true;
                    }
                    Some(_) => {
                        let mut restored = st.clone();
                        restored.count = original;
                        self.raw_set(pos, slot, Some(restored));
                    }
                }
            }
        }
        false
    }

    fn hopper_suck(&mut self, pos: BlockPos) -> bool {
        let above = pos.above();
        if let Some(src) = self.container_at(above) {
            for slot in 0..self.c_size(src) {
                if let Some(st) = self.c_get(src, slot) {
                    // canTakeItemFromContainer: all simulated containers allow it
                    let original = st.count;
                    let one = self.c_remove_item(src, slot, 1).unwrap();
                    match self.c_add_item(Some(src_pos(src)), CRef::Single(pos), one) {
                        None => {
                            self.c_set_changed(src);
                            return true;
                        }
                        Some(_) => {
                            let mut restored = st.clone();
                            restored.count = original;
                            let (p, s) = self.c_locate(src, slot);
                            if original == 1 {
                                self.c_set_item(src, slot, Some(restored));
                            } else {
                                self.raw_set(p, s, Some(restored));
                            }
                        }
                    }
                }
            }
            false
        } else {
            // item entities above are not simulated
            false
        }
    }

    // ----- dropper / dispenser ------------------------------------------------------------

    pub(crate) fn dropper_dispense(&mut self, s: State, pos: BlockPos) {
        let me = CRef::Single(pos);
        let filled: Vec<u32> = (0..self.c_size(me)).filter(|&i| self.c_peek(me, i).is_some()).collect();
        if filled.is_empty() {
            // levelEvent (sound); a dispenser (not a dropper) also emits BLOCK_ACTIVATE
            if self.d.block_of(s) != blk::DROPPER {
                let st = self.bes().get(&pos).map_or(s, |be| be.state);
                self.game_event(crate::vibration::GameEvent::BlockActivate, pos, Some(st));
            }
            return;
        }
        if filled.len() > 1 {
            self.warn(pos, "dropper/dispenser with several filled slots picks one at random in the game; redsim uses the first");
        }
        let slot = filled[0];
        if self.d.block_of(s) != blk::DROPPER {
            self.warn(pos, "dispenser item behaviours (projectiles, placing, equipping...) are not simulated");
            return;
        }
        let st = self.c_get(me, slot).unwrap();
        let facing = self.d.get_dir(self.get(pos), prop::FACING);
        let remaining = match self.container_at(pos.rel(facing)) {
            None => {
                self.warn(pos, "dropper drops an item entity: item entities are not simulated");
                let mut r = st.clone();
                r.count -= 1;
                r
            }
            Some(into) => {
                let one = ItemStack { slot: 0, id: st.id.clone(), count: 1 };
                match self.c_add_item(Some(pos), into, one) {
                    None => {
                        let mut r = st.clone();
                        r.count -= 1;
                        r
                    }
                    Some(_) => st.clone(),
                }
            }
        };
        self.c_set_item(me, slot, if remaining.count > 0 { Some(remaining) } else { None });
    }
}

fn src_pos(c: CRef) -> BlockPos {
    match c {
        CRef::Single(p) => p,
        CRef::Double(a, _) => a,
    }
}

#[allow(dead_code)]
fn _unused(_d: Dir) {}
