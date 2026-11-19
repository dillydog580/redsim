//! TNT, lightning rods, bells, lecterns, jukeboxes, chiseled bookshelves, crafters and cauldrons.

use crate::data::{Kind, State, AIR};
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::shape::Support;
use crate::world::{BeData, BlockEntity, World};

impl World {
    // ----- TNT ------------------------------------------------------------------------

    /// `TntBlock.onPlace` / `neighborChanged`: a powered TNT block primes (becomes an entity).
    pub(crate) fn tnt_check(&mut self, pos: BlockPos) {
        if self.has_neighbor_signal(pos) {
            // TntBlock.prime: spawns PrimedTnt (fuse 80), then the block is removed
            self.primed_tnt.push((pos, self.game_time + 80));
            // the primed TNT entity hops in a random direction and falls: it may press plates or
            // tripwire it lands on or passes through
            'scan: for dx in -2..=2 {
                for dy in -12..=1 {
                    for dz in -2..=2 {
                        let k = self.kind(self.get(pos.offset(dx, dy, dz)));
                        if matches!(k, Kind::PressurePlate | Kind::Tripwire) {
                            self.warn(pos, "primed TNT (an entity, not simulated) may land on a nearby pressure plate or tripwire");
                            break 'scan;
                        }
                    }
                }
            }
            // the primed TNT entity lands, bounces and explodes: vibrations sculk sensors hear
            if self.vibration_listeners.iter().any(|l| {
                let (dx, dy, dz) = ((l.x - pos.x) as i64, (l.y - pos.y) as i64, (l.z - pos.z) as i64);
                dx * dx + dy * dy + dz * dz <= 20 * 20
            }) {
                self.warn(pos, "primed TNT (an entity, not simulated) makes vibrations when it lands, bounces or explodes");
            }
            // TntBlock.prime: PRIME_FUSE, then the caller removes the block
            self.game_event(crate::vibration::GameEvent::PrimeFuse, pos, None);
            self.remove_block(pos, false);
        }
    }

    // ----- lightning rod --------------------------------------------------------------

    fn rod_update_neighbours(&mut self, s: State, pos: BlockPos) {
        let front = self.d.get_dir(s, prop::FACING).opposite();
        self.update_neighbors_at(pos.rel(front), self.d.block_of(s));
    }

    pub(crate) fn rod_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        if self.d.block_of(s) != self.d.block_of(old)
            && self.d.get_bool(s, prop::POWERED)
            && !self.has_scheduled_tick(pos, self.d.block_of(s))
        {
            self.schedule_tick(pos, self.d.block_of(s), 8, 0);
        }
    }

    pub(crate) fn rod_tick(&mut self, s: State, pos: BlockPos) {
        let ns = self.d.with_bool(s, prop::POWERED, false);
        self.set_block_and_update(pos, ns);
        self.rod_update_neighbours(s, pos);
    }

    pub(crate) fn rod_affect_after_removal(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) {
            self.rod_update_neighbours(s, pos);
        }
    }

    /// `LightningRodBlock.onLightningStrike` (API: strike a rod).
    pub fn strike_lightning_rod(&mut self, pos: BlockPos) -> bool {
        let s = self.get(pos);
        if self.kind(s) != Kind::LightningRod {
            return false;
        }
        let ns = self.d.with_bool(s, prop::POWERED, true);
        self.set_block_and_update(pos, ns);
        self.rod_update_neighbours(s, pos);
        let b = self.d.block_of(s);
        self.schedule_tick(pos, b, 8, 0);
        true
    }

    // ----- bell -------------------------------------------------------------------------

    fn bell_connected(&self, s: State) -> Dir {
        match self.d.get_str(s, prop::BELL_ATTACHMENT) {
            "floor" => Dir::Up,
            "ceiling" => Dir::Down,
            _ => self.d.get_dir(s, prop::HORIZONTAL_FACING).opposite(),
        }
    }

    pub(crate) fn bell_can_survive(&self, s: State, pos: BlockPos) -> bool {
        let c = self.bell_connected(s).opposite();
        if c == Dir::Up {
            let p = pos.above();
            self.sturdy(self.get(p), p, Dir::Down, Support::Center)
        } else {
            let p = pos.rel(c);
            self.sturdy(self.get(p), p, c.opposite(), Support::Full)
        }
    }

    pub(crate) fn bell_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        let att = self.d.get_str(s, prop::BELL_ATTACHMENT).to_string();
        let connected = self.bell_connected(s).opposite();
        if connected == dir && !self.bell_can_survive(s, pos) && att != "double_wall" {
            return AIR;
        }
        let facing = self.d.get_dir(s, prop::HORIZONTAL_FACING);
        if dir.axis() == facing.axis() {
            if att == "double_wall" && !self.sturdy(ns, npos, dir, Support::Full) {
                let t = self.d.with_str(s, prop::BELL_ATTACHMENT, "single_wall");
                return self.d.with_dir(t, prop::HORIZONTAL_FACING, dir.opposite());
            }
            if att == "single_wall" && connected.opposite() == dir && self.sturdy(ns, npos, facing, Support::Full) {
                return self.d.with_str(s, prop::BELL_ATTACHMENT, "double_wall");
            }
        }
        s
    }

    pub(crate) fn bell_neighbor_changed(&mut self, s: State, pos: BlockPos) {
        let signal = self.has_neighbor_signal(pos);
        if signal != self.d.get_bool(s, prop::POWERED) {
            if signal && self.bes().contains_key(&pos) {
                // BellBlockEntity.onHit -> block event (animation only)
                let dir = self.d.get_dir(self.get(pos), prop::HORIZONTAL_FACING);
                self.block_event(pos, blk::BELL, 1, dir.ordinal() as i32);
                self.game_event(crate::vibration::GameEvent::BlockChange, pos, None);
            }
            let ns = self.d.with_bool(s, prop::POWERED, signal);
            self.set_block_and_update(pos, ns);
        }
    }

    // ----- lectern ----------------------------------------------------------------------

    fn lectern_update_below(&mut self, s: State, pos: BlockPos) {
        self.update_neighbors_at(pos.below(), self.d.block_of(s));
    }

    fn lectern_change_powered(&mut self, s: State, pos: BlockPos, powered: bool) {
        let ns = self.d.with_bool(s, prop::POWERED, powered);
        self.set_block_and_update(pos, ns);
        self.lectern_update_below(s, pos);
    }

    pub(crate) fn lectern_tick(&mut self, s: State, pos: BlockPos) {
        self.lectern_change_powered(s, pos, false);
    }

    pub(crate) fn lectern_affect_after_removal(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) {
            self.lectern_update_below(s, pos);
        }
    }

    /// A player turning the lectern's book to `page` (`LecternBlockEntity.setPage` -> signalPageChange).
    pub fn lectern_turn_page(&mut self, pos: BlockPos, page: i32) -> bool {
        let s = self.get(pos);
        if self.kind(s) != Kind::Lectern || !self.d.get_bool(s, prop::HAS_BOOK) {
            return false;
        }
        let changed = match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Lectern { page: p, pages }, .. }) => {
                let np = page.clamp(0, (*pages - 1).max(0));
                if np != *p {
                    *p = np;
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if changed {
            // LecternBlockEntity.setPage: setChanged() (comparators) first, then signalPageChange
            let b = self.d.block_of(s);
            self.update_neighbour_for_output_signal(pos, b);
            let s = self.get(pos);
            if self.kind(s) != Kind::Lectern {
                return true;
            }
            self.lectern_change_powered(s, pos, true);
            let b = self.d.block_of(s);
            self.schedule_tick(pos, b, 2, 0);
        }
        changed
    }

    pub(crate) fn lectern_signal(&self, s: State, pos: BlockPos) -> i32 {
        if !self.d.get_bool(s, prop::HAS_BOOK) {
            return 0;
        }
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Lectern { page, pages }, .. }) => {
                let progress: f32 = if *pages > 1 { *page as f32 / (*pages as f32 - 1.0) } else { 1.0 };
                (progress * 14.0).floor() as i32 + 1
            }
            _ => 0,
        }
    }

    // ----- jukebox ----------------------------------------------------------------------

    pub(crate) fn jukebox_playing(&self, pos: BlockPos) -> bool {
        matches!(self.bes().get(&pos), Some(BlockEntity { data: BeData::Jukebox { playing: true, .. }, .. }))
    }

    pub(crate) fn jukebox_signal(&self, pos: BlockPos) -> i32 {
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Jukebox { item: Some(id), .. }, .. }) => self.d.jukebox_songs.get(id).map_or(0, |v| v.0),
            _ => 0,
        }
    }

    /// `JukeboxSongPlayer.tick`
    pub(crate) fn jukebox_tick(&mut self, pos: BlockPos, state: State) {
        let mut stop = false;
        let d = self.d;
        if let Some(BlockEntity { data: BeData::Jukebox { item, ticks, playing }, .. }) = self.bes_mut().get_mut(&pos) {
            if *playing {
                let len = item.as_ref().and_then(|i| d.jukebox_songs.get(i)).map_or(0, |v| v.1);
                if *ticks >= len + 20 {
                    *playing = false;
                    *ticks = 0;
                    stop = true;
                } else {
                    *ticks += 1;
                }
            }
        }
        if stop {
            // onSongChanged: updateNeighborsAt + setChanged
            let b = self.d.block_of(state);
            self.update_neighbors_at(pos, b);
            self.update_neighbour_for_output_signal(pos, b);
        }
    }

    /// Put a music disc in a jukebox as a paste would (no updates): it plays from `ticks`.
    pub fn set_jukebox(&mut self, pos: BlockPos, disc: Option<String>, ticks: i64) -> bool {
        let len = disc.as_ref().and_then(|i| self.d.jukebox_songs.get(i)).map(|v| v.1);
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Jukebox { item, ticks: t, playing }, .. }) => {
                *playing = matches!(len, Some(l) if ticks < l + 20);
                *item = disc;
                *t = if *playing { ticks } else { 0 };
                true
            }
            _ => false,
        }
    }

    // ----- chiseled bookshelf ---------------------------------------------------------------

    pub(crate) fn bookshelf_signal(&self, pos: BlockPos) -> i32 {
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Bookshelf { last_slot }, .. }) => last_slot + 1,
            _ => 0,
        }
    }

    // ----- crafter ----------------------------------------------------------------------

    pub(crate) fn crafter_neighbor_changed(&mut self, s: State, pos: BlockPos) {
        let should = self.has_neighbor_signal(pos);
        let triggered = self.d.get_bool(s, prop::TRIGGERED);
        if should && !triggered {
            let b = self.d.block_of(s);
            self.schedule_tick(pos, b, 4, 0);
            let ns = self.d.with_bool(s, prop::TRIGGERED, true);
            self.set_block(pos, ns, 2);
        } else if !should && triggered {
            let ns = self.d.with_bool(self.d.with_bool(s, prop::TRIGGERED, false), prop::CRAFTING, false);
            self.set_block(pos, ns, 2);
        }
    }

    pub(crate) fn crafter_tick(&mut self, _s: State, pos: BlockPos) {
        if self.container_has_items(pos) {
            self.warn(pos, "crafter fires with items inside: crafting recipes are not simulated");
        }
        // empty crafter: no recipe -> levelEvent only
    }

    pub(crate) fn crafter_signal(&self, pos: BlockPos) -> i32 {
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Container(c), .. }) => {
                let mut n = 0;
                for i in 0..c.size {
                    let filled = c.items.iter().any(|it| it.slot == i && it.count > 0);
                    if filled || (c.disabled_slots >> i) & 1 == 1 {
                        n += 1;
                    }
                }
                n
            }
            _ => 0,
        }
    }

    // ----- daylight detector ------------------------------------------------------------------

    /// The output a daylight detector at pos would compute now (`updateSignalStrength`).
    pub(crate) fn daylight_target(&self, s: State, pos: BlockPos) -> i32 {
        let (cos, darken) = self.day_factors();
        let mut target = (self.sky_light(pos) - darken).max(0);
        if self.d.get_bool(s, prop::INVERTED) {
            target = 15 - target;
        } else if target > 0 {
            target = crate::light::java_round(target as f32 * cos);
        }
        target.clamp(0, 15)
    }

    pub(crate) fn daylight_is_current(&self, pos: BlockPos) -> bool {
        let s = self.get(pos);
        self.kind(s) != Kind::DaylightDetector || self.daylight_target(s, pos) == self.d.get_int(s, prop::POWER)
    }

    /// `DaylightDetectorBlock.updateSignalStrength`
    pub(crate) fn daylight_update(&mut self, s: State, pos: BlockPos) {
        let recent = self.game_time - 1;
        if self.light_changes.iter().any(|&(p, t)| {
            t >= recent && (p.x - pos.x).abs() <= 16 && (p.z - pos.z).abs() <= 16 && p.y >= pos.y - 16
        }) {
            self.warn(
                pos,
                "daylight detector read light that changed nearby within the last tick: the game updates light on a \
                 background thread, so this reading may come out a tick late",
            );
        }
        let target = self.daylight_target(s, pos);
        if self.d.get_int(s, prop::POWER) != target {
            let ns = self.d.with_int(s, prop::POWER, target);
            self.set_block_and_update(pos, ns);
        }
    }

    // ----- cauldrons ----------------------------------------------------------------------

    pub(crate) fn cauldron_tick(&mut self, pos: BlockPos) {
        let mut p = pos.above();
        for _ in 0..11 {
            let s = self.get(p);
            if self.d.block_of(s) == blk::POINTED_DRIPSTONE {
                self.warn(pos, "pointed dripstone above a cauldron: dripping is not simulated");
                return;
            }
            if !self.is_air(s) {
                return;
            }
            p = p.above();
        }
    }
}
