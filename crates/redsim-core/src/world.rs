//! The simulated level: block storage, `setBlock`, neighbour/shape update queue,
//! scheduled ticks, block events, block entities and the per-tick phase loop.
//!
//! Every routine here is a port of the corresponding game method in 26.3
//! (`Level`, `ServerLevel`, `LevelChunk`, `CollectingNeighborUpdater`), keeping the
//! exact order of operations, because redstone timing depends on it.

use crate::data::{data, flag, BlockData, BlockId, Kind, State, AIR};
use crate::gen::{blk, prop, st};
use crate::pos::{BlockPos, Dir};
use crate::ticks::{LevelTicks, ScheduledTick};
use crate::shape::{face_supports, Aabb, Support};
use crate::trace::{Cause, Phase, Trace, TraceKind};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 319;
pub const MAX_CHAINED_NEIGHBOR_UPDATES: u32 = 1_000_000;

const NOOP_MEMO_SIZE: usize = 2048;

#[inline]
fn noop_slot(p: BlockPos) -> usize {
    let h = (p.x as u32).wrapping_mul(0x9E37_79B1) ^ (p.y as u32).wrapping_mul(0x85EB_CA77) ^ (p.z as u32).wrapping_mul(0xC2B2_AE3D);
    (h >> 21) as usize
}

pub mod flags {
    pub const UPDATE_NEIGHBORS: i32 = 1;
    pub const UPDATE_CLIENTS: i32 = 2;
    pub const UPDATE_INVISIBLE: i32 = 4;
    pub const UPDATE_IMMEDIATE: i32 = 8;
    pub const UPDATE_KNOWN_SHAPE: i32 = 16;
    pub const UPDATE_SUPPRESS_DROPS: i32 = 32;
    pub const UPDATE_MOVE_BY_PISTON: i32 = 64;
    pub const UPDATE_SKIP_SHAPE_UPDATE_ON_WIRE: i32 = 128;
    pub const UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS: i32 = 256;
    pub const UPDATE_SKIP_ON_PLACE: i32 = 512;
    pub const UPDATE_ALL: i32 = 3;
    pub const UPDATE_SKIP_ALL_SIDEEFFECTS: i32 = 816;
    pub const UPDATE_LIMIT: i32 = 512;
}

// ---------------------------------------------------------------------------------
// Storage

/// Dense block storage over a growable bounding box; everything outside is air.
#[derive(Clone)]
pub struct Storage {
    min: (i32, i32, i32),
    dim: (i32, i32, i32),
    data: Vec<State>,
}

