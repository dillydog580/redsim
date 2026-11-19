//! Sky light, as the game's light engine settles it (`ChunkSkyLightSources` + `SkyLightEngine`),
//! and the overworld day clock. Only daylight detectors read light.
//!
//! The game applies light updates on a background thread a moment after a block changes, so a
//! reading taken within about a tick of a nearby change can come out either way in the real game.
//! redsim uses the settled value and warns when a daylight detector reads light that recently
//! changed nearby.

use crate::data::State;
use crate::pos::{BlockPos, Dir};
use crate::world::{World, MAX_Y, MIN_Y};
use rustc_hash::FxHashMap;

impl World {
    #[inline]
    fn face_occ(&self, s: State, d: Dir) -> u8 {
        self.d.face_occ[s as usize][d.ordinal() as usize]
    }

    #[inline]
    fn faces_occlude(&self, a: u8, b: u8) -> bool {
        (self.d.face_occludes[a as usize] >> b) & 1 != 0
    }

    #[inline]
    pub(crate) fn light_dampening(&self, s: State) -> i32 {
        self.d.light_dampening[s as usize] as i32
    }

    /// True if the two states' light-relevant properties differ (`LightEngine.hasDifferentLightProperties`).
    pub(crate) fn light_props_differ(&self, old: State, new: State) -> bool {
        old != new
            && (self.d.light_dampening[old as usize] != self.d.light_dampening[new as usize]
                || self.d.info(old).light != self.d.info(new).light
                || self.d.face_occ[old as usize] != [0; 6]
                || self.d.face_occ[new as usize] != [0; 6])
    }

    /// `ChunkSkyLightSources.isEdgeOccluded`
    fn edge_occluded(&self, top: State, bottom: State) -> bool {
        if self.light_dampening(bottom) != 0 {
            return true;
        }
        self.faces_occlude(self.face_occ(top, Dir::Down), self.face_occ(bottom, Dir::Up))
    }

    /// Lowest y of the column whose position (and everything above) receives full sky light;
    /// `i32::MIN` when the whole column is open.
    pub fn lowest_sky_source_y(&self, x: i32, z: i32) -> i32 {
        let Some((lo, hi)) = self.storage().y_range() else { return i32::MIN };
        let mut top = crate::data::AIR;
        let mut y = hi.min(MAX_Y);
        let lo = lo.max(MIN_Y);
        while y >= lo {
            let b = self.get(BlockPos::new(x, y, z));
            if self.edge_occluded(top, b) {
                return y + 1;
            }
            top = b;
            y -= 1;
        }
        i32::MIN
    }

    /// Settled sky light level at pos (`getBrightness(LightLayer.SKY, pos)`).
    pub fn sky_light(&self, pos: BlockPos) -> i32 {
        let mut columns: FxHashMap<(i32, i32), i32> = FxHashMap::default();
        let mut is_source = |w: &World, p: BlockPos| -> bool {
            let lowest = *columns.entry((p.x, p.z)).or_insert_with(|| w.lowest_sky_source_y(p.x, p.z));
            p.y >= lowest
        };
        if pos.y > MAX_Y || is_source(self, pos) {
            return 15;
        }
        let sp = self.get(pos);
        let first = self.light_dampening(sp).max(1);
        if first >= 15 {
            return 0;
        }
        // Dijkstra backwards from pos: dist(x) = light lost between x and pos (x included).
        let mut dist: FxHashMap<BlockPos, i32> = FxHashMap::default();
        let mut buckets: Vec<Vec<BlockPos>> = vec![Vec::new(); 16];
        dist.insert(pos, first);
        buckets[first as usize].push(pos);
        let mut best = 0;
        for c in 1..15 {
            if 15 - c <= best {
                break;
            }
            let list = std::mem::take(&mut buckets[c as usize]);
            for x in list {
                if dist.get(&x).copied() != Some(c) {
                    continue;
                }
                let sx = self.get(x);
                for d in Dir::ALL {
                    let y = x.rel(d);
                    if y.y < MIN_Y {
                        continue;
                    }
                    let sy = self.get(y);
                    // light flows from y into x (direction d.opposite())
                    if self.faces_occlude(self.face_occ(sy, d.opposite()), self.face_occ(sx, d)) {
                        continue;
                    }
                    if is_source(self, y) {
                        best = best.max(15 - c);
                        continue;
                    }
                    let cy = c + self.light_dampening(sy).max(1);
                    if cy >= 15 {
                        continue;
                    }
                    if dist.get(&y).map_or(true, |&o| cy < o) {
                        dist.insert(y, cy);
                        buckets[cy as usize].push(y);
                    }
                }
            }
        }
        best
    }

    /// (cos term, sky darkening) of the daylight detector formula at the current day time.
    pub(crate) fn day_factors(&self) -> (f32, i32) {
        let t = self.day_time.rem_euclid(24000) as usize;
        match self.d.daylight.get(t) {
            Some(&(c, dk)) => (c, dk as i32),
            None => (1.0, 0),
        }
    }
}

/// `Math.round(float)`: nearest integer, ties toward positive infinity, computed exactly.
pub(crate) fn java_round(x: f32) -> i32 {
    ((x as f64) + 0.5).floor() as i32
}
