//! Game events and vibrations: `GameEventDispatcher`, `VibrationSystem` (listener, selector,
//! ticker) and the sculk sensor / calibrated sculk sensor blocks, including amethyst resonance.
//!
//! Only events from blocks are simulated (no entities: steps, item drops, projectiles...).

use crate::data::{Kind, State};
use crate::gen::{blk, prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::world::{BeData, BlockEntity, World};

/// The game events redstone blocks emit (all have notification radius 16).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum GameEvent {
    BlockActivate,
    BlockAttach,
    BlockChange,
    BlockClose,
    BlockDeactivate,
    BlockDestroy,
    BlockDetach,
    BlockOpen,
    BlockPlace,
    NoteBlockPlay,
    PrimeFuse,
    /// `resonate_1` .. `resonate_15` (amethyst next to an activated sensor)
    Resonate(u8),
}

impl GameEvent {
    /// `VibrationSystem.VIBRATION_FREQUENCY_FOR_EVENT`
    pub fn frequency(self) -> i32 {
        match self {
            GameEvent::BlockClose | GameEvent::BlockDeactivate | GameEvent::BlockDetach => 9,
            GameEvent::BlockOpen | GameEvent::BlockActivate | GameEvent::BlockAttach | GameEvent::PrimeFuse | GameEvent::NoteBlockPlay => 10,
            GameEvent::BlockChange => 11,
            GameEvent::BlockDestroy => 12,
            GameEvent::BlockPlace => 13,
            GameEvent::Resonate(n) => n as i32,
        }
    }

    pub fn name(self) -> String {
        match self {
            GameEvent::BlockActivate => "block_activate".into(),
            GameEvent::BlockAttach => "block_attach".into(),
            GameEvent::BlockChange => "block_change".into(),
            GameEvent::BlockClose => "block_close".into(),
            GameEvent::BlockDeactivate => "block_deactivate".into(),
            GameEvent::BlockDestroy => "block_destroy".into(),
            GameEvent::BlockDetach => "block_detach".into(),
            GameEvent::BlockOpen => "block_open".into(),
            GameEvent::BlockPlace => "block_place".into(),
            GameEvent::NoteBlockPlay => "note_block_play".into(),
            GameEvent::PrimeFuse => "prime_fuse".into(),
            GameEvent::Resonate(n) => format!("resonate_{n}"),
        }
    }
}

/// `VibrationInfo` (block events only: the origin is always a block centre).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Vibration {
    pub event: GameEvent,
    pub distance: f32,
    pub origin: BlockPos,
}

/// `SculkSensorBlockEntity` + its `VibrationSystem.Data` and `VibrationSelector`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SculkData {
    pub current: Option<Vibration>,
    pub travel: i32,
    /// selector candidate and the game time it was heard
    pub selector: Option<(Vibration, i64)>,
    pub last_frequency: i32,
}

impl World {
    fn is_calibrated(&self, s: State) -> bool {
        self.d.block_of(s) == blk::CALIBRATED_SCULK_SENSOR
    }

    fn sculk_radius(&self, s: State) -> i32 {
        if self.is_calibrated(s) {
            16
        } else {
            8
        }
    }

