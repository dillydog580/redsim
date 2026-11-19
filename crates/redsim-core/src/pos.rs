//! Block positions and directions, mirroring `net.minecraft.core.BlockPos` / `Direction`.

use serde::Serialize;
use std::fmt;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        BlockPos { x, y, z }
    }
    #[inline]
    pub fn rel(self, d: Dir) -> Self {
        let (dx, dy, dz) = d.step();
        BlockPos::new(self.x + dx, self.y + dy, self.z + dz)
    }
    #[inline]
    pub fn rel_n(self, d: Dir, n: i32) -> Self {
        let (dx, dy, dz) = d.step();
        BlockPos::new(self.x + dx * n, self.y + dy * n, self.z + dz * n)
    }
    #[inline]
    pub fn above(self) -> Self {
        BlockPos::new(self.x, self.y + 1, self.z)
    }
    #[inline]
    pub fn below(self) -> Self {
        BlockPos::new(self.x, self.y - 1, self.z)
    }
    #[inline]
    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        BlockPos::new(self.x + dx, self.y + dy, self.z + dz)
    }
    /// `Vec3i.hashCode()`: drives Java HashSet/HashMap iteration order.
    #[inline]
    pub fn java_hash(self) -> i32 {
        (self.y.wrapping_add(self.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(self.x)
    }
    /// `ChunkPos.pack(pos)`.
    #[inline]
    pub fn chunk_key(self) -> i64 {
        let cx = (self.x >> 4) as i64;
        let cz = (self.z >> 4) as i64;
        (cx & 0xffff_ffff) | ((cz & 0xffff_ffff) << 32)
    }
}

impl fmt::Debug for BlockPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}
impl fmt::Display for BlockPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.x, self.y, self.z)
    }
}

/// Directions with the game's ordinal order (DOWN, UP, NORTH, SOUTH, WEST, EAST).
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Down = 0,
    Up = 1,
    North = 2,
    South = 3,
    West = 4,
    East = 5,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Dir {
    /// `Direction.values()`
    pub const ALL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];
    /// Directions indexed by `ordinal()`.
    pub const ALL_ORDINAL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];
    /// `Direction.Plane.HORIZONTAL`
    pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];
    /// `Direction.Plane.VERTICAL`
    pub const VERTICAL: [Dir; 2] = [Dir::Up, Dir::Down];
    /// `NeighborUpdater.UPDATE_ORDER`
    pub const UPDATE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::Down, Dir::Up, Dir::North, Dir::South];
    /// `BlockBehaviour.UPDATE_SHAPE_ORDER`
    pub const SHAPE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::North, Dir::South, Dir::Down, Dir::Up];

    #[inline]
    pub fn from_ordinal(i: u8) -> Dir {
        Dir::ALL[i as usize]
    }
    /// `Direction.from3DDataValue` (same as ordinal, modulo 6).
    #[inline]
    pub fn from_3d(i: i32) -> Dir {
        Dir::ALL[(i.rem_euclid(6)) as usize]
    }
    #[inline]
    pub fn ordinal(self) -> u8 {
        self as u8
    }
    #[inline]
    pub fn step(self) -> (i32, i32, i32) {
        match self {
            Dir::Down => (0, -1, 0),
            Dir::Up => (0, 1, 0),
            Dir::North => (0, 0, -1),
            Dir::South => (0, 0, 1),
            Dir::West => (-1, 0, 0),
            Dir::East => (1, 0, 0),
        }
    }
    #[inline]
    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }
    #[inline]
    pub fn axis(self) -> Axis {
        match self {
            Dir::Down | Dir::Up => Axis::Y,
            Dir::North | Dir::South => Axis::Z,
            Dir::West | Dir::East => Axis::X,
        }
    }
    #[inline]
    pub fn is_horizontal(self) -> bool {
        self.axis() != Axis::Y
    }
    /// `Direction.getClockWise()` (around Y). Panics for vertical directions like the game.
    pub fn cw(self) -> Dir {
        match self {
            Dir::North => Dir::East,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            Dir::East => Dir::South,
            _ => panic!("Unable to get Y-rotated facing of {:?}", self),
        }
    }
    /// `Direction.getCounterClockWise()`
    pub fn ccw(self) -> Dir {
        match self {
            Dir::North => Dir::West,
            Dir::South => Dir::East,
            Dir::West => Dir::South,
            Dir::East => Dir::North,
            _ => panic!("Unable to get CCW facing of {:?}", self),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::North => "north",
            Dir::South => "south",
            Dir::West => "west",
            Dir::East => "east",
        }
    }
    pub fn from_name(s: &str) -> Option<Dir> {
        Some(match s {
            "down" => Dir::Down,
            "up" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }
    /// Axis direction is positive (UP, SOUTH, EAST).
    pub fn positive(self) -> bool {
        matches!(self, Dir::Up | Dir::South | Dir::East)
    }
}
