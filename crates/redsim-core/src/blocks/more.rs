//! Note blocks, dispensers/droppers, hoppers, copper bulbs, pressure plates, skulls,
//! and comparator-readable block states.

use crate::data::{Kind, State, AIR};
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::shape::Support;
use crate::world::{BeData, World};

impl World {
    // ----- note block -------------------------------------------------------------

    fn instrument_of(&self, s: State) -> (&str, bool) {
        let b = self.d.binfo(self.d.block_of(s));
        let name = b.fields.get("instrument").and_then(|v| v.as_str()).unwrap_or("harp");
        let above = b.fields.get("instrument_above").and_then(|v| v.as_bool()).unwrap_or(false);
        (name, above)
    }

    /// `NoteBlock.setInstrument`
    fn note_set_instrument(&self, pos: BlockPos, s: State) -> State {
        let (above_name, above_works) = self.instrument_of(self.get(pos.above()));
        if above_works {
            return self.d.with_str(s, prop::NOTEBLOCK_INSTRUMENT, above_name);
        }
        let (below_name, below_works) = self.instrument_of(self.get(pos.below()));
        let n = if below_works { "harp" } else { below_name };
        self.d.with_str(s, prop::NOTEBLOCK_INSTRUMENT, n)
    }

    pub(crate) fn note_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir) -> State {
        if dir.axis() == crate::pos::Axis::Y {
            self.note_set_instrument(pos, s)
        } else {
            s
        }
    }

    fn note_play(&mut self, s: State, pos: BlockPos) {
        // NoteBlockInstrument.worksAboveNoteBlock, precomputed per instrument at load
        let inst = self.d.get_idx(s, prop::NOTEBLOCK_INSTRUMENT).expect("note block instrument");
        let works_above = self.d.instrument_works_above[inst as usize];
        if works_above || self.is_air(self.get(pos.above())) {
            self.block_event(pos, blk::NOTE_BLOCK, 0, 0);
            self.game_event(crate::vibration::GameEvent::NoteBlockPlay, pos, None);
        }
    }

    /// Returns true if the power already matches (nothing to do).
    pub(crate) fn note_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let signal = self.has_neighbor_signal(pos);
        if signal == self.d.get_bool(s, prop::POWERED) {
            return true;
        }
        if signal {
            self.note_play(s, pos);
        }
        let ns = self.d.with_bool(s, prop::POWERED, signal);
        self.set_block_and_update(pos, ns);
        false
    }

    pub(crate) fn note_use(&mut self, s: State, pos: BlockPos) -> bool {
        let ns = self.d.cycle(s, prop::NOTE);
        self.set_block_and_update(pos, ns);
        self.note_play(ns, pos);
        true
    }

    // ----- dispenser / dropper ------------------------------------------------------

    /// Returns true if `triggered` already matches the power (nothing to do).
    pub(crate) fn dispenser_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let should = self.has_neighbor_signal(pos) || self.has_neighbor_signal(pos.above());
        let triggered = self.d.get_bool(s, prop::TRIGGERED);
        if should == triggered {
            return true;
        }
        if should && !triggered {
            let b = self.d.block_of(s);
            self.schedule_tick(pos, b, 4, 0);
            let ns = self.d.with_bool(s, prop::TRIGGERED, true);
            self.set_block(pos, ns, 2);
        } else if !should && triggered {
            let ns = self.d.with_bool(s, prop::TRIGGERED, false);
            self.set_block(pos, ns, 2);
        }
        false
    }

    pub(crate) fn dispenser_tick(&mut self, s: State, pos: BlockPos) {
        self.dropper_dispense(s, pos);
    }

    // ----- hopper -------------------------------------------------------------------

    /// Returns true if `enabled` already matches the power (nothing to do).
    pub(crate) fn hopper_check_powered(&mut self, s: State, pos: BlockPos) -> bool {
        let should_be_on = !self.has_neighbor_signal(pos);
        if should_be_on == self.d.get_bool(s, prop::ENABLED) {
            return true;
        }
        let ns = self.d.with_bool(s, prop::ENABLED, should_be_on);
        self.set_block(pos, ns, 2);
        false
    }

    // ----- copper bulb ----------------------------------------------------------------

    /// Returns true if the power already matches (nothing to do).
    pub(crate) fn copper_bulb_check_and_flip(&mut self, s: State, pos: BlockPos) -> bool {
        let signal = self.has_neighbor_signal(pos);
        if signal == self.d.get_bool(s, prop::POWERED) {
            return true;
        }
        {
            let mut ns = s;
            if !self.d.get_bool(s, prop::POWERED) {
                ns = self.d.cycle(ns, prop::LIT);
            }
            ns = self.d.with_bool(ns, prop::POWERED, signal);
            self.set_block_and_update(pos, ns);
        }
        false
    }

    // ----- pressure plates (no entities are simulated: nothing presses them) -----------

    pub(crate) fn plate_signal(&self, s: State) -> i32 {
        if self.d.has_prop(s, prop::POWER) {
            self.d.get_int(s, prop::POWER)
        } else if self.d.get_bool(s, prop::POWERED) {
            15
        } else {
            0
        }
    }

    fn plate_with_signal(&self, s: State, sig: i32) -> State {
        if self.d.has_prop(s, prop::POWER) {
            self.d.with_int(s, prop::POWER, sig)
        } else {
            self.d.with_bool(s, prop::POWERED, sig > 0)
        }
    }

    pub(crate) fn plate_update_neighbours(&mut self, s: State, pos: BlockPos) {
        let b = self.d.block_of(s);
        self.update_neighbors_at(pos, b);
        self.update_neighbors_at(pos.below(), b);
    }

    pub(crate) fn plate_can_survive(&self, pos: BlockPos) -> bool {
        let below = pos.below();
        let bs = self.get(below);
        self.sturdy(bs, below, Dir::Up, Support::Rigid) || self.sturdy(bs, below, Dir::Up, Support::Center)
    }

    pub(crate) fn plate_tick(&mut self, s: State, pos: BlockPos) {
        let old = self.plate_signal(s);
        if old > 0 {
            // checkPressed with no entities: signal strength 0
            let ns = self.plate_with_signal(s, 0);
            self.set_block(pos, ns, 2);
            self.plate_update_neighbours(s, pos);
            self.game_event(crate::vibration::GameEvent::BlockDeactivate, pos, None);
        }
    }

    // ----- skulls -----------------------------------------------------------------------

    /// Returns true if the power already matches (nothing to do).
    pub(crate) fn skull_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let signal = self.has_neighbor_signal(pos);
        if signal == self.d.get_bool(s, prop::POWERED) {
            return true;
        }
        let ns = self.d.with_bool(s, prop::POWERED, signal);
        self.set_block(pos, ns, 2);
        false
    }

    // ----- containers & analog outputs ---------------------------------------------------

    pub(crate) fn container_has_items(&self, pos: BlockPos) -> bool {
        match self.bes().get(&pos) {
            Some(be) => match &be.data {
                BeData::Container(c) => c.items.iter().any(|it| it.count > 0),
                _ => false,
            },
            None => false,
        }
    }

    pub(crate) fn container_signal(&self, pos: BlockPos) -> i32 {
        match self.bes().get(&pos) {
            Some(be) => match &be.data {
                BeData::Container(c) => c.signal(),
                _ => 0,
            },
            None => 0,
        }
    }

    /// `getAnalogOutputSignal` for blocks whose output depends only on their state or contents.
    pub(crate) fn analog_output_impl(&mut self, s: State, pos: BlockPos) -> Option<i32> {
        let d = self.d;
        let b = d.block_of(s);
        let classes = &d.binfo(b).classes;
        let has = |c: &str| classes.iter().any(|x| x == c);
        Some(match self.kind(s) {
            Kind::CopperBulb => {
                let cur = self.get(pos);
                if d.has_prop(cur, prop::LIT) && d.get_bool(cur, prop::LIT) {
                    15
                } else {
                    0
                }
            }
            Kind::Lectern => self.lectern_signal(s, pos),
            Kind::Jukebox => self.jukebox_signal(pos),
            Kind::ChiseledBookshelf => self.bookshelf_signal(pos),
            Kind::Crafter => self.crafter_signal(pos),
            Kind::DetectorRail => 0,
            Kind::SculkSensor => self.sculk_analog(s, pos),
            Kind::Statue => self.statue_analog(s),
            _ if has("CakeBlock") => (7 - d.get_int(s, prop::BITES)) * 2,
            _ if has("CandleCakeBlock") => 14,
            _ if has("ComposterBlock") => d.get_int(s, prop::LEVEL_COMPOSTER),
            _ if has("LayeredCauldronBlock") => d.get_int(s, prop::LEVEL_CAULDRON),
            _ if b == blk::LAVA_CAULDRON => 3,
            _ if b == blk::CAULDRON => 0,
            _ if has("EndPortalFrameBlock") => {
                if d.get_bool(s, prop::EYE) {
                    15
                } else {
                    0
                }
            }
            _ if has("RespawnAnchorBlock") => {
                let c = d.get_int(s, prop::RESPAWN_ANCHOR_CHARGES);
                (c as f32 / 4.0 * 15.0).floor() as i32
            }
            _ if has("BeehiveBlock") => d.get_int(s, prop::LEVEL_HONEY),
            _ if has("BaseContainerBlockEntity") || matches!(self.kind(s), Kind::Dispenser | Kind::Hopper) || has("BarrelBlock") || has("ShulkerBoxBlock") || has("AbstractFurnaceBlock") || has("BrewingStandBlock") || has("DecoratedPotBlock") => {
                self.container_signal(pos)
            }
            _ if has("ChestBlock") => self.chest_signal(s, pos),
            _ => return None,
        })
    }

    /// `ChestBlock.getConnectedDirection`
    fn chest_connected_direction(&self, s: State) -> Dir {
        let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
        if self.d.get_str(s, prop::CHEST_TYPE) == "left" {
            f.cw()
        } else {
            f.ccw()
        }
    }

    pub(crate) fn container_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, ns: State) -> State {
        self.warn_waterlogged_pub(s, pos);
        if !self.d.has_prop(s, prop::CHEST_TYPE) {
            return s;
        }
        let same = self.d.block_of(ns) == self.d.block_of(s);
        if same && dir.is_horizontal() {
            let nt = self.d.get_str(ns, prop::CHEST_TYPE);
            if self.d.get_str(s, prop::CHEST_TYPE) == "single"
                && nt != "single"
                && self.d.get_dir(s, prop::HORIZONTAL_FACING) == self.d.get_dir(ns, prop::HORIZONTAL_FACING)
                && self.chest_connected_direction(ns) == dir.opposite()
            {
                let opp = if nt == "left" { "right" } else { "left" };
                return self.d.with_str(s, prop::CHEST_TYPE, opp);
            }
        } else if self.chest_connected_direction(s) == dir {
            return self.d.with_str(s, prop::CHEST_TYPE, "single");
        }
        s
    }

    fn chest_blocked(&self, pos: BlockPos) -> bool {
        // isBlockedChestByBlock (cats are entities: not simulated)
        self.is_conductor(self.get(pos.above()))
    }

    fn chest_items(&self, pos: BlockPos) -> Option<&crate::world::ContainerData> {
        match self.bes().get(&pos) {
            Some(be) => match &be.data {
                BeData::Container(c) => Some(c),
                _ => None,
            },
            None => None,
        }
    }

    /// `ChestBlock.getContainer(.., ignoreBeingBlocked=false)` -> redstone signal (double chests combined).
    fn chest_signal(&self, s: State, pos: BlockPos) -> i32 {
        let Some(own) = self.chest_items(pos) else { return 0 };
        if self.chest_blocked(pos) {
            return 0;
        }
        let ty = self.d.get_str(s, prop::CHEST_TYPE);
        if ty == "single" {
            return own.signal();
        }
        let np = pos.rel(self.chest_connected_direction(s));
        let ns = self.get(np);
        if self.d.block_of(ns) == self.d.block_of(s) {
            let nt = self.d.get_str(ns, prop::CHEST_TYPE);
            if nt != "single" && nt != ty && self.d.get_dir(ns, prop::HORIZONTAL_FACING) == self.d.get_dir(s, prop::HORIZONTAL_FACING) {
                if self.chest_blocked(np) {
                    return 0;
                }
                if let Some(other) = self.chest_items(np) {
                    // RIGHT is the first half of the compound container
                    let (first, second) = if ty == "right" { (own, other) } else { (other, own) };
                    let mut items: Vec<crate::world::ItemStack> = first.items.clone();
                    for it in &second.items {
                        let mut i2 = it.clone();
                        i2.slot += first.size;
                        items.push(i2);
                    }
                    let combined = crate::world::ContainerData { size: first.size + second.size, max_stack: first.max_stack, items, hopper: None, disabled_slots: 0 };
                    return combined.signal();
                }
            }
        }
        own.signal()
    }

    #[allow(dead_code)]
    fn _air(&self) -> State {
        AIR
    }
}
