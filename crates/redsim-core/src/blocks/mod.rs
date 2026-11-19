//! Block behaviour dispatch: the `BlockBehaviour` virtual methods of the game,
//! routed by block kind to the per-component ports in the sibling modules.

pub mod connect;
pub mod diode;
pub mod doors;
pub mod extra;
pub mod items;
pub mod misc;
pub mod more;
pub mod observer;
pub mod piston;
pub mod rails;
pub mod shelf;
pub mod torch;
pub mod tripwire;
pub mod wire;

use crate::data::{flag, BlockId, Kind, State, AIR};
use crate::gen::{blk, prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::world::World;

impl World {
    // ----- signals (SignalGetter + BlockBehaviour) -----------------------------

    /// `BlockState.isSignalSource()`
    #[inline]
    pub fn is_signal_source(&self, s: State) -> bool {
        match self.kind(s) {
            Kind::Wire => self.wire_should_signal,
            _ => self.sflags(s) & flag::SIGNAL_SOURCE != 0,
        }
    }

    /// `BlockState.getSignal(level, pos, direction)` (weak power emitted towards `-dir`).
    #[inline(always)]
    pub fn state_signal(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if self.d.signal_bits(s) & crate::data::SIGNAL_WEAK == 0 {
            return 0;
        }
        // dust gives no signal while dust is computing its own power (shouldSignal = false)
        if !self.wire_should_signal && self.kind(s) == Kind::Wire {
            return 0;
        }
        self.state_signal_of(s, pos, dir)
    }

    #[inline(never)]
    fn state_signal_of(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        match self.kind(s) {
            Kind::Air | Kind::Plain => 0,
            Kind::Wire => self.wire_get_signal(s, pos, dir),
            Kind::Repeater | Kind::Comparator => self.diode_get_signal(s, pos, dir),
            Kind::Torch => {
                if dir != Dir::Up {
                    self.torch_own_signal(s)
                } else {
                    0
                }
            }
            Kind::WallTorch => {
                if self.d.get_dir(s, prop::HORIZONTAL_FACING) != dir {
                    self.torch_own_signal(s)
                } else {
                    0
                }
            }
            Kind::Lever | Kind::Button => {
                if self.d.get_bool(s, prop::POWERED) {
                    15
                } else {
                    0
                }
            }
            Kind::PoweredBlock => 15,
            Kind::Observer => {
                if self.d.get_dir(s, prop::FACING) == dir && self.d.get_bool(s, prop::POWERED) {
                    15
                } else {
                    0
                }
            }
            Kind::Target | Kind::DaylightDetector => self.d.get_int(s, prop::POWER),
            Kind::SculkSensor => self.sculk_signal(s, dir),
            Kind::PressurePlate => self.plate_signal(s),
            Kind::LightningRod | Kind::Lectern | Kind::DetectorRail | Kind::TripwireHook => {
                if self.d.get_bool(s, prop::POWERED) {
                    15
                } else {
                    0
                }
            }
            Kind::Jukebox => {
                if self.jukebox_playing(pos) {
                    15
                } else {
                    0
                }
            }
            _ => self.own_signal_generic(s, pos),
        }
    }

    fn own_signal_generic(&self, _s: State, _pos: BlockPos) -> i32 {
        0
    }

    /// `BlockState.getDirectSignal(level, pos, direction)` (strong power).
    #[inline(always)]
    pub fn state_direct_signal(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if self.d.signal_bits(s) & crate::data::SIGNAL_DIRECT == 0 {
            return 0;
        }
        if !self.wire_should_signal && self.kind(s) == Kind::Wire {
            return 0;
        }
        self.state_direct_signal_of(s, pos, dir)
    }

    #[inline(never)]
    fn state_direct_signal_of(&self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        match self.kind(s) {
            Kind::Air | Kind::Plain => 0,
            Kind::Wire => {
                if !self.wire_should_signal {
                    0
                } else {
                    self.wire_get_signal(s, pos, dir)
                }
            }
            Kind::Repeater | Kind::Comparator => self.diode_get_signal(s, pos, dir),
            Kind::Torch | Kind::WallTorch => {
                if dir == Dir::Down {
                    self.state_signal(s, pos, dir)
                } else {
                    0
                }
            }
            Kind::Lever | Kind::Button => {
                if self.d.get_bool(s, prop::POWERED) && misc::connected_direction(self, s) == dir {
                    15
                } else {
                    0
                }
            }
            Kind::Observer => self.state_signal(s, pos, dir),
            Kind::PressurePlate => {
                if dir == Dir::Up {
                    self.plate_signal(s)
                } else {
                    0
                }
            }
            Kind::SculkSensor => {
                if dir == Dir::Up {
                    self.sculk_signal(s, dir)
                } else {
                    0
                }
            }
            Kind::TripwireHook => {
                if self.d.get_bool(s, prop::POWERED) && self.d.get_dir(s, prop::HORIZONTAL_FACING) == dir {
                    15
                } else {
                    0
                }
            }
            Kind::LightningRod => {
                if self.d.get_bool(s, prop::POWERED) && self.d.get_dir(s, prop::FACING) == dir {
                    15
                } else {
                    0
                }
            }
            Kind::Lectern | Kind::DetectorRail => {
                if dir == Dir::Up && self.d.get_bool(s, prop::POWERED) {
                    15
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    /// `SignalGetter.getSignal(pos, direction)`
    #[inline]
    pub fn get_signal(&self, pos: BlockPos, dir: Dir) -> i32 {
        let s = self.get(pos);
        let sig = self.state_signal(s, pos, dir);
        if self.is_conductor(s) {
            sig.max(self.get_direct_signal_to(pos))
        } else {
            sig
        }
    }

    #[inline]
    pub fn has_signal(&self, pos: BlockPos, dir: Dir) -> bool {
        self.get_signal(pos, dir) > 0
    }

    /// `SignalGetter.getDirectSignal(pos, direction)`
    #[inline]
    pub fn get_direct_signal(&self, pos: BlockPos, dir: Dir) -> i32 {
        let s = self.get(pos);
        self.state_direct_signal(s, pos, dir)
    }

    /// `SignalGetter.getDirectSignalTo(pos)`: strong power into a block.
    pub fn get_direct_signal_to(&self, pos: BlockPos) -> i32 {
        let mut r = 0;
        for d in [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East] {
            r = r.max(self.get_direct_signal(pos.rel(d), d));
            if r >= 15 {
                return r;
            }
        }
        r
    }

    /// `SignalGetter.hasNeighborSignal(pos)`
    pub fn has_neighbor_signal(&self, pos: BlockPos) -> bool {
        for d in [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East] {
            if self.get_signal(pos.rel(d), d) > 0 {
                return true;
            }
        }
        false
    }

    /// `SignalGetter.getBestNeighborSignal(pos)`
    pub fn get_best_neighbor_signal(&self, pos: BlockPos) -> i32 {
        let mut best = 0;
        for d in Dir::ALL {
            let s = self.get_signal(pos.rel(d), d);
            if s >= 15 {
                return 15;
            }
            if s > best {
                best = s;
            }
        }
        best
    }

    /// `SignalGetter.getControlInputSignal(pos, direction, onlyDiodes)`
    pub fn get_control_input_signal(&self, pos: BlockPos, dir: Dir, only_diodes: bool) -> i32 {
        let s = self.get(pos);
        if only_diodes {
            if matches!(self.kind(s), Kind::Repeater | Kind::Comparator) {
                self.get_direct_signal(pos, dir)
            } else {
                0
            }
        } else if self.is(s, blk::REDSTONE_BLOCK) {
            15
        } else if self.is(s, blk::REDSTONE_WIRE) {
            self.d.get_int(s, prop::POWER)
        } else if self.is_signal_source(s) {
            self.get_direct_signal(pos, dir)
        } else {
            0
        }
    }

    // ----- analog output (comparator input) --------------------------------------

    #[inline]
    pub fn has_analog_output(&self, s: State) -> bool {
        self.sflags(s) & flag::ANALOG != 0
    }

    /// `BlockState.getAnalogOutputSignal(level, pos, direction)`
    pub fn analog_output(&mut self, s: State, pos: BlockPos, dir: Dir) -> i32 {
        if self.kind(s) == Kind::Shelf {
            return self.shelf_analog(s, pos, dir);
        }
        match self.analog_output_impl(s, pos) {
            Some(v) => v,
            None => {
                let name = self.d.binfo(self.d.block_of(s)).short.clone();
                self.warn(pos, format!("comparator reads {}: its analog output is not simulated yet, using 0", name));
                0
            }
        }
    }

    // ----- connection / survival ---------------------------------------------------

    /// `BlockState.shouldRedstoneWireConnectTo(level, pos, direction)`
    pub fn should_wire_connect_to(&self, s: State, _pos: BlockPos, dir: Option<Dir>) -> bool {
        match self.kind(s) {
            Kind::Wire => true,
            Kind::Repeater => {
                let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
                Some(f) == dir || Some(f.opposite()) == dir
            }
            Kind::Observer => Some(self.d.get_dir(s, prop::FACING)) == dir,
            _ => self.is_signal_source(s) && dir.is_some(),
        }
    }

    /// `BlockState.canSurvive(level, pos)`
    pub fn can_survive(&self, s: State, pos: BlockPos) -> bool {
        match self.kind(s) {
            Kind::Wire => {
                let bp = pos.below();
                self.wire_can_survive_on(self.get(bp), bp)
            }
            Kind::Repeater | Kind::Comparator => self.sturdy_rigid(self.get(pos.below()), pos.below(), Dir::Up),
            Kind::Torch | Kind::DecoTorch => self.sturdy_center(self.get(pos.below()), pos.below(), Dir::Up),
            Kind::Cake => self.sflags(self.get(pos.below())) & flag::SOLID != 0,
            Kind::Carpet => !self.is_air(self.get(pos.below())),
            Kind::Vegetation => {
                let below = self.d.block_of(self.get(pos.below()));
                let classes = &self.d.binfo(self.d.block_of(s)).classes;
                let t = if classes.iter().any(|c| c == "DryVegetationBlock") {
                    tag::SUPPORTS_DRY_VEGETATION
                } else if classes.iter().any(|c| c == "WitherRoseBlock") {
                    tag::SUPPORTS_WITHER_ROSE
                } else if classes.iter().any(|c| c == "AzaleaBlock") {
                    tag::SUPPORTS_AZALEA
                } else {
                    tag::SUPPORTS_VEGETATION
                };
                self.d.has_tag(below, t)
            }
            Kind::WallTorch | Kind::DecoWallTorch => {
                let f = self.d.get_dir(s, prop::HORIZONTAL_FACING);
                let p = pos.rel(f.opposite());
                self.sturdy_full(self.get(p), p, f)
            }
            Kind::Lever | Kind::Button => {
                let d = misc::connected_direction(self, s).opposite();
                let rp = pos.rel(d);
                self.sturdy_full(self.get(rp), rp, d.opposite())
            }
            Kind::PistonHead => self.piston_head_can_survive(s, pos),
            Kind::Door => self.door_can_survive(s, pos),
            Kind::Bell => self.bell_can_survive(s, pos),
            Kind::PressurePlate => self.plate_can_survive(pos),
            Kind::Rail | Kind::PoweredRail | Kind::DetectorRail => self.rail_can_survive(pos),
            Kind::TripwireHook => self.hook_can_survive(s, pos),
            Kind::Dripleaf => self.dripleaf_can_survive(pos),
            Kind::DripleafStem => self.dripleaf_stem_can_survive(pos),
            _ => true,
        }
    }

    // ----- BlockBehaviour hooks ------------------------------------------------------

    /// `BlockState.handleNeighborChanged`
    ///
    /// Most neighbour updates in busy redstone find the block already in the state its power
    /// asks for. Handlers of the kinds in `Kind::skips_repeats` return true when they did nothing
    /// for a reason that depends only on block states and block entities; the call is then
    /// recorded, and repeating it with nothing in the world changed is skipped (`noop_hit`).
    pub fn handle_neighbor_changed(&mut self, s: State, pos: BlockPos, block: BlockId, moved: bool) {
        let kind = self.kind(s);
        if kind.skips_repeats() && self.noop_hit(pos, s) {
            return;
        }
        let settled = match kind {
            Kind::Air | Kind::Plain => false,
            Kind::Wire => self.wire_neighbor_changed(s, pos, block, moved),
            Kind::Repeater | Kind::Comparator => self.diode_neighbor_changed(s, pos),
            Kind::Torch | Kind::WallTorch => self.torch_neighbor_changed(s, pos),
            Kind::Lamp => self.lamp_neighbor_changed(s, pos),
            Kind::Piston => self.piston_check_if_extend(pos, s),
            Kind::Door => self.door_neighbor_changed(s, pos, block),
            Kind::Trapdoor => self.trapdoor_neighbor_changed(s, pos),
            Kind::FenceGate => self.fence_gate_neighbor_changed(s, pos),
            Kind::NoteBlock => self.note_neighbor_changed(s, pos),
            Kind::Dispenser => self.dispenser_neighbor_changed(s, pos),
            Kind::Hopper => self.hopper_check_powered(s, pos),
            Kind::CopperBulb => self.copper_bulb_check_and_flip(s, pos),
            Kind::Skull => self.skull_neighbor_changed(s, pos),
            Kind::PoweredRail => self.rail_neighbor_changed(s, pos, block, moved),
            other => {
                self.handle_neighbor_changed_other(other, s, pos, block, moved);
                false
            }
        };
        if settled && kind.skips_repeats() {
            self.noop_record(pos, s);
        }
    }

    fn handle_neighbor_changed_other(&mut self, kind: Kind, s: State, pos: BlockPos, block: BlockId, moved: bool) {
        match kind {
            Kind::PistonHead => self.piston_head_neighbor_changed(s, pos, block),
            Kind::Tnt => self.tnt_check(pos),
            Kind::Bell => self.bell_neighbor_changed(s, pos),
            Kind::Crafter => self.crafter_neighbor_changed(s, pos),
            Kind::Rail | Kind::DetectorRail => {
                self.rail_neighbor_changed(s, pos, block, moved);
            }
            Kind::Shelf => self.shelf_neighbor_changed(s, pos),
            Kind::Dripleaf => self.dripleaf_neighbor_changed(s, pos),
            Kind::Unsupported => self.warn_unsupported(s, pos),
            _ => {}
        }
    }

    /// `BlockState.updateShape(...)`: returns the new state for `pos`.
    pub fn update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        match self.kind(s) {
            Kind::Air | Kind::Plain => s,
            Kind::Wire => self.wire_update_shape(s, pos, dir, npos, ns),
            Kind::Repeater => {
                if dir == Dir::Down && !self.sturdy_rigid(ns, npos, Dir::Up) {
                    AIR
                } else if dir.axis() != self.d.get_dir(s, prop::HORIZONTAL_FACING).axis() {
                    let locked = self.repeater_is_locked(pos, s);
                    self.d.with_bool(s, prop::LOCKED, locked)
                } else {
                    s
                }
            }
            Kind::Comparator => {
                if dir == Dir::Down && !self.sturdy_rigid(ns, npos, Dir::Up) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Torch | Kind::DecoTorch | Kind::Cake => {
                if dir == Dir::Down && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Snowy => {
                if dir == Dir::Up {
                    let snowy = self.d.has_tag(self.d.block_of(ns), tag::SNOW);
                    self.d.with_bool(s, prop::SNOWY, snowy)
                } else {
                    s
                }
            }
            Kind::Carpet | Kind::Vegetation => {
                if !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::WallTorch | Kind::DecoWallTorch => {
                if dir.opposite() == self.d.get_dir(s, prop::HORIZONTAL_FACING) && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Lever | Kind::Button => {
                if misc::connected_direction(self, s).opposite() == dir && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Observer => {
                if self.d.get_dir(s, prop::FACING) == dir && !self.d.get_bool(s, prop::POWERED) {
                    self.observer_start_signal(pos);
                }
                s
            }
            Kind::PistonHead => {
                if dir.opposite() == self.d.get_dir(s, prop::FACING) && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Door => self.door_update_shape(s, pos, dir, ns),
            Kind::Trapdoor | Kind::WaterlogOnly => self.waterlog_update_shape(s, pos),
            Kind::FenceGate => self.fence_gate_update_shape(s, pos, dir, ns),
            Kind::NoteBlock => self.note_update_shape(s, pos, dir),
            Kind::Wall => self.wall_update_shape(s, pos, dir, npos, ns),
            Kind::Fence => self.fence_update_shape(s, pos, dir, npos, ns),
            Kind::Bars => self.bars_update_shape(s, pos, dir, npos, ns),
            Kind::Stairs => self.stairs_update_shape(s, pos, dir),
            Kind::Bell => self.bell_update_shape(s, pos, dir, npos, ns),
            Kind::Dripleaf => self.dripleaf_update_shape(s, pos, dir, ns),
            Kind::DripleafStem => self.dripleaf_stem_update_shape(s, pos, dir),
            Kind::LightningRod | Kind::Rail | Kind::PoweredRail | Kind::DetectorRail | Kind::SculkSensor | Kind::Shelf | Kind::Statue => {
                self.waterlog_update_shape(s, pos)
            }
            Kind::Tripwire => self.tripwire_update_shape(s, dir, ns),
            Kind::TripwireHook => {
                if dir.opposite() == self.d.get_dir(s, prop::HORIZONTAL_FACING) && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Container => self.container_update_shape(s, pos, dir, ns),
            Kind::PressurePlate => {
                if dir == Dir::Down && !self.can_survive(s, pos) {
                    AIR
                } else {
                    s
                }
            }
            Kind::Unsupported => {
                self.warn_unsupported(s, pos);
                s
            }
            _ => s,
        }
    }

    /// `BlockState.updateIndirectNeighbourShapes`
    pub fn update_indirect_neighbour_shapes(&mut self, s: State, pos: BlockPos, fl: i32, limit: i32) {
        if self.kind(s) == Kind::Wire {
            self.wire_update_indirect_neighbour_shapes(s, pos, fl, limit);
        }
    }

    /// `BlockState.tick` (scheduled tick)
    pub fn tick_block(&mut self, s: State, pos: BlockPos) {
        match self.kind(s) {
            Kind::Repeater => self.diode_tick(s, pos),
            Kind::Comparator => self.comparator_refresh_output(s, pos),
            Kind::Torch | Kind::WallTorch => self.torch_tick(s, pos),
            Kind::Lamp => {
                if self.d.get_bool(s, prop::LIT) && !self.has_neighbor_signal(pos) {
                    let ns = self.d.cycle(s, prop::LIT);
                    self.set_block(pos, ns, 2);
                }
            }
            Kind::Observer => self.observer_tick(s, pos),
            Kind::Button => self.button_tick(s, pos),
            Kind::Target => {
                if self.d.get_int(s, prop::POWER) != 0 {
                    let ns = self.d.with_int(s, prop::POWER, 0);
                    self.set_block_and_update(pos, ns);
                }
            }
            Kind::Dispenser => self.dispenser_tick(s, pos),
            Kind::LightningRod => self.rod_tick(s, pos),
            Kind::Lectern => self.lectern_tick(s, pos),
            Kind::Crafter => self.crafter_tick(s, pos),
            Kind::DetectorRail => self.detector_tick(s, pos),
            Kind::SculkSensor => self.sculk_tick(s, pos),
            Kind::Dripleaf => self.dripleaf_tick(s, pos),
            Kind::DripleafStem => self.dripleaf_stem_tick(pos),
            Kind::Tripwire => self.tripwire_tick(pos),
            Kind::TripwireHook => self.hook_tick(s, pos),
            Kind::StaticAnalog => self.cauldron_tick(pos),
            Kind::Composter => {
                if self.d.get_int(s, prop::LEVEL_COMPOSTER) == 7 {
                    let ns = self.d.cycle(s, prop::LEVEL_COMPOSTER);
                    self.set_block_and_update(pos, ns);
                }
            }
            Kind::PressurePlate => self.plate_tick(s, pos),
            Kind::Unsupported => self.warn_unsupported(s, pos),
            _ => {}
        }
    }

    /// `BlockState.onPlace`
    pub fn on_place(&mut self, s: State, pos: BlockPos, old: State, moved: bool) {
        match self.kind(s) {
            Kind::Wire => self.wire_on_place(s, pos, old),
            Kind::Repeater | Kind::Comparator => self.diode_update_neighbors_in_front(pos, s),
            Kind::Torch | Kind::WallTorch => self.torch_notify_neighbors(pos, s),
            Kind::Observer => self.observer_on_place(s, pos, old),
            Kind::Piston => self.piston_on_place(s, pos, old),
            Kind::Target => {
                if !self.is(old, blk::TARGET) && self.d.get_int(s, prop::POWER) > 0 && !self.has_scheduled_tick(pos, blk::TARGET) {
                    let ns = self.d.with_int(s, prop::POWER, 0);
                    self.set_block(pos, ns, 18);
                }
            }
            Kind::Composter => {
                if self.d.get_int(s, prop::LEVEL_COMPOSTER) == 7 {
                    let b = self.d.block_of(s);
                    self.schedule_tick(pos, b, 20, 0);
                }
            }
            Kind::Tnt => {
                if !self.is(old, blk::TNT) {
                    self.tnt_check(pos);
                }
            }
            Kind::LightningRod => self.rod_on_place(s, pos, old),
            Kind::Rail | Kind::PoweredRail | Kind::DetectorRail => self.rail_on_place(s, pos, old, moved),
            Kind::Tripwire => self.tripwire_on_place(s, pos, old),
            Kind::SculkSensor => self.sculk_on_place(s, pos, old),
            Kind::Shelf => self.shelf_on_place(s, pos, old),
            Kind::Hopper => {
                if !self.is(old, blk::HOPPER) {
                    self.hopper_check_powered(s, pos);
                }
            }
            Kind::CopperBulb => {
                if self.d.block_of(old) != self.d.block_of(s) {
                    self.copper_bulb_check_and_flip(s, pos);
                }
            }
            Kind::Unsupported => self.warn_unsupported(s, pos),
            _ => {
                let _ = moved;
            }
        }
    }

    /// `BlockState.affectNeighborsAfterRemoval`
    pub fn affect_neighbors_after_removal(&mut self, s: State, pos: BlockPos, moved: bool) {
        match self.kind(s) {
            Kind::Wire => self.wire_affect_neighbors_after_removal(s, pos, moved),
            Kind::Repeater | Kind::Comparator => {
                if !moved {
                    self.diode_update_neighbors_in_front(pos, s);
                }
            }
            Kind::Torch | Kind::WallTorch => {
                if !moved {
                    self.torch_notify_neighbors(pos, s);
                }
            }
            Kind::Lever | Kind::Button => {
                if !moved && self.d.get_bool(s, prop::POWERED) {
                    self.attached_update_neighbours(s, pos);
                }
            }
            Kind::Observer => self.observer_affect_neighbors_after_removal(s, pos),
            Kind::PistonHead => self.piston_head_affect_neighbors_after_removal(s, pos),
            Kind::LightningRod => self.rod_affect_after_removal(s, pos),
            Kind::Lectern => self.lectern_affect_after_removal(s, pos),
            Kind::Rail | Kind::PoweredRail | Kind::DetectorRail => self.rail_affect_after_removal(s, pos, moved),
            Kind::Tripwire => self.tripwire_affect_after_removal(s, pos, moved),
            Kind::TripwireHook => self.hook_affect_after_removal(s, pos, moved),
            Kind::SculkSensor => self.sculk_affect_after_removal(s, pos),
            Kind::Shelf => self.shelf_affect_after_removal(s, pos),
            Kind::Statue => {
                let b = self.d.block_of(s);
                self.update_neighbour_for_output_signal(pos, b);
            }
            Kind::Dispenser | Kind::Hopper | Kind::Container | Kind::ChiseledBookshelf | Kind::Jukebox | Kind::Crafter => {
                let b = self.d.block_of(s);
                self.update_neighbour_for_output_signal(pos, b);
            }
            Kind::PressurePlate => {
                if !moved && self.plate_signal(s) > 0 {
                    self.plate_update_neighbours(s, pos);
                }
            }
            Kind::Unsupported => self.warn_unsupported(s, pos),
            _ => {}
        }
    }

    /// `BlockState.triggerEvent` (block events)
    pub fn trigger_event(&mut self, s: State, pos: BlockPos, a: i32, b: i32) -> bool {
        match self.kind(s) {
            Kind::Piston => self.piston_trigger_event(s, pos, a, b),
            Kind::Comparator => false,
            Kind::NoteBlock | Kind::Bell => true,
            Kind::Unsupported => {
                self.warn_unsupported(s, pos);
                false
            }
            _ => false,
        }
    }

    pub(crate) fn warn_unsupported(&mut self, s: State, pos: BlockPos) {
        let name = self.d.binfo(self.d.block_of(s)).name.clone();
        self.warn(pos, format!("{} has game behaviour that redsim does not simulate yet", name));
    }
}

#[cfg(test)]
mod tests {
    use crate::data::{data, State, SIGNAL_DIRECT, SIGNAL_WEAK};
    use crate::pos::{BlockPos, Dir};
    use crate::world::World;

    /// The signal pre-check skips states whose kind can only return 0: check every state, in
    /// every direction, among random neighbours (and with its own block entity, if any).
    #[test]
    fn signal_bits_only_skip_zero_signals() {
        let d = data();
        let n = d.states.len() as u32;
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let c = BlockPos::new(-7, 70, 12);
        let mut w = World::new();
        for s in 0..n {
            if s % 512 == 0 {
                w = World::new();
                for dir in Dir::ALL {
                    w.place_raw(c.rel(dir), (rnd() % n as u64) as State);
                }
            }
            let bits = d.signal_bits(s);
            if bits == (SIGNAL_WEAK | SIGNAL_DIRECT) {
                continue;
            }
            w.place_raw(c, s);
            for dir in Dir::ALL {
                if bits & SIGNAL_WEAK == 0 {
                    assert_eq!(w.state_signal_of(s, c, dir), 0, "{}", d.format_state(s));
                }
                if bits & SIGNAL_DIRECT == 0 {
                    assert_eq!(w.state_direct_signal_of(s, c, dir), 0, "{}", d.format_state(s));
                }
            }
        }
    }

    /// The `power` lookup table agrees with the general property path for every state.
    #[test]
    fn power_table_matches_property() {
        let d = data();
        for s in 0..d.states.len() as State {
            if d.has_prop(s, crate::gen::prop::POWER) {
                let i = d.get_idx(s, crate::gen::prop::POWER).unwrap();
                let v = d.props[crate::gen::prop::POWER as usize].ints[i as usize].unwrap();
                assert_eq!(d.get_int(s, crate::gen::prop::POWER), v);
            }
        }
    }
}
