//! Shape-update driven connections: walls, fences, iron bars/glass panes, stairs.
//! Observers see these state changes, so they matter for redstone.

use crate::data::{Kind, State};
use crate::gen::{blk, prop, tag};
use crate::pos::{BlockPos, Dir};
use crate::shape::{face_covers, Rect, Support};
use crate::world::World;

const S: f64 = 1.0 / 16.0;

fn wall_prop(d: Dir) -> u16 {
    match d {
        Dir::North => prop::NORTH_WALL,
        Dir::East => prop::EAST_WALL,
        Dir::South => prop::SOUTH_WALL,
        _ => prop::WEST_WALL,
    }
}

fn side_bool_prop(d: Dir) -> u16 {
    match d {
        Dir::North => prop::NORTH,
        Dir::East => prop::EAST,
        Dir::South => prop::SOUTH,
        _ => prop::WEST,
    }
}

/// TEST_SHAPES_WALL projected on the XZ plane: box(7,0,0, 9,16,9) rotated per side.
fn wall_test_rect(d: Dir) -> Rect {
    match d {
        Dir::North => [7.0 * S, 0.0, 9.0 * S, 9.0 * S],
        Dir::East => [7.0 * S, 7.0 * S, 1.0, 9.0 * S],
        Dir::South => [7.0 * S, 7.0 * S, 9.0 * S, 1.0],
        _ => [0.0, 7.0 * S, 9.0 * S, 9.0 * S],
    }
}
const POST_RECT: Rect = [7.0 * S, 7.0 * S, 9.0 * S, 9.0 * S];

impl World {
    /// `Block.isExceptionForConnection`
    pub(crate) fn is_exception_for_connection(&self, s: State) -> bool {
        let b = self.d.block_of(s);
        self.d.bflags(b) & crate::data::bflag::LEAVES != 0
            || b == blk::BARRIER
            || b == blk::CARVED_PUMPKIN
            || b == blk::JACK_O_LANTERN
            || b == blk::MELON
            || b == blk::PUMPKIN
            || self.d.has_tag(b, tag::SHULKER_BOXES)
    }

    fn gate_connects_to_direction(&self, s: State, d: Dir) -> bool {
        self.kind(s) == Kind::FenceGate && self.d.get_dir(s, prop::HORIZONTAL_FACING).axis() == d.cw().axis()
    }

    fn is_bars(&self, s: State) -> bool {
        self.kind(s) == Kind::Bars
    }

    // ----- walls -----------------------------------------------------------------

    fn wall_connects_to(&self, s: State, face_solid: bool, d: Dir) -> bool {
        self.d.has_tag(self.d.block_of(s), tag::WALLS)
            || (!self.is_exception_for_connection(s) && face_solid)
            || self.is_bars(s)
            || self.gate_connects_to_direction(s, d)
    }

    fn wall_side_connected(&self, s: State, d: Dir) -> bool {
        self.d.get_str(s, wall_prop(d)) != "none"
    }