impl Storage {
    pub fn new() -> Self {
        Storage { min: (0, 0, 0), dim: (0, 0, 0), data: Vec::new() }
    }
    /// (lowest, highest) y that may hold a non-air block.
    pub fn y_range(&self) -> Option<(i32, i32)> {
        if self.dim.1 == 0 {
            None
        } else {
            Some((self.min.1, self.min.1 + self.dim.1 - 1))
        }
    }
    #[inline]
    fn index(&self, p: BlockPos) -> Option<usize> {
        let x = p.x.wrapping_sub(self.min.0);
        let y = p.y.wrapping_sub(self.min.1);
        let z = p.z.wrapping_sub(self.min.2);
        if (x as u32) < self.dim.0 as u32 && (y as u32) < self.dim.1 as u32 && (z as u32) < self.dim.2 as u32 {
            Some(((y * self.dim.2 + z) * self.dim.0 + x) as usize)
        } else {
            None
        }
    }
    /// The stored state, or None outside the stored box (everything there is air).
    #[inline]
    pub fn get_stored(&self, p: BlockPos) -> Option<State> {
        self.index(p).map(|i| unsafe { *self.data.get_unchecked(i) })
    }
    #[inline]
    pub fn get(&self, p: BlockPos) -> State {
        match self.index(p) {
            Some(i) => unsafe { *self.data.get_unchecked(i) },
            None => AIR,
        }
    }
    #[inline]
    pub fn set(&mut self, p: BlockPos, s: State) {
        if let Some(i) = self.index(p) {
            self.data[i] = s;
            return;
        }
        if s == AIR {
            return;
        }
        self.grow_to(p);
        let i = self.index(p).unwrap();
        self.data[i] = s;
    }
    fn grow_to(&mut self, p: BlockPos) {
        const PAD: i32 = 8;
        let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1);
        if self.data.is_empty() {
            x0 = p.x - PAD;
            y0 = p.y - PAD;
            z0 = p.z - PAD;
            x1 = p.x + PAD;
            y1 = p.y + PAD;
            z1 = p.z + PAD;
        } else {
            x0 = self.min.0;
            y0 = self.min.1;
            z0 = self.min.2;
            x1 = x0 + self.dim.0 - 1;
            y1 = y0 + self.dim.1 - 1;
            z1 = z0 + self.dim.2 - 1;
            if p.x < x0 {
                x0 = p.x - PAD.max(self.dim.0 / 2);
            }
            if p.y < y0 {
                y0 = p.y - PAD.max(self.dim.1 / 2);
            }
            if p.z < z0 {
                z0 = p.z - PAD.max(self.dim.2 / 2);
            }
            if p.x > x1 {
                x1 = p.x + PAD.max(self.dim.0 / 2);
            }
            if p.y > y1 {
                y1 = p.y + PAD.max(self.dim.1 / 2);
            }
            if p.z > z1 {
                z1 = p.z + PAD.max(self.dim.2 / 2);
            }
        }
        // the box never leaves the world's height range (World::get relies on it)
        y0 = y0.max(MIN_Y);
        y1 = y1.min(MAX_Y);
        let nd = (x1 - x0 + 1, y1 - y0 + 1, z1 - z0 + 1);
        let mut nd_data = vec![AIR; (nd.0 as usize) * (nd.1 as usize) * (nd.2 as usize)];
        if !self.data.is_empty() {
            for y in 0..self.dim.1 {
                for z in 0..self.dim.2 {
                    let src = ((y * self.dim.2 + z) * self.dim.0) as usize;
                    let ny = y + self.min.1 - y0;
                    let nz = z + self.min.2 - z0;
                    let nx = self.min.0 - x0;
                    let dst = ((ny * nd.2 + nz) * nd.0 + nx) as usize;
                    nd_data[dst..dst + self.dim.0 as usize].copy_from_slice(&self.data[src..src + self.dim.0 as usize]);
                }
            }
        }
        self.min = (x0, y0, z0);
        self.dim = nd;
        self.data = nd_data;
    }
    /// Bounding box of storage (not necessarily tight).
    pub fn bounds(&self) -> Option<(BlockPos, BlockPos)> {
        if self.data.is_empty() {
            return None;
        }
        Some((
            BlockPos::new(self.min.0, self.min.1, self.min.2),
            BlockPos::new(self.min.0 + self.dim.0 - 1, self.min.1 + self.dim.1 - 1, self.min.2 + self.dim.2 - 1),
        ))
    }
    /// Iterate all non-air blocks.
    pub fn for_each_non_air(&self, mut f: impl FnMut(BlockPos, State)) {
        if self.data.is_empty() {
            return;
        }
        let mut i = 0usize;
        for y in 0..self.dim.1 {
            for z in 0..self.dim.2 {
                for x in 0..self.dim.0 {
                    let s = self.data[i];
                    if s != AIR {
                        f(BlockPos::new(self.min.0 + x, self.min.1 + y, self.min.2 + z), s);
                    }
                    i += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// Block entities

#[derive(Clone, Debug, PartialEq)]
pub struct MovingPiston {
    pub moved_state: State,
    pub direction: Dir,
    pub extending: bool,
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    pub slot: u32,
    pub id: String,
    pub count: i32,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HopperState {
    pub cooldown: i32,
    pub ticked: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerData {
    pub size: u32,
    pub max_stack: i32,
    pub items: Vec<ItemStack>,
    /// hoppers only: TransferCooldown and tickedGameTime
    pub hopper: Option<HopperState>,
    /// crafters only: bitmask of disabled slots
    pub disabled_slots: u32,
}

impl ContainerData {
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` (float arithmetic as in Java).
    pub fn signal(&self) -> i32 {
        let d = data();
        let mut total: f32 = 0.0;
        let mut items: Vec<&ItemStack> = self.items.iter().filter(|i| i.count > 0 && i.slot < self.size).collect();
        items.sort_by_key(|i| i.slot);
        for it in items {
            let item_max = *d.item_max_stack.get(&it.id).unwrap_or(&64);
            let max = self.max_stack.min(item_max).max(1);
            total += it.count as f32 / max as f32;
        }
        total /= self.size.max(1) as f32;
        (total * 14.0).floor() as i32 + if total > 0.0 { 1 } else { 0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BeData {
    Comparator { output: i32 },
    Moving(MovingPiston),
    Container(ContainerData),
    Lectern { page: i32, pages: i32 },
    Bookshelf { last_slot: i32 },
    Jukebox { item: Option<String>, ticks: i64, playing: bool },
    /// daylight detector (ticks every game tick, reads light every 20)
    Daylight,
    /// sculk sensor vibration listener
    Sculk(crate::vibration::SculkData),
    /// A block entity tracked for existence only (no redstone-relevant data).
    Generic,
}

/// Which ticker a block entity runs (from its data).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Ticker {
    Moving,
    Jukebox,
    Daylight,
    Sculk,
    Hopper,
    Container,
    Other,
}

impl Ticker {
    fn of(d: &BeData) -> Ticker {
        match d {
            BeData::Moving(_) => Ticker::Moving,
            BeData::Jukebox { .. } => Ticker::Jukebox,
            BeData::Daylight => Ticker::Daylight,
            BeData::Sculk(_) => Ticker::Sculk,
            BeData::Container(c) if c.hopper.is_some() => Ticker::Hopper,
            BeData::Container(_) => Ticker::Container,
            _ => Ticker::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockEntity {
    pub gen: u64,
    pub state: State,
    /// index into BlockData.be_types (the game's BlockEntityType)
    pub be_type: u16,
    pub data: BeData,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlockEvent {
    pub pos: BlockPos,
    pub block: BlockId,
    pub a: i32,
    pub b: i32,
}

#[derive(Copy, Clone, Debug)]
struct QueuedEvent {
    e: BlockEvent,
    cause: Cause,
}

// ---------------------------------------------------------------------------------
// Neighbour updater (CollectingNeighborUpdater)

#[derive(Copy, Clone, Debug)]
enum Upd {
    /// the neighbour that changed is always at `pos.rel(dir)` (every caller), so it is not stored
    Shape { dir: Dir, neighbor_state: State, pos: BlockPos, flags: i32, limit: i32, cause: Cause },
    Simple { pos: BlockPos, block: BlockId, cause: Cause },
    Full { state: State, pos: BlockPos, block: BlockId, moved: bool, cause: Cause },
    Multi { src: BlockPos, block: BlockId, skip: Option<Dir>, idx: u8, cause: Cause },
}

const _: () = assert!(std::mem::size_of::<Upd>() <= 40);

#[derive(Default, Clone)]
struct Updater {
    stack: Vec<Upd>,
    added: Vec<Upd>,
    count: u32,
}

/// One block-state write.
#[derive(Copy, Clone, Debug)]
pub struct Change {
    pub tick: i64,
    pub phase: Phase,
    pub pos: BlockPos,
    pub old: State,
    pub new: State,
}

/// Something the simulator could not reproduce exactly (e.g. unsupported block).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Warning {
    pub pos: BlockPos,
    pub message: String,
}

// ---------------------------------------------------------------------------------

#[derive(Clone)]
pub struct World {
    pub(crate) d: &'static BlockData,
    /// block states: written only by `set_block_limit` (it bumps `epoch`)
    storage: Storage,
    /// block entities: read with `bes()`, change only through `bes_mut()` (it bumps `epoch`)
    block_entities: FxHashMap<BlockPos, BlockEntity>,
    /// Counts changes to block states and block entities. Anything computed only from those
    /// (signals, survival, connections) is unchanged while this stays the same.
    pub(crate) epoch: u64,
    /// neighbour-update handlers whose last run changed nothing: (pos, state, epoch),
    /// direct-mapped by position (see `noop_hit`)
    noop_memo: Vec<(BlockPos, State, u64)>,
    be_gen: u64,
    /// level.blockEntityTickers (wrapper ids)
    tickers: Vec<usize>,
    /// spare buffer for `tick_block_entities`
    ticker_spare: Vec<usize>,
    pending_tickers: Vec<usize>,
    /// wrapper id -> bound ticker (pos, BE generation) or None for NULL_TICKER
    wrappers: Vec<Option<(BlockPos, u64)>>,
    free_wrappers: Vec<usize>,
    /// LevelChunk.tickersInLevel
    tickers_in_level: FxHashMap<BlockPos, usize>,
    ticking_block_entities: bool,
    pub(crate) block_ticks: LevelTicks,
    block_events: VecDeque<QueuedEvent>,
    block_event_set: FxHashSet<BlockEvent>,
    updater: Updater,
    pub game_time: i64,
    sub_tick_count: i64,
    pub(crate) handling_tick: bool,
    /// RedstoneWireBlock.shouldSignal (an instance field of the singleton wire block)
    pub(crate) wire_should_signal: bool,
    /// RedstoneTorchBlock.RECENT_TOGGLES for this level
    pub(crate) torch_toggles: Vec<(BlockPos, i64)>,
    /// primed TNT entities (position, game time of the explosion): explosions are not simulated
    pub(crate) primed_tnt: Vec<(BlockPos, i64)>,
    pub trace: Option<Trace>,
    /// Every block-state write (tick, phase, pos, old, new), when enabled.
    pub changes: Option<Vec<Change>>,
    pub(crate) cause: Cause,
    pub(crate) phase: Phase,
    pub warnings: FxHashSet<Warning>,
    /// overworld clock (`ClockInstance.totalTicks`): drives daylight detectors
    pub day_time: i64,
    /// the `advance_time` game rule: the day clock moves one tick per game tick
    pub advance_time: bool,
    /// recent light-relevant block changes (pos, game time), for daylight detector warnings
    pub(crate) light_changes: Vec<(BlockPos, i64)>,
    /// positions of block entities that listen for vibrations (sculk sensors)
    pub(crate) vibration_listeners: Vec<BlockPos>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        World {
            d: data(),
            storage: Storage::new(),
            block_entities: FxHashMap::default(),
            epoch: 1,
            noop_memo: Vec::new(),
            be_gen: 0,
            tickers: Vec::new(),
            ticker_spare: Vec::new(),
            pending_tickers: Vec::new(),
            wrappers: Vec::new(),
            free_wrappers: Vec::new(),
            tickers_in_level: FxHashMap::default(),
            ticking_block_entities: false,
            block_ticks: LevelTicks::new(),
            block_events: VecDeque::new(),
            block_event_set: FxHashSet::default(),
            updater: Updater::default(),
            game_time: 0,
            sub_tick_count: 0,
            handling_tick: false,
            wire_should_signal: true,
            torch_toggles: Vec::new(),
            primed_tnt: Vec::new(),
            trace: None,
            changes: None,
            cause: Cause::None,
            phase: Phase::Action,
            warnings: FxHashSet::default(),
            day_time: 0,
            advance_time: true,
            light_changes: Vec::new(),
            vibration_listeners: Vec::new(),
        }
    }

    // ----- basic access --------------------------------------------------------

    #[inline]
    pub(crate) fn bes(&self) -> &FxHashMap<BlockPos, BlockEntity> {
        &self.block_entities
    }

    /// True if the neighbour-update handler of state `s` at `pos` last ran with nothing in the
    /// world changed since (same `epoch`) and found nothing to do. Handlers record that only
    /// when their outcome depends on nothing but block states and block entities (dust power,
    /// piston and lamp power checks), so running them again would do nothing again: skipping
    /// them is exact. Most neighbour updates in busy redstone are such repeats.
    #[inline]
    pub(crate) fn noop_hit(&self, pos: BlockPos, s: State) -> bool {
        match self.noop_memo.get(noop_slot(pos)) {
            Some(&(p, st, ep)) => ep == self.epoch && p == pos && st == s,
            None => false,
        }
    }

    /// Records that the handler of `s` at `pos` just ran and changed nothing (see `noop_hit`).
    #[inline]
    pub(crate) fn noop_record(&mut self, pos: BlockPos, s: State) {
        if self.noop_memo.is_empty() {
            self.noop_memo = vec![(BlockPos::new(0, 0, 0), 0, 0); NOOP_MEMO_SIZE];
        }
        self.noop_memo[noop_slot(pos)] = (pos, s, self.epoch);
    }

    /// Mutable access to the block entities; counts as a world change (see `epoch`).
    #[inline]
    pub(crate) fn bes_mut(&mut self) -> &mut FxHashMap<BlockPos, BlockEntity> {
        self.epoch += 1;
        &mut self.block_entities
    }

    #[inline]
    pub fn in_valid_bounds(p: BlockPos) -> bool {
        p.y >= MIN_Y && p.y <= MAX_Y && p.x >= -30_000_000 && p.z >= -30_000_000 && p.x < 30_000_000 && p.z < 30_000_000
    }

    #[inline]
    pub fn get(&self, p: BlockPos) -> State {
        // the stored box lies within the height range, so a hit needs no range check
        match self.storage.get_stored(p) {
            Some(s) => s,
            None if p.y < MIN_Y || p.y > MAX_Y => st::VOID_AIR,
            None => AIR,
        }
    }

    #[inline]
    pub fn block(&self, p: BlockPos) -> BlockId {
        self.d.block_of(self.get(p))
    }

    #[inline]
    pub(crate) fn sflags(&self, s: State) -> u16 {
        self.d.flags(s)
    }
    #[inline]
    pub(crate) fn is_air(&self, s: State) -> bool {
        self.d.flags(s) & flag::AIR != 0
    }
    #[inline]
    pub(crate) fn is_conductor(&self, s: State) -> bool {
        self.d.flags(s) & flag::CONDUCTOR != 0
    }
    #[inline]
    pub(crate) fn has_be_flag(&self, s: State) -> bool {
        self.d.flags(s) & flag::HAS_BE != 0
    }
    #[inline]
    pub(crate) fn kind(&self, s: State) -> Kind {
        self.d.kind(s)
    }
    #[inline]
    pub(crate) fn is(&self, s: State, b: BlockId) -> bool {
        self.d.block_of(s) == b
    }
    /// `state.isFaceSturdy(level, pos, dir, supportType)`; moving blocks use their live shape.
    #[inline]
    pub fn sturdy(&self, s: State, pos: BlockPos, d: Dir, ty: Support) -> bool {
        if self.d.block_of(s) == blk::MOVING_PISTON {
            return self.moving_piston_sturdy(pos, d, ty);
        }
        let i = self.d.info(s);
        let m = match ty {
            Support::Full => i.sturdy_full,
            Support::Center => i.sturdy_center,
            Support::Rigid => i.sturdy_rigid,
        };
        m & (1 << d.ordinal()) != 0
    }
    #[inline]
    pub(crate) fn sturdy_full(&self, s: State, pos: BlockPos, d: Dir) -> bool {
        self.sturdy(s, pos, d, Support::Full)
    }
    #[inline]
    pub(crate) fn sturdy_center(&self, s: State, pos: BlockPos, d: Dir) -> bool {
        self.sturdy(s, pos, d, Support::Center)
    }
    #[inline]
    pub(crate) fn sturdy_rigid(&self, s: State, pos: BlockPos, d: Dir) -> bool {
        self.sturdy(s, pos, d, Support::Rigid)
    }

    /// `state.getCollisionShape(level, pos)` as boxes (live shape for moving blocks).
    pub fn collision_boxes(&self, s: State, pos: BlockPos) -> Vec<Aabb> {
        if self.d.block_of(s) == blk::MOVING_PISTON {
            self.moving_piston_boxes(pos)
        } else {
            self.d.collision(s).to_vec()
        }
    }

    fn moving_piston_sturdy(&self, pos: BlockPos, d: Dir, ty: Support) -> bool {
        face_supports(&self.moving_piston_boxes(pos), d, ty)
    }

    /// `PistonMovingBlockEntity.getCollisionShape`
    fn moving_piston_boxes(&self, pos: BlockPos) -> Vec<Aabb> {
        let Some(m) = self.moving_be(pos) else { return Vec::new() };
        let mut boxes: Vec<Aabb> = Vec::new();
        let base_kind = self.d.kind(m.moved_state) == Kind::Piston;
        if !m.extending && m.source && base_kind {
            let ext = self.d.with_bool(m.moved_state, prop::EXTENDED, true);
            boxes.extend_from_slice(self.d.collision(ext));
        }
        let shown = if m.source {
            let mut h = self.d.with_dir(st::PISTON_HEAD, prop::FACING, m.direction);
            let short = m.extending != (1.0 - m.progress < 0.25);
            h = self.d.with_bool(h, prop::SHORT, short);
            h
        } else {
            m.moved_state
        };
        let ext = if m.extending { m.progress - 1.0 } else { 1.0 - m.progress } as f64;
        let (dx, dy, dz) = m.direction.step();
        let off = [dx as f64 * ext, dy as f64 * ext, dz as f64 * ext];
        for b in self.d.collision(shown) {
            boxes.push(b.moved(off));
        }
        boxes
    }

    pub fn warn(&mut self, pos: BlockPos, message: impl Into<String>) {
        self.warnings.insert(Warning { pos, message: message.into() });
    }

    // ----- setBlock ------------------------------------------------------------

    pub fn set_block(&mut self, pos: BlockPos, state: State, flags: i32) -> bool {
        self.set_block_limit(pos, state, flags, flags::UPDATE_LIMIT)
    }

    /// `Level.setBlock(pos, state, flags, updateLimit)` + `LevelChunk.setBlockState`.
    pub fn set_block_limit(&mut self, pos: BlockPos, new: State, fl: i32, limit: i32) -> bool {
        if !Self::in_valid_bounds(pos) {
            return false;
        }
        // ---- LevelChunk.setBlockState
        let old = self.storage.get(pos);
        if old == new {
            return false;
        }
        self.storage.set(pos, new);
        self.epoch += 1;
        if self.light_props_differ(old, new) {
            self.light_changes.push((pos, self.game_time));
        }
        if let Some(c) = &mut self.changes {
            c.push(Change { tick: self.game_time, phase: self.phase, pos, old, new });
        }
        let d = self.d;
        let new_block = d.block_of(new);
        let old_block = d.block_of(old);
        let block_changed = old_block != new_block;
        let moved = fl & flags::UPDATE_MOVE_BY_PISTON != 0;
        let side_effects = fl & flags::UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS == 0;
        self.trace_set(pos, old, new, fl);
        if block_changed && self.has_be_flag(old) && !self.should_changed_state_keep_be(new, old) {
            if side_effects && self.bes().contains_key(&pos) {
                self.pre_remove_side_effects(pos, old);
            }
            self.remove_block_entity(pos);
        }
        if (block_changed || self.is_rail_block(new_block)) && (fl & flags::UPDATE_NEIGHBORS != 0 || moved) {
            self.affect_neighbors_after_removal(old, pos, moved);
        }
        if d.block_of(self.storage.get(pos)) != new_block {
            return false;
        }
        if fl & flags::UPDATE_SKIP_ON_PLACE == 0 {
            self.on_place(new, pos, old, moved);
        }
        if self.has_be_flag(new) && d.block_of(self.storage.get(pos)) == new_block {
            let valid = match self.bes().get(&pos) {
                Some(be) => Some(self.be_type_valid(be, new)),
                None => None,
            };
            match valid {
                Some(false) => {
                    self.remove_block_entity(pos);
                    self.create_block_entity(pos, new);
                }
                None => {
                    self.create_block_entity(pos, new);
                }
                Some(true) => {
                    if let Some(be) = self.bes_mut().get_mut(&pos) {
                        be.state = new;
                    }
                    self.update_block_entity_ticker(pos);
                }
            }
        }
        // ---- back in Level.setBlock
        let now = self.storage.get(pos);
        if now == new {
            if fl & flags::UPDATE_NEIGHBORS != 0 {
                self.update_neighbors_at(pos, old_block);
                if d.flags(new) & flag::ANALOG != 0 {
                    self.update_neighbour_for_output_signal(pos, new_block);
                }
            }
            if fl & flags::UPDATE_KNOWN_SHAPE == 0 && limit > 0 {
                let f2 = fl & !33;
                self.update_indirect_neighbour_shapes(old, pos, f2, limit - 1);
                self.update_neighbour_shapes(new, pos, f2, limit - 1);
                self.update_indirect_neighbour_shapes(new, pos, f2, limit - 1);
            }
        }
        true
    }

    #[inline]
    pub fn set_block_and_update(&mut self, pos: BlockPos, state: State) -> bool {
        self.set_block(pos, state, 3)
    }

    /// `Level.removeBlock(pos, movedByPiston)`
    pub fn remove_block(&mut self, pos: BlockPos, moved: bool) -> bool {
        let fl = self.fluid_legacy_block(self.get(pos));
        self.set_block(pos, fl, 3 | if moved { 64 } else { 0 })
    }

    /// `Level.destroyBlock(pos, drop, null, updateLimit)` (drops are not simulated)
    pub fn destroy_block(&mut self, pos: BlockPos, _drop: bool, limit: i32) -> bool {
        let s = self.get(pos);
        if self.is_air(s) {
            return false;
        }
        let fl = self.fluid_legacy_block(s);
        let done = self.set_block_limit(pos, fl, 3, limit);
        if done {
            self.game_event(crate::vibration::GameEvent::BlockDestroy, pos, Some(s));
        }
        done
    }

    /// `FluidState.createLegacyBlock()` for the fluid in state `s`.
    pub(crate) fn fluid_legacy_block(&self, s: State) -> State {
        let f = self.d.info(s).fluid;
        if f == 0 {
            return AIR;
        }
        let name = &self.d.fluids[f as usize];
        // "minecraft:water:8" / "minecraft:flowing_water:3"
        let (fname, amount) = name.rsplit_once(':').unwrap();
        let amount: i32 = amount.parse().unwrap();
        let source = !fname.contains("flowing");
        let base = if fname.contains("lava") { st::LAVA } else { st::WATER };
        // LiquidBlock LEVEL: 0 for source, else 8 - amount (+8 if falling, not tracked here)
        let level = if source { 0 } else { (8 - amount).clamp(0, 15) };
        self.d.with_int(base, prop::LEVEL, level)
    }

    /// `Block.updateOrDestroy`
    pub(crate) fn update_or_destroy(&mut self, old: State, new: State, pos: BlockPos, fl: i32, limit: i32) {
        if new != old {
            if self.is_air(new) {
                self.destroy_block(pos, fl & 32 == 0, limit);
            } else {
                self.set_block_limit(pos, new, fl & !32, limit);
            }
        }
    }

    /// `Block.updateFromNeighbourShapes`
    pub(crate) fn update_from_neighbour_shapes(&mut self, state: State, pos: BlockPos) -> State {
        let mut s = state;
        for d in Dir::SHAPE_ORDER {
            let np = pos.rel(d);
            let ns = self.get(np);
            s = self.update_shape(s, pos, d, np, ns);
        }
        s
    }

    // ----- neighbour / shape updates -------------------------------------------

    /// `ServerLevel.updateNeighborsAt(pos, block)`
    #[inline]
    pub fn update_neighbors_at(&mut self, pos: BlockPos, block: BlockId) {
        let cause = self.cause;
        self.add_and_run(pos, Upd::Multi { src: pos, block, skip: None, idx: 0, cause });
    }
    /// `ServerLevel.updateNeighborsAtExceptFromFacing`
    #[inline]
    pub fn update_neighbors_at_except_from_facing(&mut self, pos: BlockPos, block: BlockId, skip: Dir) {
        let cause = self.cause;
        let idx = if Dir::UPDATE_ORDER[0] == skip { 1 } else { 0 };
        self.add_and_run(pos, Upd::Multi { src: pos, block, skip: Some(skip), idx, cause });
    }
    /// `level.neighborChanged(pos, block, orientation)`
    #[inline]
    pub fn neighbor_changed(&mut self, pos: BlockPos, block: BlockId) {
        let cause = self.cause;
        self.add_and_run(pos, Upd::Simple { pos, block, cause });
    }
    /// `level.neighborChanged(state, pos, block, orientation, movedByPiston)`
    #[inline]
    pub fn neighbor_changed_full(&mut self, state: State, pos: BlockPos, block: BlockId, moved: bool) {
        let cause = self.cause;
        self.add_and_run(pos, Upd::Full { state, pos, block, moved, cause });
    }
    /// `level.neighborShapeChanged(direction, pos, neighborPos, neighborState, flags, limit)`
    #[inline]
    pub fn neighbor_shape_changed(&mut self, dir: Dir, pos: BlockPos, neighbor_pos: BlockPos, neighbor_state: State, fl: i32, limit: i32) {
        let cause = self.cause;
        debug_assert_eq!(neighbor_pos, pos.rel(dir));
        let _ = neighbor_pos;
        self.add_and_run(pos, Upd::Shape { dir, neighbor_state, pos, flags: fl, limit, cause });
    }

    /// `BlockState.updateNeighbourShapes`
    pub fn update_neighbour_shapes(&mut self, state: State, pos: BlockPos, fl: i32, limit: i32) {
        for d in Dir::SHAPE_ORDER {
            self.neighbor_shape_changed(d.opposite(), pos.rel(d), pos, state, fl, limit);
        }
    }

    fn add_and_run(&mut self, _pos: BlockPos, u: Upd) {
        let running = self.updater.count > 0;
        let too_many = self.updater.count >= MAX_CHAINED_NEIGHBOR_UPDATES;
        self.updater.count += 1;
        if !too_many {
            if running {
                self.updater.added.push(u);
            } else {
                self.updater.stack.push(u);
            }
        } else if self.updater.count - 1 == MAX_CHAINED_NEIGHBOR_UPDATES {
            self.warn(_pos, "Too many chained neighbor updates. Skipping the rest.");
        }
        if !running {
            self.run_updates();
        }
    }

    fn run_updates(&mut self) {
        loop {
            if self.updater.stack.is_empty() && self.updater.added.is_empty() {
                break;
            }
            // the updates added by the last one go on top, first-added topmost
            let Updater { stack, added, .. } = &mut self.updater;
            stack.extend(added.drain(..).rev());
            let top = self.updater.stack.len() - 1;
            if !self.run_entry(top) {
                self.updater.stack.pop();
            }
        }
        self.updater.stack.clear();
        self.updater.added.clear();
        self.updater.count = 0;
    }

    /// Runs the stack entry at `top` until it is finished (returns false) or one of its updates
    /// queued new ones (returns true): those run first, as in `CollectingNeighborUpdater`.
    fn run_entry(&mut self, top: usize) -> bool {
        match self.updater.stack[top] {
            Upd::Full { state, pos, block, moved, cause } => {
                self.execute_update(state, pos, block, moved, cause, None);
                false
            }
            Upd::Simple { pos, block, cause } => {
                let state = self.get(pos);
                self.execute_update(state, pos, block, false, cause, None);
                false
            }
            Upd::Shape { dir, neighbor_state, pos, flags, limit, cause } => {
                self.execute_shape_update(dir, pos, pos.rel(dir), neighbor_state, flags, limit, cause);
                false
            }
            Upd::Multi { src, block, skip, idx, cause } => {
                let mut i = idx as usize;
                loop {
                    let np = src.rel(Dir::UPDATE_ORDER[i]);
                    let state = self.get(np);
                    self.execute_update(state, np, block, false, cause, Some(src));
                    i += 1;
                    if i < 6 && Some(Dir::UPDATE_ORDER[i]) == skip {
                        i += 1;
                    }
                    if i >= 6 {
                        return false;
                    }
                    if !self.updater.added.is_empty() {
                        if let Upd::Multi { idx, .. } = &mut self.updater.stack[top] {
                            *idx = i as u8;
                        }
                        return true;
                    }
                }
            }
        }
    }

    #[inline]
    fn execute_update(&mut self, state: State, pos: BlockPos, block: BlockId, moved: bool, cause: Cause, src: Option<BlockPos>) {
        // air and plain blocks ignore neighbour changes; only the trace would record it
        if self.trace.is_none() && matches!(self.kind(state), Kind::Air | Kind::Plain) {
            return;
        }
        let saved = self.cause;
        if self.trace.is_some() {
            self.cause = self.trace_event(TraceKind::NeighborUpdate { block, src, moved }, pos, cause);
        }
        self.handle_neighbor_changed(state, pos, block, moved);
        self.cause = saved;
    }

    fn execute_shape_update(&mut self, dir: Dir, pos: BlockPos, neighbor_pos: BlockPos, neighbor_state: State, fl: i32, limit: i32, cause: Cause) {
        let current = self.get(pos);
        // air and plain blocks keep their state on shape updates; only the trace would record it
        if self.trace.is_none() && matches!(self.kind(current), Kind::Air | Kind::Plain) {
            return;
        }
        if fl & 128 == 0 || !self.is(current, blk::REDSTONE_WIRE) {
            let saved = self.cause;
            if self.trace.is_some() {
                self.cause = self.trace_event(TraceKind::ShapeUpdate { dir, src: neighbor_pos }, pos, cause);
            }
            let new = self.update_shape(current, pos, dir, neighbor_pos, neighbor_state);
            self.update_or_destroy(current, new, pos, fl, limit);
            self.cause = saved;
        }
    }

    /// `Level.updateNeighbourForOutputSignal`
    pub fn update_neighbour_for_output_signal(&mut self, pos: BlockPos, changed: BlockId) {
        for d in Dir::HORIZONTAL {
            let mut rp = pos.rel(d);
            let s = self.get(rp);
            if self.is(s, blk::COMPARATOR) {
                self.neighbor_changed_full(s, rp, changed, false);
            } else if self.is_conductor(s) {
                rp = rp.rel(d);
                let s2 = self.get(rp);
                if self.is(s2, blk::COMPARATOR) {
                    self.neighbor_changed_full(s2, rp, changed, false);
                }
            }
        }
    }

    // ----- scheduled ticks ------------------------------------------------------

    pub fn schedule_tick(&mut self, pos: BlockPos, block: BlockId, delay: i32, priority: i8) {
        let sub = self.sub_tick_count;
        self.sub_tick_count += 1;
        let mut cause = 0u32;
        if self.trace.is_some() {
            let c = self.cause;
            if let Cause::Event(id) = self.trace_event(TraceKind::ScheduleTick { block, delay, priority }, pos, c) {
                cause = id + 1;
            }
        }
        let t = ScheduledTick { block, pos, trigger: self.game_time + delay as i64, priority, sub, cause };
        self.block_ticks.schedule(t);
    }
    #[inline]
    pub fn has_scheduled_tick(&self, pos: BlockPos, block: BlockId) -> bool {
        self.block_ticks.has_scheduled_tick(pos, block)
    }
    #[inline]
    pub fn will_tick_this_tick(&mut self, pos: BlockPos, block: BlockId) -> bool {
        self.block_ticks.will_tick_this_tick(pos, block)
    }

    // ----- block events ---------------------------------------------------------

    pub fn block_event(&mut self, pos: BlockPos, block: BlockId, a: i32, b: i32) {
        let e = BlockEvent { pos, block, a, b };
        if self.block_event_set.insert(e) {
            let mut cause = Cause::None;
            if self.trace.is_some() {
                let c = self.cause;
                cause = self.trace_event(TraceKind::QueueBlockEvent { block, a, b }, pos, c);
            }
            self.block_events.push_back(QueuedEvent { e, cause });
        }
    }

    fn run_block_events(&mut self) {
        self.phase = Phase::BlockEvents;
        while let Some(q) = self.block_events.pop_front() {
            let e = q.e;
            self.block_event_set.remove(&e);
            let state = self.get(e.pos);
            if self.is(state, e.block) {
                let saved = self.cause;
                self.cause = if self.trace.is_some() {
                    self.trace_event(TraceKind::RunBlockEvent { block: e.block, a: e.a, b: e.b }, e.pos, q.cause)
                } else {
                    Cause::None
                };
                self.trigger_event(state, e.pos, e.a, e.b);
                self.cause = saved;
            }
        }
    }

    // ----- block entities ---------------------------------------------------------

    fn be_valid_for(&self, be: &BeData, state: State) -> bool {
        match be {
            BeData::Comparator { .. } => self.is(state, blk::COMPARATOR),
            BeData::Moving(_) => self.is(state, blk::MOVING_PISTON),
            _ => self.has_be_flag(state),
        }
    }

    fn be_type_valid(&self, be: &BlockEntity, state: State) -> bool {
        self.be_valid_for(&be.data, state) && self.d.block_be_type[self.d.block_of(state) as usize] == be.be_type
    }

    fn should_changed_state_keep_be(&self, _new: State, _old: State) -> bool {
        false
    }

    pub(crate) fn is_rail_block(&self, b: BlockId) -> bool {
        self.d.bflags(b) & crate::data::bflag::RAIL != 0
    }

    /// `EntityBlock.newBlockEntity` + `addAndRegisterBlockEntity`.
    fn create_block_entity(&mut self, pos: BlockPos, state: State) {
        let b = self.d.binfo(self.d.block_of(state));
        let data = match self.kind(state) {
            Kind::Comparator => BeData::Comparator { output: 0 },
            Kind::MovingPiston => return, // MovingPistonBlock.newBlockEntity returns null
            Kind::Lectern => BeData::Lectern { page: 0, pages: 0 },
            Kind::ChiseledBookshelf => BeData::Bookshelf { last_slot: -1 },
            Kind::Jukebox => BeData::Jukebox { item: None, ticks: 0, playing: false },
            Kind::DaylightDetector => BeData::Daylight,
            Kind::SculkSensor => BeData::Sculk(Default::default()),
            _ => match b.fields.get("container_size").and_then(|v| v.as_u64()) {
                Some(size) => BeData::Container(ContainerData {
                    size: size as u32,
                    max_stack: b.fields.get("container_max_stack").and_then(|v| v.as_i64()).unwrap_or(99) as i32,
                    items: Vec::new(),
                    hopper: if self.kind(state) == Kind::Hopper { Some(HopperState { cooldown: -1, ticked: 0 }) } else { None },
                    disabled_slots: 0,
                }),
                None => BeData::Generic,
            },
        };
        self.add_block_entity(pos, state, data);
    }

    /// `Level.setBlockEntity` -> `LevelChunk.addAndRegisterBlockEntity`.
    pub(crate) fn add_block_entity(&mut self, pos: BlockPos, state_hint: State, data: BeData) {
        if !Self::in_valid_bounds(pos) {
            return;
        }
        let cur = self.get(pos);
        if !self.has_be_flag(cur) {
            return;
        }
        if !self.be_valid_for(&data, cur) {
            return;
        }
        let be_type = match &data {
            BeData::Moving(_) => self.d.block_be_type[self.d.block_of(state_hint) as usize],
            _ => self.d.block_be_type[self.d.block_of(cur) as usize],
        };
        self.be_gen += 1;
        let gen = self.be_gen;
        let listens = matches!(data, BeData::Sculk(_));
        self.bes_mut().insert(pos, BlockEntity { gen, state: cur, be_type, data });
        // LevelChunk.addGameEventListener
        if listens && !self.vibration_listeners.contains(&pos) {
            self.vibration_listeners.push(pos);
        }
        self.update_block_entity_ticker(pos);
    }

    /// `Level.removeBlockEntity` -> `LevelChunk.removeBlockEntity`.
    pub(crate) fn remove_block_entity(&mut self, pos: BlockPos) {
        if let Some(BlockEntity { data: BeData::Sculk(_), .. }) = self.bes_mut().remove(&pos) {
            self.vibration_listeners.retain(|&p| p != pos);
        }
        if let Some(w) = self.tickers_in_level.remove(&pos) {
            self.wrappers[w] = None;
        }
    }

    fn be_has_ticker(&self, be: &BlockEntity) -> bool {
        match be.data {
            BeData::Moving(_) => true,
            // item-processing block entities (hoppers, furnaces, brewing stands...) tick in the game
            BeData::Jukebox { .. } => self.d.get_bool(be.state, prop::HAS_RECORD),
            // overworld: the dimension has sky light
            BeData::Daylight => true,
            BeData::Sculk(_) => true,
            BeData::Container(_) => matches!(self.kind(be.state), Kind::Hopper)
                || self.d.bflags(self.d.block_of(be.state)) & crate::data::bflag::ITEM_PROCESSOR != 0,
            _ => false,
        }
    }

    /// `LevelChunk.updateBlockEntityTicker`
    fn update_block_entity_ticker(&mut self, pos: BlockPos) {
        let Some(be) = self.bes().get(&pos) else { return };
        if !self.be_has_ticker(be) {
            if let Some(w) = self.tickers_in_level.remove(&pos) {
                self.wrappers[w] = None;
            }
            return;
        }
        let binding = Some((pos, be.gen));
        if let Some(&w) = self.tickers_in_level.get(&pos) {
            self.wrappers[w] = binding;
        } else {
            let w = if let Some(w) = self.free_wrappers.pop() {
                self.wrappers[w] = binding;
                w
            } else {
                self.wrappers.push(binding);
                self.wrappers.len() - 1
            };
            self.tickers_in_level.insert(pos, w);
            if self.ticking_block_entities {
                self.pending_tickers.push(w);
            } else {
                self.tickers.push(w);
            }
        }
    }

    #[inline]
    fn wrapper_removed(&self, w: usize) -> bool {
        match self.wrappers[w] {
            None => true,
            Some((pos, gen)) => self.bes().get(&pos).map_or(true, |be| be.gen != gen),
        }
    }

    fn tick_block_entities(&mut self) {
        self.phase = Phase::BlockEntities;
        self.ticking_block_entities = true;
        if !self.pending_tickers.is_empty() {
            let p = std::mem::take(&mut self.pending_tickers);
            self.tickers.extend(p);
        }
        let list = std::mem::take(&mut self.tickers);
        let mut kept = std::mem::take(&mut self.ticker_spare);
        kept.clear();
        for &w in &list {
            // one lookup answers what wrapper_removed, the type check and the ticker dispatch ask
            let found = self.wrappers[w].and_then(|(pos, gen)| {
                let state = self.get(pos);
                match self.bes().get(&pos) {
                    Some(be) if be.gen == gen => {
                        let ticker = if self.be_type_valid(be, state) { Some(Ticker::of(&be.data)) } else { None };
                        Some((pos, state, ticker))
                    }
                    _ => None,
                }
            });
            let Some((pos, state, ticker)) = found else {
                // a removed wrapper is never referenced from tickers_in_level
                self.wrappers[w] = None;
                self.free_wrappers.push(w);
                continue;
            };
            kept.push(w);
            if let Some(t) = ticker {
                self.run_ticker(t, pos, state);
            }
        }
        // anything added to `tickers` directly during the loop (should not happen) is kept
        kept.extend(std::mem::take(&mut self.tickers));
        self.tickers = kept;
        self.ticker_spare = list; // reused next tick, so ticking allocates nothing
        self.ticking_block_entities = false;
    }

    /// Ticks the block entity at `pos`, whose data is of the `t` kind (`Ticker::of`).
    fn run_ticker(&mut self, t: Ticker, pos: BlockPos, state: State) {
        let saved = self.cause;
        if self.trace.is_some() {
            self.cause = self.trace_event(TraceKind::BlockEntityTick, pos, Cause::None);
        }
        match t {
            Ticker::Moving => self.moving_piston_tick(pos, state),
            Ticker::Jukebox => self.jukebox_tick(pos, state),
            Ticker::Daylight => {
                if self.game_time % 20 == 0 {
                    self.daylight_update(state, pos);
                }
            }
            Ticker::Sculk => self.vibration_tick(pos),
            Ticker::Hopper => self.hopper_tick(pos, state),
            Ticker::Container => self.item_ticker_check(pos, state),
            Ticker::Other => {}
        }
        self.cause = saved;
    }

    /// Item movement is not simulated: warn whenever a ticking container would move items.
    fn item_ticker_check(&mut self, pos: BlockPos, state: State) {
        let has_items = self.container_has_items(pos);
        let _ = state;
        if has_items {
            self.warn(pos, "furnace/brewing stand contents are not processed (not simulated)");
        }
    }

    pub fn block_entity(&self, pos: BlockPos) -> Option<&BlockEntity> {
        self.bes().get(&pos)
    }
    pub(crate) fn moving_be(&self, pos: BlockPos) -> Option<&MovingPiston> {
        match self.bes().get(&pos) {
            Some(BlockEntity { data: BeData::Moving(m), .. }) => Some(m),
            _ => None,
        }
    }
    pub(crate) fn moving_be_mut(&mut self, pos: BlockPos) -> Option<&mut MovingPiston> {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Moving(m), .. }) => Some(m),
            _ => None,
        }
    }

    fn pre_remove_side_effects(&mut self, pos: BlockPos, old: State) {
        if self.moving_be(pos).is_some() {
            self.moving_piston_final_tick(pos);
        } else if self.container_has_items(pos) {
            // Containers.dropContents: the items become item entities
            self.warn(pos, "container with items was removed: its items drop as item entities (not simulated)");
        } else {
            // other block entities that pop their item out when removed
            let what = match self.kind(old) {
                Kind::Jukebox if matches!(self.bes().get(&pos), Some(BlockEntity { data: BeData::Jukebox { item: Some(_), .. }, .. })) => "jukebox's disc",
                Kind::Lectern if self.d.get_bool(old, prop::HAS_BOOK) => "lectern's book",
                Kind::ChiseledBookshelf
                    if [prop::SLOT_0_OCCUPIED, prop::SLOT_1_OCCUPIED, prop::SLOT_2_OCCUPIED, prop::SLOT_3_OCCUPIED, prop::SLOT_4_OCCUPIED, prop::SLOT_5_OCCUPIED]
                        .iter()
                        .any(|&p| self.d.get_bool(old, p)) =>
                {
                    "chiseled bookshelf's books"
                }
                _ => return,
            };
            self.warn(pos, format!("{what} pops out as an item entity when the block is removed (item entities are not simulated)"));
        }
    }

    // ----- game tick ---------------------------------------------------------------

    /// One full game tick (`ServerLevel.tick`, redstone-relevant phases).
    pub fn tick(&mut self) {
        // MinecraftServer.tickChildren: clocks advance before the levels tick
        if self.advance_time {
            self.day_time += 1;
        }
        self.handling_tick = true;
        self.game_time += 1;
        if !self.light_changes.is_empty() {
            let keep = self.game_time - 1;
            self.light_changes.retain(|c| c.1 >= keep);
        }
        // blockTicks
        self.phase = Phase::BlockTicks;
        self.block_ticks.collect(self.game_time);
        while let Some(t) = self.block_ticks.poll_to_run() {
            let state = self.get(t.pos);
            if self.is(state, t.block) {
                let saved = self.cause;
                if self.trace.is_some() {
                    let c = if t.cause > 0 { Cause::Event(t.cause - 1) } else { Cause::None };
                    self.cause = self.trace_event(TraceKind::RunTick { block: t.block }, t.pos, c);
                }
                self.tick_block(state, t.pos);
                self.cause = saved;
            }
        }
        self.block_ticks.cleanup();
        // fluidTicks: not simulated (no fluid behaviour yet)
        // blockEvents
        self.run_block_events();
        self.handling_tick = false;
        // entities: only primed TNT is tracked, to warn when it would explode
        if !self.primed_tnt.is_empty() {
            let now = self.game_time;
            let due: Vec<BlockPos> = self.primed_tnt.iter().filter(|t| t.1 <= now).map(|t| t.0).collect();
            self.primed_tnt.retain(|t| t.1 > now);
            for p in due {
                self.warn(p, "primed TNT explodes here: explosions are not simulated");
            }
        }
        // blockEntities
        self.tick_block_entities();
        self.phase = Phase::Action;
    }

    /// Run `n` ticks, jumping over stretches where nothing is due (exactly equivalent to
    /// calling `tick()` n times: a tick with no due scheduled tick, no block event and no
    /// live block-entity ticker changes nothing but the game time).
    pub fn tick_many(&mut self, n: u64) {
        let mut left = n;
        while left > 0 {
            if self.block_events.is_empty() && self.primed_tnt.is_empty() {
                let next = match (self.block_ticks.next_trigger(), self.next_ticker_time()) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                let quiet = match next {
                    None => left,
                    Some(t) => ((t - self.game_time - 1).max(0) as u64).min(left),
                };
                if quiet > 0 {
                    self.skip_quiet(quiet as i64);
                    left -= quiet;
                    continue;
                }
            }
            self.tick();
            left -= 1;
        }
    }

    /// Game time of the next tick in which some block-entity ticker can act (daylight detectors
    /// only act every 20 ticks), or None if there are no live tickers.
    fn next_ticker_time(&self) -> Option<i64> {
        let mut next: Option<i64> = None;
        for &w in self.tickers.iter().chain(self.pending_tickers.iter()) {
            if self.wrapper_removed(w) {
                continue;
            }
            let (pos, _) = self.wrappers[w].unwrap();
            let t = match self.bes().get(&pos) {
                Some(BlockEntity { data: BeData::Daylight, .. }) => (self.game_time.div_euclid(20) + 1) * 20,
                Some(BlockEntity { data: BeData::Sculk(_), .. }) if !self.sculk_ticker_live(pos) => continue,
                _ => return Some(self.game_time + 1),
            };
            next = Some(next.map_or(t, |n: i64| n.min(t)));
        }
        next
    }

    /// Advance the clocks over `n` ticks in which nothing happens.
    fn skip_quiet(&mut self, n: i64) {
        self.game_time += n;
        if self.advance_time {
            self.day_time += n;
        }
    }

    /// True if nothing can change without outside input (no pending ticks, events or tickers;
    /// daylight detectors count as idle while the day clock is stopped and their output is current).
    pub fn is_idle(&self) -> bool {
        self.block_ticks.count() == 0
            && self.block_events.is_empty()
            && self.primed_tnt.is_empty()
            && self.tickers.iter().chain(self.pending_tickers.iter()).all(|&w| {
                if self.wrapper_removed(w) {
                    return true;
                }
                let (pos, _) = self.wrappers[w].unwrap();
                match self.bes().get(&pos) {
                    Some(BlockEntity { data: BeData::Daylight, .. }) => !self.advance_time && self.daylight_is_current(pos),
                    Some(BlockEntity { data: BeData::Sculk(_), .. }) => !self.sculk_ticker_live(pos),
                    _ => false,
                }
            })
    }

    pub fn pending_ticks(&self) -> Vec<ScheduledTick> {
        self.block_ticks.all()
    }
    pub fn pending_block_events(&self) -> Vec<BlockEvent> {
        self.block_events.iter().map(|q| q.e).collect()
    }
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Like a player placing a block: `setBlock(pos, state, 11)` then `Block.setPlacedBy`.
    pub fn place_like_player(&mut self, pos: BlockPos, state: State) -> bool {
        if !self.set_block(pos, state, 11) {
            return false;
        }
        // BlockItem.place: setPlacedBy runs if the placed block is still the same block type
        let s = self.get(pos);
        if self.d.block_of(s) == self.d.block_of(state) {
            match self.kind(s) {
                Kind::Repeater | Kind::Comparator => self.diode_set_placed_by(pos, s),
                Kind::Piston => self.piston_set_placed_by(pos, s),
                Kind::TripwireHook => self.hook_set_placed_by(pos, s),
                _ => {}
            }
        }
        self.game_event(crate::vibration::GameEvent::BlockPlace, pos, Some(s));
        true
    }

    /// Recompute shape-dependent state at pos from its neighbours, with no side effects.
    pub fn reshape(&mut self, pos: BlockPos) -> bool {
        let s = self.get(pos);
        if self.kind(s) == Kind::Observer || self.is_air(s) {
            return false;
        }
        let saved_ticks = self.block_ticks.clone();
        let saved_sub = self.sub_tick_count;
        let ns = self.update_from_neighbour_shapes(s, pos);
        self.block_ticks = saved_ticks;
        self.sub_tick_count = saved_sub;
        if ns != s && !self.is_air(ns) {
            self.place_raw(pos, ns);
            return true;
        }
        false
    }

    /// Run ticks until nothing is pending (or `max` ticks); returns ticks run.
    pub fn run_until_idle(&mut self, max: u64) -> u64 {
        let start = self.game_time;
        while ((self.game_time - start) as u64) < max && !self.is_idle() {
            if self.block_events.is_empty() && self.primed_tnt.is_empty() {
                let next = match (self.block_ticks.next_trigger(), self.next_ticker_time()) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                if let Some(t) = next {
                    let quiet = (t - self.game_time - 1).max(0);
                    let room = max as i64 - (self.game_time - start);
                    if quiet > 0 {
                        self.skip_quiet(quiet.min(room));
                        continue;
                    }
                }
            }
            self.tick();
        }
        (self.game_time - start) as u64
    }

    /// Place a block exactly as given with no updates and no onPlace (used when loading builds);
    /// block entities are created like the game does.
    pub fn place_raw(&mut self, pos: BlockPos, state: State) {
        self.set_block(pos, state, 2 | 16 | 32 | 256 | 512);
    }

    pub fn bes_positions(&self) -> Vec<BlockPos> {
        self.bes().keys().copied().collect()
    }

    /// Set a container's contents: (slot, item id, count).
    pub fn set_container_items(&mut self, pos: BlockPos, items: Vec<ItemStack>) -> bool {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Container(c), .. }) => {
                c.items = items;
                true
            }
            _ => false,
        }
    }

    pub fn set_lectern(&mut self, pos: BlockPos, page: i32, pages: i32) -> bool {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Lectern { page: p, pages: n }, .. }) => {
                *n = pages.max(0);
                *p = page.clamp(0, (pages - 1).max(0));
                true
            }
            _ => false,
        }
    }
    pub fn set_bookshelf_slot(&mut self, pos: BlockPos, slot: i32) -> bool {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Bookshelf { last_slot }, .. }) => {
                *last_slot = slot;
                true
            }
            _ => false,
        }
    }
    pub fn set_crafter_disabled(&mut self, pos: BlockPos, mask: u32) -> bool {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Container(c), .. }) => {
                c.disabled_slots = mask;
                true
            }
            _ => false,
        }
    }

    pub fn set_comparator_output(&mut self, pos: BlockPos, value: i32) -> bool {
        match self.bes_mut().get_mut(&pos) {
            Some(BlockEntity { data: BeData::Comparator { output }, .. }) => {
                *output = value;
                true
            }
            _ => false,
        }
    }

    // ----- tracing -------------------------------------------------------------

    /// Mark the start of an external input; trace events until `end_input` are caused by it.
    pub fn begin_input(&mut self, pos: BlockPos, what: String) {
        self.phase = Phase::Action;
        if self.trace.is_some() {
            self.cause = self.trace_event(TraceKind::Input { what }, pos, Cause::None);
        }
    }
    pub fn end_input(&mut self) {
        self.cause = Cause::None;
    }

    pub(crate) fn trace_set(&mut self, pos: BlockPos, old: State, new: State, fl: i32) {
        if self.trace.is_some() {
            let c = self.cause;
            self.trace_event(TraceKind::SetBlock { old, new, flags: fl }, pos, c);
        }
    }

    pub(crate) fn trace_event(&mut self, kind: TraceKind, pos: BlockPos, cause: Cause) -> Cause {
        let t = self.game_time;
        let ph = self.phase;
        match &mut self.trace {
            Some(tr) => tr.push(t, ph, kind, pos, cause),
            None => Cause::None,
        }
    }
}
