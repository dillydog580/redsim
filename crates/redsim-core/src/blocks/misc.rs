//! Levers, buttons, lamps, target blocks and player interactions.

use crate::data::{Kind, State};
use crate::gen::{blk, prop};
use crate::pos::{BlockPos, Dir};
use crate::world::World;

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`
pub(crate) fn connected_direction(w: &World, s: State) -> Dir {
    match w.d.get_str(s, prop::ATTACH_FACE) {
        "ceiling" => Dir::Down,
        "floor" => Dir::Up,
        _ => w.d.get_dir(s, prop::HORIZONTAL_FACING),
    }
}

impl World {
    /// Lever/button `updateNeighbours`.
    pub(crate) fn attached_update_neighbours(&mut self, s: State, pos: BlockPos) {
        let front = connected_direction(self, s).opposite();
        let block = self.d.block_of(s);
        self.update_neighbors_at(pos, block);
        self.update_neighbors_at(pos.rel(front), block);
    }

    /// `LeverBlock.pull`
    pub(crate) fn lever_pull(&mut self, s: State, pos: BlockPos) {
        let ns = self.d.cycle(s, prop::POWERED);
        self.set_block_and_update(pos, ns);
        self.attached_update_neighbours(ns, pos);
        let ev = if self.d.get_bool(ns, prop::POWERED) { crate::vibration::GameEvent::BlockActivate } else { crate::vibration::GameEvent::BlockDeactivate };
        self.game_event(ev, pos, None);
    }

    fn button_ticks(&self, s: State) -> i32 {
        self.d.block_field_i64(self.d.block_of(s), "ButtonBlock.ticksToStayPressed").unwrap_or(20) as i32
    }

    /// `ButtonBlock.press`
    pub(crate) fn button_press(&mut self, s: State, pos: BlockPos) {
        let ns = self.d.with_bool(s, prop::POWERED, true);
        self.set_block_and_update(pos, ns);
        self.attached_update_neighbours(s, pos);
        let b = self.d.block_of(s);
        let t = self.button_ticks(s);
        self.schedule_tick(pos, b, t, 0);
        self.game_event(crate::vibration::GameEvent::BlockActivate, pos, None);
    }

    /// `ButtonBlock.tick` -> `checkPressed` (arrows are not simulated)
    pub(crate) fn button_tick(&mut self, s: State, pos: BlockPos) {
        if self.d.get_bool(s, prop::POWERED) {
            let ns = self.d.with_bool(s, prop::POWERED, false);
            self.set_block_and_update(pos, ns);
            self.attached_update_neighbours(s, pos);
            self.game_event(crate::vibration::GameEvent::BlockDeactivate, pos, None);
        }
    }

    /// Returns true if nothing changed (lit already matches the power).
    pub(crate) fn lamp_neighbor_changed(&mut self, s: State, pos: BlockPos) -> bool {
        let lit = self.d.get_bool(s, prop::LIT);
        if lit == self.has_neighbor_signal(pos) {
            return true;
        }
        if lit {
            self.schedule_tick(pos, blk::REDSTONE_LAMP, 4, 0);
        } else {
            let ns = self.d.cycle(s, prop::LIT);
            self.set_block(pos, ns, 2);
        }
        false
    }

    /// A player right-clicking `pos` with an empty hand (creative).
    /// Returns false if the block has no use action that the simulator models.
    pub fn use_block(&mut self, pos: BlockPos) -> bool {
        let s = self.get(pos);
        match self.kind(s) {
            Kind::Lever => {
                self.lever_pull(s, pos);
                true
            }
            Kind::Button => {
                if !self.d.get_bool(s, prop::POWERED) {
                    self.button_press(s, pos);
                }
                true
            }
            Kind::Repeater | Kind::Comparator => self.diode_use(s, pos),
            Kind::Wire => self.wire_use(s, pos),
            Kind::Door => self.door_use(s, pos),
            Kind::Trapdoor => self.trapdoor_use(s, pos),
            Kind::FenceGate => self.fence_gate_use(s, pos),
            Kind::NoteBlock => self.note_use(s, pos),
            Kind::DaylightDetector => {
                // DaylightDetectorBlock.useWithoutItem: flip inverted (flags 2), then re-read light
                let ns = self.d.cycle(s, prop::INVERTED);
                self.set_block(pos, ns, 2);
                self.game_event(crate::vibration::GameEvent::BlockChange, pos, Some(ns));
                self.daylight_update(ns, pos);
                true
            }
            _ => false,
        }
    }
}