    pub(crate) fn wall_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        self.warn_waterlogged_pub(s, pos);
        if dir == Dir::Down {
            return s;
        }
        if dir == Dir::Up {
            let c = [Dir::North, Dir::East, Dir::South, Dir::West].map(|d| self.wall_side_connected(s, d));
            return self.wall_shape(s, npos, ns, c);
        }
        let mut c = [false; 4];
        for (i, d) in [Dir::North, Dir::East, Dir::South, Dir::West].into_iter().enumerate() {
            c[i] = if d == dir {
                let opp = dir.opposite();
                let solid = self.sturdy(ns, npos, opp, Support::Full);
                self.wall_connects_to(ns, solid, opp)
            } else {
                self.wall_side_connected(s, d)
            };
        }
        let above = pos.above();
        let a = self.get(above);
        self.wall_shape(s, above, a, c)
    }

    fn wall_shape(&self, s: State, top_pos: BlockPos, top: State, c: [bool; 4]) -> State {
        let above = self.collision_boxes(top, top_pos);
        let mut ns = s;
        for (i, d) in [Dir::North, Dir::East, Dir::South, Dir::West].into_iter().enumerate() {
            let v = if c[i] {
                if face_covers(&above, Dir::Down, &[wall_test_rect(d)]) {
                    "tall"
                } else {
                    "low"
                }
            } else {
                "none"
            };
            ns = self.d.with_str(ns, wall_prop(d), v);
        }
        let post = self.wall_should_raise_post(ns, top, &above);
        self.d.with_bool(ns, prop::UP, post)
    }

    fn wall_should_raise_post(&self, s: State, top: State, above: &[crate::shape::Aabb]) -> bool {
        if self.kind(top) == Kind::Wall && self.d.get_bool(top, prop::UP) {
            return true;
        }
        let n = self.d.get_str(s, prop::NORTH_WALL);
        let so = self.d.get_str(s, prop::SOUTH_WALL);
        let e = self.d.get_str(s, prop::EAST_WALL);
        let w = self.d.get_str(s, prop::WEST_WALL);
        let (sn, wn, en, nn) = (so == "none", w == "none", e == "none", n == "none");
        let corner = (nn && sn && wn && en) || nn != sn || wn != en;
        if corner {
            return true;
        }
        let high = (n == "tall" && so == "tall") || (e == "tall" && w == "tall");
        if high {
            false
        } else {
            self.d.has_tag(self.d.block_of(top), tag::WALL_POST_OVERRIDE) || face_covers(above, Dir::Down, &[POST_RECT])
        }
    }

    // ----- fences and bars ------------------------------------------------------------

    pub(crate) fn fence_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        self.warn_waterlogged_pub(s, pos);
        if !dir.is_horizontal() {
            return s;
        }
        let opp = dir.opposite();
        let solid = self.sturdy(ns, npos, opp, Support::Full);
        let nb = self.d.block_of(ns);
        let same_fence = self.d.has_tag(nb, tag::FENCES)
            && self.d.has_tag(nb, tag::WOODEN_FENCES) == self.d.has_tag(self.d.block_of(s), tag::WOODEN_FENCES);
        let gate = self.gate_connects_to_direction(ns, opp);
        let v = (!self.is_exception_for_connection(ns) && solid) || same_fence || gate;
        self.d.with_bool(s, side_bool_prop(dir), v)
    }

    pub(crate) fn bars_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir, npos: BlockPos, ns: State) -> State {
        self.warn_waterlogged_pub(s, pos);
        if !dir.is_horizontal() {
            return s;
        }
        let solid = self.sturdy(ns, npos, dir.opposite(), Support::Full);
        let v = (!self.is_exception_for_connection(ns) && solid) || self.is_bars(ns) || self.d.has_tag(self.d.block_of(ns), tag::WALLS);
        self.d.with_bool(s, side_bool_prop(dir), v)
    }

    // ----- stairs -------------------------------------------------------------------

    fn is_stairs(&self, s: State) -> bool {
        self.kind(s) == Kind::Stairs
    }

    fn stairs_can_take_shape(&self, s: State, pos: BlockPos, d: Dir) -> bool {
        let n = self.get(pos.rel(d));
        !self.is_stairs(n)
            || self.d.get_dir(n, prop::HORIZONTAL_FACING) != self.d.get_dir(s, prop::HORIZONTAL_FACING)
            || self.d.get_str(n, prop::HALF) != self.d.get_str(s, prop::HALF)
    }

    fn stairs_shape(&self, s: State, pos: BlockPos) -> &'static str {
        let facing = self.d.get_dir(s, prop::HORIZONTAL_FACING);
        let half = self.d.get_str(s, prop::HALF);
        let behind = self.get(pos.rel(facing));
        if self.is_stairs(behind) && half == self.d.get_str(behind, prop::HALF) {
            let bf = self.d.get_dir(behind, prop::HORIZONTAL_FACING);
            if bf.axis() != facing.axis() && self.stairs_can_take_shape(s, pos, bf.opposite()) {
                return if bf == facing.ccw() { "outer_left" } else { "outer_right" };
            }
        }
        let front = self.get(pos.rel(facing.opposite()));
        if self.is_stairs(front) && half == self.d.get_str(front, prop::HALF) {
            let ff = self.d.get_dir(front, prop::HORIZONTAL_FACING);
            if ff.axis() != facing.axis() && self.stairs_can_take_shape(s, pos, ff) {
                return if ff == facing.ccw() { "inner_left" } else { "inner_right" };
            }
        }
        "straight"
    }

    pub(crate) fn stairs_update_shape(&mut self, s: State, pos: BlockPos, dir: Dir) -> State {
        self.warn_waterlogged_pub(s, pos);
        if dir.is_horizontal() {
            let shape = self.stairs_shape(s, pos);
            self.d.with_str(s, prop::STAIRS_SHAPE, shape)
        } else {
            s
        }
    }
}
