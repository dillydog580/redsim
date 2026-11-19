//! redsim-core: a tick-exact simulator of Minecraft Java Edition 26.3 redstone.
//!
//! The engine is a faithful port of the game's own update machinery (setBlock,
//! neighbour/shape update queue, scheduled ticks, block events, block entities)
//! and of each redstone component, validated tick-by-tick against the real game.

pub mod blocks;
mod blocktable;
pub mod data;
pub mod gen;
pub mod javahash;
pub mod light;
pub mod pos;
pub mod shape;
pub mod ticks;
pub mod trace;
pub mod vibration;
pub mod world;

pub use data::{data, Kind, State};
pub use pos::{BlockPos, Dir};
pub use world::World;