    fn sculk_phase(&self, s: State) -> &'static str {
        match self.d.get_str(s, prop::SCULK_SENSOR_PHASE) {
            "active" => "active",
            "cooldown" => "cooldown",
            _ => "inactive",
        }
    }

    pub(crate) fn sculk_data(&self, pos: BlockPos) -> Option<&SculkData> {
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Sculk(d), .. }) => Some(d),
            _ => None,
        }
    }

    fn sculk_data_mut(&mut self, pos: BlockPos) -> Option<&mut SculkData> {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Sculk(d), .. }) => Some(d),
            _ => None,
        }
    }

    /// `ServerLevel.gameEvent(event, Vec3.atCenterOf(pos), context)` -> `GameEventDispatcher.post`.
    /// `affected` is the context's affected block state (wool dampens).
    pub fn game_event(&mut self, ev: GameEvent, pos: BlockPos, affected: Option<State>) {
        if self.vibration_listeners.is_empty() {
            return;
        }
        if self.trace.is_some() {
            let c = self.cause;
            self.trace_event(crate::trace::TraceKind::Note { text: format!("game event {}", ev.name()) }, pos, c);
        }
        let freq = ev.frequency();
        let now = self.game_time;
        let listeners = self.vibration_listeners.clone();
        for l in listeners {
            let Some(be) = self.bes().get(&l) else { continue };
            let BeData::Sculk(data) = &be.data else { continue };
            let be_state = be.state;
            // EuclideanGameEventListenerRegistry: block distance within the listener radius
            let (dx, dy, dz) = ((l.x - pos.x) as f64, (l.y - pos.y) as f64, (l.z - pos.z) as f64);
            let dsq = dx * dx + dy * dy + dz * dz;
            let r = self.sculk_radius(be_state);
            if dsq > (r * r) as f64 {
                continue;
            }
            // VibrationSystem.Listener.handleGameEvent
            if data.current.is_some() {
                continue;
            }
            // isValidVibration: every event emitted here is in #vibrations; wool context dampens
            if let Some(a) = affected {
                if self.d.has_tag(self.d.block_of(a), tag::DAMPENS_VIBRATIONS) {
                    continue;
                }
            }
            // canReceiveVibration
            if self.is_calibrated(be_state) {
                let back = self.d.get_dir(be_state, prop::HORIZONTAL_FACING).opposite();
                let cmp = self.get_signal(l.rel(back), back);
                if cmp != 0 && freq != cmp {
                    continue;
                }
            }
            if l == pos && matches!(ev, GameEvent::BlockDestroy | GameEvent::BlockPlace) {
                continue;
            }
            if freq == 0 || self.sculk_phase(be_state) != "inactive" {
                continue;
            }
            if self.vibration_occluded(pos, l) {
                continue;
            }
            let v = Vibration { event: ev, distance: dsq.sqrt() as f32, origin: pos };
            // VibrationSelector.addCandidate
            if let Some(d) = self.sculk_data_mut(l) {
                let replace = match d.selector {
                    None => true,
                    Some((prev, t)) => {
                        if now != t {
                            false
                        } else if v.distance < prev.distance {
                            true
                        } else if v.distance > prev.distance {
                            false
                        } else {
                            v.event.frequency() > prev.event.frequency()
                        }
                    }
                };
                if replace {
                    d.selector = Some((v, now));
                }
            }
        }
    }

    /// `VibrationSystem.Listener.isOccluded`: blocked if the line from every nudged copy of the
    /// origin hits a vibration-occluding block (wool).
    fn vibration_occluded(&self, origin: BlockPos, dest: BlockPos) -> bool {
        let from = [origin.x as f64 + 0.5, origin.y as f64 + 0.5, origin.z as f64 + 0.5];
        let to = [dest.x as f64 + 0.5, dest.y as f64 + 0.5, dest.z as f64 + 0.5];
        for d in Dir::ALL {
            let (sx, sy, sz) = d.step();
            let k = 9.999999747378752E-6;
            let nudged = [from[0] + sx as f64 * k, from[1] + sy as f64 * k, from[2] + sz as f64 * k];
            if !self.line_hits(nudged, to, |w, s| w.d.has_tag(w.d.block_of(s), tag::OCCLUDES_VIBRATION_SIGNALS)) {
                return false;
            }
        }
        true
    }

    /// `BlockGetter.traverseBlocks` with a block-state predicate (`isBlockInLine`).
    fn line_hits(&self, from: [f64; 3], to: [f64; 3], hit: impl Fn(&World, State) -> bool) -> bool {
        if from == to {
            return false;
        }
        let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
        let to_x = lerp(-1.0E-7, to[0], from[0]);
        let to_y = lerp(-1.0E-7, to[1], from[1]);
        let to_z = lerp(-1.0E-7, to[2], from[2]);
        let from_x = lerp(-1.0E-7, from[0], to[0]);
        let from_y = lerp(-1.0E-7, from[1], to[1]);
        let from_z = lerp(-1.0E-7, from[2], to[2]);
        let mut bx = from_x.floor() as i32;
        let mut by = from_y.floor() as i32;
        let mut bz = from_z.floor() as i32;
        if hit(self, self.get(BlockPos::new(bx, by, bz))) {
            return true;
        }
        let (dx, dy, dz) = (to_x - from_x, to_y - from_y, to_z - from_z);
        let sign = |v: f64| -> i32 {
            if v == 0.0 {
                0
            } else if v > 0.0 {
                1
            } else {
                -1
            }
        };
        let (sx, sy, sz) = (sign(dx), sign(dy), sign(dz));
        let frac = |v: f64| v - v.floor();
        let t_dx = if sx == 0 { f64::MAX } else { sx as f64 / dx };
        let t_dy = if sy == 0 { f64::MAX } else { sy as f64 / dy };
        let t_dz = if sz == 0 { f64::MAX } else { sz as f64 / dz };
        let mut tx = t_dx * if sx > 0 { 1.0 - frac(from_x) } else { frac(from_x) };
        let mut ty = t_dy * if sy > 0 { 1.0 - frac(from_y) } else { frac(from_y) };
        let mut tz = t_dz * if sz > 0 { 1.0 - frac(from_z) } else { frac(from_z) };
        while tx <= 1.0 || ty <= 1.0 || tz <= 1.0 {
            if tx < ty {
                if tx < tz {
                    bx += sx;
                    tx += t_dx;
                } else {
                    bz += sz;
                    tz += t_dz;
                }
            } else if ty < tz {
                by += sy;
                ty += t_dy;
            } else {
                bz += sz;
                tz += t_dz;
            }
            if hit(self, self.get(BlockPos::new(bx, by, bz))) {
                return true;
            }
        }
        false
    }

    /// True if the sensor's vibration ticker has work (a vibration heard or travelling).
    pub(crate) fn sculk_ticker_live(&self, pos: BlockPos) -> bool {
        self.sculk_data(pos).map_or(false, |d| d.current.is_some() || d.selector.is_some())
    }

    /// `VibrationSystem.Ticker.tick`
    pub(crate) fn vibration_tick(&mut self, pos: BlockPos) {
        let now = self.game_time;
        let mut selected = false;
        if let Some(d) = self.sculk_data_mut(pos) {
            if d.current.is_none() {
                if let Some((v, t)) = d.selector {
                    if t < now {
                        d.current = Some(v);
                        d.travel = v.distance.floor() as i32;
                        d.selector = None;
                        selected = true;
                    }
                }
            }
        } else {
            return;
        }
        if selected {
            self.be_set_changed(pos);
        }
        let mut changed = false;
        let mut receive = None;
        if let Some(d) = self.sculk_data_mut(pos) {
            if let Some(v) = d.current {
                changed = d.travel > 0;
                d.travel = (d.travel - 1).max(0);
                if d.travel <= 0 {
                    receive = Some(v);
                }
            } else {
                return;
            }
        }
        if let Some(v) = receive {
            self.receive_vibration(pos, v);
            if let Some(d) = self.sculk_data_mut(pos) {
                d.current = None;
            }
            changed = true;
        }
        if changed {
            self.be_set_changed(pos);
        }
    }

    /// `SculkSensorBlockEntity.VibrationUser.onReceiveVibration`
    fn receive_vibration(&mut self, pos: BlockPos, v: Vibration) {
        let Some(be) = self.bes().get(&pos) else { return };
        let state = be.state;
        if self.sculk_phase(state) != "inactive" {
            return;
        }
        let freq = v.event.frequency();
        if let Some(d) = self.sculk_data_mut(pos) {
            d.last_frequency = freq;
        }
        let (dx, dy, dz) = ((v.origin.x - pos.x) as f64, (v.origin.y - pos.y) as f64, (v.origin.z - pos.z) as f64);
        let distance = (dx * dx + dy * dy + dz * dz).sqrt() as f32;
        let scale = 15.0 / self.sculk_radius(state) as f64;
        let power = (15 - (scale * distance as f64).floor() as i32).max(1);
        if self.kind(state) == Kind::SculkSensor {
            self.sculk_activate(pos, state, power, freq);
        }
    }

    fn sculk_update_neighbours(&mut self, pos: BlockPos, s: State) {
        let b = self.d.block_of(s);
        self.update_neighbors_at(pos, b);
        self.update_neighbors_at(pos.below(), b);
    }

    /// `SculkSensorBlock.activate`
    fn sculk_activate(&mut self, pos: BlockPos, s: State, power: i32, freq: i32) {
        let ns = self.d.with_int(self.d.with_str(s, prop::SCULK_SENSOR_PHASE, "active"), prop::POWER, power);
        self.set_block_and_update(pos, ns);
        let ticks = if self.is_calibrated(s) { 10 } else { 30 };
        self.schedule_tick(pos, self.d.block_of(s), ticks, 0);
        self.sculk_update_neighbours(pos, s);
        // tryResonateVibration
        if (1..=15).contains(&freq) {
            for d in Dir::ALL {
                let rp = pos.rel(d);
                let bs = self.get(rp);
                if self.d.has_tag(self.d.block_of(bs), tag::VIBRATION_RESONATORS) {
                    self.game_event(GameEvent::Resonate(freq as u8), rp, Some(bs));
                }
            }
        }
        // sculk_sensor_tendrils_clicking: not a vibration
    }

    /// `SculkSensorBlock.tick` (scheduled)
    pub(crate) fn sculk_tick(&mut self, s: State, pos: BlockPos) {
        match self.sculk_phase(s) {
            "active" => {
                // deactivate
                let ns = self.d.with_int(self.d.with_str(s, prop::SCULK_SENSOR_PHASE, "cooldown"), prop::POWER, 0);
                self.set_block_and_update(pos, ns);
                self.schedule_tick(pos, self.d.block_of(s), 10, 0);
                self.sculk_update_neighbours(pos, s);
            }
            "cooldown" => {
                let ns = self.d.with_str(s, prop::SCULK_SENSOR_PHASE, "inactive");
                self.set_block_and_update(pos, ns);
            }
            _ => {}
        }
    }

    pub(crate) fn sculk_on_place(&mut self, s: State, pos: BlockPos, old: State) {
        let b = self.d.block_of(s);
        if b != self.d.block_of(old) && self.d.get_int(s, prop::POWER) > 0 && !self.has_scheduled_tick(pos, b) {
            let ns = self.d.with_int(s, prop::POWER, 0);
            self.set_block(pos, ns, 18);
        }
    }

    pub(crate) fn sculk_affect_after_removal(&mut self, s: State, pos: BlockPos) {
        if self.sculk_phase(s) == "active" {
            self.sculk_update_neighbours(pos, s);
        }
    }

    pub(crate) fn sculk_signal(&self, s: State, dir: Dir) -> i32 {
        if self.is_calibrated(s) && self.d.get_dir(s, prop::HORIZONTAL_FACING) == dir {
            0
        } else {
            self.d.get_int(s, prop::POWER)
        }
    }

    pub(crate) fn sculk_analog(&self, s: State, pos: BlockPos) -> i32 {
        match self.sculk_data(pos) {
            Some(d) if self.sculk_phase(s) == "active" => d.last_frequency,
            _ => 0,
        }
    }
}
