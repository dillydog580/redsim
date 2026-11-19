//! Block and block-state tables, extracted from the real game (see tools/gen_blockdata.py).
//!
//! A block state is identified by its global id (`Block.getId(state)` in the game),
//! so ids round-trip with the game, schematics and the oracle.

use crate::gen::{blk, prop};
use crate::pos::Dir;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

pub type State = u32;
pub type BlockId = u16;
pub type PropId = u16;

/// `data/blocks.json`, packed and deflated by build.rs (see `blocktable`).
static PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blocks.bin.z"));

pub mod flag {
    pub const CONDUCTOR: u16 = 1 << 0;
    pub const SIGNAL_SOURCE: u16 = 1 << 1;
    pub const ANALOG: u16 = 1 << 2;
    pub const AIR: u16 = 1 << 3;
    pub const REPLACEABLE: u16 = 1 << 4;
    pub const HAS_BE: u16 = 1 << 5;
    pub const SOLID: u16 = 1 << 6;
    pub const FULL_COLLISION: u16 = 1 << 7;
    pub const SOLID_RENDER: u16 = 1 << 8;
    pub const TICKS_RANDOMLY: u16 = 1 << 9;
    pub const COLLISION_EMPTY: u16 = 1 << 10;
    pub const SHAPE_FULL: u16 = 1 << 11;
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Push {
    PushPull = 0,
    PushOnly = 1,
    Popped = 2,
    Immoveable = 3,
    IgnoreEntity = 4,
}

/// Behaviour class of a block (which game Block subclass it is).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Air,
    /// No redstone-relevant behaviour (full blocks, decoration...).
    Plain,
    Wire,
    Repeater,
    Comparator,
    Torch,
    WallTorch,
    Lever,
    Button,
    Lamp,
    PoweredBlock,
    Observer,
    Piston,
    PistonHead,
    MovingPiston,
    Target,
    Door,
    Trapdoor,
    FenceGate,
    NoteBlock,
    CopperBulb,
    Dispenser,
    Hopper,
    PressurePlate,
    Skull,
    /// Only reacts to shape updates by scheduling a fluid tick when waterlogged.
    WaterlogOnly,
    Wall,
    Fence,
    Bars,
    Stairs,
    /// cake / candle cake: breaks when the block below is not solid
    Cake,
    Carpet,
    /// ordinary (non-redstone) torches
    DecoTorch,
    DecoWallTorch,
    Composter,
    /// chests, barrels, shulker boxes, furnaces, brewing stands
    Container,
    /// plants that pop off when the block below stops supporting them (VegetationBlock family)
    Vegetation,
    /// grass block, podzol, mycelium: `snowy` follows the block above
    Snowy,
    Tnt,
    LightningRod,
    Bell,
    Lectern,
    Jukebox,
    ChiseledBookshelf,
    Crafter,
    /// cauldrons, beehives: comparator-readable, no other redstone behaviour
    StaticAnalog,
    /// rail (the only rail that curves)
    Rail,
    /// powered rail and activator rail (`PoweredRailBlock`)
    PoweredRail,
    /// detector rail (no minecarts are simulated: it only ever switches off)
    DetectorRail,
    TripwireHook,
    /// tripwire string (no entities are simulated: a pressed wire is only ever released)
    Tripwire,
    /// daylight detector (sky light + overworld day clock)
    DaylightDetector,
    /// sculk sensor and calibrated sculk sensor (vibrations from block game events)
    SculkSensor,
    /// shelves: redstone-powered, chain sideways, 3-slot container read from the front
    Shelf,
    /// copper golem statues: comparator output from the pose
    Statue,
    /// big dripleaf (redstone resets the tilt) and its stem
    Dripleaf,
    DripleafStem,
    /// Has game behaviour that the simulator does not model yet.
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct StateInfo {
    pub block: BlockId,
    pub flags: u16,
    pub sturdy_full: u8,
    pub sturdy_center: u8,
    pub sturdy_rigid: u8,
    pub push: Push,
    pub destroy_speed: f32,
    pub fluid: u8,
    pub light: u8,
    pub collision: u16,
}

#[derive(Clone, Debug)]
pub struct PropInfo {
    pub field: String,
    pub name: String,
    pub values: Vec<String>,
    /// value index -> direction, when the values are direction names
    pub dirs: Vec<Option<Dir>>,
    /// value index -> integer, when numeric
    pub ints: Vec<Option<i32>>,
}

/// Per-block "instanceof" facts precomputed from the class hierarchy.
pub mod bflag {
    pub const RAIL: u32 = 1;
    pub const TRAPDOOR: u32 = 2;
    pub const LEAVES: u32 = 4;
    pub const ITEM_PROCESSOR: u32 = 8;
    pub const CHEST: u32 = 16;
}

#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub bflags: u32,
    pub name: String,
    pub short: String,
    pub first: State,
    pub n: u32,
    pub default: State,
    /// (prop, stride, count) in the game's (alphabetical) property order
    pub props: Vec<(PropId, u32, u32)>,
    pub classes: Vec<String>,
    pub hooks: u64,
    pub tags: Vec<u16>,
    pub kind: Kind,
    pub fields: serde_json::Value,
}

pub struct BlockData {
    pub version: String,
    pub data_version: i32,
    pub blocks: Vec<BlockInfo>,
    pub states: Vec<StateInfo>,
    pub props: Vec<PropInfo>,
    pub tags: Vec<String>,
    pub fluids: Vec<String>,
    pub hooks: Vec<String>,
    pub by_name: FxHashMap<String, BlockId>,
    pub tag_by_name: FxHashMap<String, u16>,
    pub collision_shapes: Vec<Vec<crate::shape::Aabb>>,
    pub item_max_stack: FxHashMap<String, i32>,
    /// per state: [rotate cw90, rotate 180, rotate ccw90, mirror left_right (z flip), mirror front_back (x flip)]
    pub transforms: Vec<[u16; 5]>,
    /// music disc item -> (comparator output, song length in ticks)
    pub jukebox_songs: FxHashMap<String, (i32, i64)>,
    /// block entity type names; index 0 = none
    pub be_types: Vec<String>,
    /// per state: the facts read on every block access, packed into 8 bytes (see `Hot`)
    hot: Vec<Hot>,
    /// per state: value of the `power` property, or -1 if it has none
    power: Vec<i8>,
    /// per state: ordinal of the `facing` / `horizontal_facing` direction, or -1 if it has none
    facing: Vec<i8>,
    hfacing: Vec<i8>,
    /// value index of `compare` in the comparator `mode` property
    pub(crate) comparator_compare_idx: u32,
    /// per note block instrument value: whether that instrument plays with a block above
    /// (some block with this instrument has `instrument_above`)
    pub(crate) instrument_works_above: Vec<bool>,
    /// dense (block * num_props + prop) -> where the property's value lives; mask 0 = no such property
    prop_table: Vec<PropSlot>,
    /// per block: index into be_types
    pub block_be_type: Vec<u16>,
    /// per state: `getLightDampening()`
    pub light_dampening: Vec<u8>,
    /// per state: face occlusion shape id per direction (3D data order: down, up, north, south,
    /// west, east) as `LightEngine.getOcclusionShape`; 0 = empty
    pub face_occ: Vec<[u8; 6]>,
    /// `Shapes.faceShapeOccludes(a, b)` as a bit matrix over face shape ids
    pub face_occludes: Vec<u64>,
    /// per overworld day tick: (cos term of the daylight detector, sky darkening)
    pub daylight: Vec<(f32, u8)>,
}

/// Per-state facts needed on hot paths, kept together so one load answers them.
#[derive(Copy, Clone, Debug)]
struct Hot {
    /// value index of every property, packed by `PropSlot::shift`
    vals: u16,
    block: BlockId,
    flags: u16,
    kind: Kind,
    /// `SIGNAL_*` bits: which signal methods can return anything but 0 for this kind
    signal: u8,
}

/// Bits of `Hot::signal`.
pub const SIGNAL_WEAK: u8 = 1;
pub const SIGNAL_DIRECT: u8 = 2;

impl Kind {
    /// Kinds whose neighbour-update handler reports when it did nothing for a reason that
    /// depends only on block states and block entities (see `World::handle_neighbor_changed`).
    #[inline]
    pub const fn skips_repeats(self) -> bool {
        matches!(
            self,
            Kind::Wire
                | Kind::Repeater
                | Kind::Torch
                | Kind::WallTorch
                | Kind::Lamp
                | Kind::Piston
                | Kind::Door
                | Kind::Trapdoor
                | Kind::FenceGate
                | Kind::NoteBlock
                | Kind::Dispenser
                | Kind::Hopper
                | Kind::CopperBulb
                | Kind::Skull
                | Kind::PoweredRail
        )
    }

    /// Which of `getSignal` / `getDirectSignal` can be non-zero for blocks of this kind: every
    /// other kind returns 0 from both (see `World::state_signal_of`, `state_direct_signal_of`).
    pub const fn signal_bits(self) -> u8 {
        match self {
            Kind::Wire | Kind::Repeater | Kind::Comparator | Kind::Torch | Kind::WallTorch | Kind::Lever | Kind::Button
            | Kind::Observer | Kind::PressurePlate | Kind::SculkSensor | Kind::TripwireHook | Kind::LightningRod
            | Kind::Lectern | Kind::DetectorRail => SIGNAL_WEAK | SIGNAL_DIRECT,
            Kind::PoweredBlock | Kind::Target | Kind::DaylightDetector | Kind::Jukebox => SIGNAL_WEAK,
            _ => 0,
        }
    }
}

/// Where one property of one block is stored: value index = `(vals >> shift) & mask`;
/// the state id moves by `stride` per value step (game property order, last varies fastest).
#[derive(Copy, Clone, Debug, Default)]
struct PropSlot {
    stride: u16,
    shift: u8,
    mask: u8,
}

/// Hooks that change world state; a block overriding one we don't model is Unsupported.
const BEHAVIOUR_HOOKS: &[&str] = &[
    "updateShape",
    "neighborChanged",
    "tick",
    "onPlace",
    "affectNeighborsAfterRemoval",
    "getSignal",
    "getDirectSignal",
    "isSignalSource",
    "ownSignal",
    "hasAnalogOutputSignal",
    "triggerEvent",
    "getTicker",
    "updateIndirectNeighbourShapes",
];

fn kind_for(classes: &[String], name: &str, hooks: u64, hook_names: &[String]) -> Kind {
    let leaf = classes.first().map(|s| s.as_str()).unwrap_or("");
    let has = |c: &str| classes.iter().any(|x| x == c);
    if name == "minecraft:air" || name == "minecraft:cave_air" || name == "minecraft:void_air" {
        return Kind::Air;
    }
    let k = match leaf {
        "RedstoneWireBlock" => Kind::Wire,
        "RepeaterBlock" => Kind::Repeater,
        "ComparatorBlock" => Kind::Comparator,
        "RedstoneTorchBlock" => Kind::Torch,
        "RedstoneWallTorchBlock" => Kind::WallTorch,
        "LeverBlock" => Kind::Lever,
        "RedstoneLampBlock" => Kind::Lamp,
        "PoweredBlock" => Kind::PoweredBlock,
        "ObserverBlock" => Kind::Observer,
        "PistonBaseBlock" => Kind::Piston,
        "PistonHeadBlock" => Kind::PistonHead,
        "MovingPistonBlock" => Kind::MovingPiston,
        "TargetBlock" => Kind::Target,
        _ if has("ButtonBlock") => Kind::Button,
        _ if has("DoorBlock") => Kind::Door,
        _ if has("TrapDoorBlock") => Kind::Trapdoor,
        _ if has("FenceGateBlock") => Kind::FenceGate,
        _ if has("NoteBlock") => Kind::NoteBlock,
        _ if has("CopperBulbBlock") => Kind::CopperBulb,
        _ if has("DispenserBlock") => Kind::Dispenser,
        _ if has("HopperBlock") => Kind::Hopper,
        _ if has("BasePressurePlateBlock") => Kind::PressurePlate,
        _ if has("AbstractSkullBlock") => Kind::Skull,
        _ if has("WallBlock") => Kind::Wall,
        _ if has("FenceBlock") => Kind::Fence,
        _ if has("IronBarsBlock") => Kind::Bars,
        _ if has("StairBlock") => Kind::Stairs,
        _ if has("CakeBlock") || has("CandleCakeBlock") => Kind::Cake,
        _ if has("CarpetBlock") => Kind::Carpet,
        "TorchBlock" => Kind::DecoTorch,
        "ComposterBlock" => Kind::Composter,
        _ if has("SnowyBlock") => Kind::Snowy,
        "TntBlock" => Kind::Tnt,
        _ if has("LightningRodBlock") => Kind::LightningRod,
        "BellBlock" => Kind::Bell,
        "LecternBlock" => Kind::Lectern,
        "JukeboxBlock" => Kind::Jukebox,
        "ChiseledBookShelfBlock" => Kind::ChiseledBookshelf,
        "CrafterBlock" => Kind::Crafter,
        "RailBlock" => Kind::Rail,
        "PoweredRailBlock" => Kind::PoweredRail,
        "DetectorRailBlock" => Kind::DetectorRail,
        "TripWireHookBlock" => Kind::TripwireHook,
        "TripWireBlock" => Kind::Tripwire,
        "DaylightDetectorBlock" => Kind::DaylightDetector,
        "SculkSensorBlock" | "CalibratedSculkSensorBlock" => Kind::SculkSensor,
        "ShelfBlock" => Kind::Shelf,
        _ if has("CopperGolemStatueBlock") => Kind::Statue,
        "BigDripleafBlock" => Kind::Dripleaf,
        "BigDripleafStemBlock" => Kind::DripleafStem,
        _ if has("AbstractCauldronBlock") || has("BeehiveBlock") => Kind::StaticAnalog,
        "VegetationBlock" | "FlowerBlock" | "TallGrassBlock" | "SaplingBlock" | "BushBlock" | "FlowerBedBlock"
        | "DryVegetationBlock" | "ShortDryGrassBlock" | "TallDryGrassBlock" | "WitherRoseBlock" | "AzaleaBlock" => Kind::Vegetation,
        _ if ["ChestBlock", "BarrelBlock", "ShulkerBoxBlock", "AbstractFurnaceBlock", "BrewingStandBlock", "DecoratedPotBlock"].iter().any(|c| has(c)) => {
            Kind::Container
        }
        "WallTorchBlock" => Kind::DecoWallTorch,
        _ if ["SlabBlock", "ChainBlock", "WaterloggedTransparentBlock", "CandleBlock", "HeavyCoreBlock", "ConduitBlock", "LightBlock"]
            .iter()
            .any(|c| has(c)) =>
        {
            Kind::WaterlogOnly
        }
        _ => {
            let mut unsupported = false;
            for (i, h) in hook_names.iter().enumerate() {
                if hooks & (1u64 << i) != 0 && BEHAVIOUR_HOOKS.contains(&h.as_str()) {
                    unsupported = true;
                }
            }
            if unsupported {
                Kind::Unsupported
            } else {
                Kind::Plain
            }
        }
    };
    k
}

fn load() -> BlockData {
    let table = {
        let bin = miniz_oxide::inflate::decompress_to_vec(PACKED).expect("embedded block table");
        crate::blocktable::decode(&bin)
    }; // the unpacked bytes are freed before the tables are built
    build(table)
}

fn build(t: crate::blocktable::Table) -> BlockData {
    let raw = t.meta;
    let props: Vec<PropInfo> = raw
        .props
        .into_iter()
        .map(|p| {
            let dirs = p.values.iter().map(|v| Dir::from_name(v)).collect();
            let ints = p.values.iter().map(|v| v.parse::<i32>().ok()).collect();
            PropInfo { field: p.field, name: p.name, values: p.values, dirs, ints }
        })
        .collect();
    let prop_index: FxHashMap<String, PropId> =
        props.iter().enumerate().map(|(i, p)| (p.field.clone(), i as PropId)).collect();
    let mut blocks = Vec::with_capacity(raw.blocks.len());
    let mut by_name = FxHashMap::default();
    for (i, b) in raw.blocks.into_iter().enumerate() {
        // strides: last property varies fastest
        let mut stride = 1u32;
        let mut ps: Vec<(PropId, u32, u32)> = Vec::new();
        for pf in b.props.iter().rev() {
            let pid = prop_index[pf];
            let count = props[pid as usize].values.len() as u32;
            ps.push((pid, stride, count));
            stride *= count;
        }
        ps.reverse();
        assert_eq!(stride, b.n, "state count mismatch for {}", b.name);
        let kind = kind_for(&b.classes, &b.name, b.hooks, &raw.hooks);
        by_name.insert(b.name.clone(), i as BlockId);
        let short = b.name.strip_prefix("minecraft:").unwrap_or(&b.name).to_string();
        by_name.insert(short.clone(), i as BlockId);
        let hasc = |c: &str| b.classes.iter().any(|x| x == c);
        let mut bflags = 0;
        if hasc("BaseRailBlock") {
            bflags |= bflag::RAIL;
        }
        if hasc("TrapDoorBlock") {
            bflags |= bflag::TRAPDOOR;
        }
        if hasc("LeavesBlock") {
            bflags |= bflag::LEAVES;
        }
        if hasc("AbstractFurnaceBlock") || hasc("BrewingStandBlock") {
            bflags |= bflag::ITEM_PROCESSOR;
        }
        if hasc("ChestBlock") {
            bflags |= bflag::CHEST;
        }
        blocks.push(BlockInfo {
            bflags,
            name: b.name,
            short,
            first: b.first,
            n: b.n,
            default: b.default,
            props: ps,
            classes: b.classes,
            hooks: b.hooks,
            tags: b.tags,
            kind,
            fields: b.fields,
        });
    }
    let mut states = Vec::with_capacity(t.states.len());
    for (bi, b) in blocks.iter().enumerate() {
        for k in 0..b.n {
            let s = &t.states[(b.first + k) as usize];
            let push = match s.4 {
                0 => Push::PushPull,
                1 => Push::PushOnly,
                2 => Push::Popped,
                3 => Push::Immoveable,
                _ => Push::IgnoreEntity,
            };
            states.push(StateInfo {
                block: bi as BlockId,
                flags: s.0,
                sturdy_full: s.1,
                sturdy_center: s.2,
                sturdy_rigid: s.3,
                push,
                destroy_speed: s.5,
                fluid: s.6,
                light: s.7,
                collision: s.8,
            });
        }
    }
    let tag_by_name = raw.tags.iter().enumerate().map(|(i, t)| (t.clone(), i as u16)).collect();
    let collision_shapes = raw
        .collision_shapes
        .iter()
        .map(|v| v.iter().map(|b| crate::shape::Aabb { min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] }).collect())
        .collect();
    let mut be_types: Vec<String> = vec![String::new()];
    let mut block_be_type = Vec::with_capacity(blocks.len());
    for b in &blocks {
        let ty = b.fields.get("block_entity").and_then(|v| v.as_str()).unwrap_or("");
        if ty.is_empty() {
            block_be_type.push(0);
        } else if let Some(i) = be_types.iter().position(|x| x == ty) {
            block_be_type.push(i as u16);
        } else {
            be_types.push(ty.to_string());
            block_be_type.push((be_types.len() - 1) as u16);
        }
    }
    // state ids (and so property strides) fit 16 bits: the compact tables below rely on it
    assert!(states.len() <= u16::MAX as usize + 1, "more block states than the compact tables hold");
    let np = props.len();
    let mut prop_table = vec![PropSlot::default(); blocks.len() * np];
    let mut hot: Vec<Hot> = states
        .iter()
        .map(|s| {
            let kind = blocks[s.block as usize].kind;
            Hot { vals: 0, block: s.block, flags: s.flags, kind, signal: kind.signal_bits() }
        })
        .collect();
    let power: Vec<i8> = vec![-1; states.len()];
    for (bi, b) in blocks.iter().enumerate() {
        let mut shift = 0u32;
        for &(pid, stride, count) in &b.props {
            let bits = 32 - (count - 1).leading_zeros();
            assert!(count >= 2 && shift + bits <= 16, "property values of {} do not fit 16 bits", b.name);
            prop_table[bi * np + pid as usize] = PropSlot { stride: stride as u16, shift: shift as u8, mask: ((1u32 << bits) - 1) as u8 };
            for k in 0..b.n {
                let idx = (k / stride) % count;
                hot[(b.first + k) as usize].vals |= (idx << shift) as u16;
            }
            shift += bits;
        }
    }
    let mut power = power;
    if let Some(&pid) = prop_index.get("POWER") {
        for (s, v) in power.iter_mut().enumerate() {
            let h = hot[s];
            let slot = prop_table[h.block as usize * np + pid as usize];
            if slot.mask != 0 {
                let idx = ((h.vals >> slot.shift) & slot.mask as u16) as usize;
                if let Some(x) = props[pid as usize].ints[idx] {
                    *v = x as i8;
                }
            }
        }
    }
    let dir_table = |name: &str| -> Vec<i8> {
        let mut v = vec![-1i8; states.len()];
        if let Some(&pid) = prop_index.get(name) {
            for (s, x) in v.iter_mut().enumerate() {
                let h = hot[s];
                let slot = prop_table[h.block as usize * np + pid as usize];
                if slot.mask != 0 {
                    let idx = ((h.vals >> slot.shift) & slot.mask as u16) as usize;
                    if let Some(d) = props[pid as usize].dirs[idx] {
                        *x = d.ordinal() as i8;
                    }
                }
            }
        }
        v
    };
    let facing = dir_table("FACING");
    let hfacing = dir_table("HORIZONTAL_FACING");
    // one pass over the blocks: the instruments some block plays with `instrument_above`
    let above: rustc_hash::FxHashSet<&str> = blocks
        .iter()
        .filter(|b| b.fields.get("instrument_above").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|b| b.fields.get("instrument").and_then(|v| v.as_str()))
        .collect();
    let instrument_works_above = match prop_index.get("NOTEBLOCK_INSTRUMENT") {
        Some(&pid) => props[pid as usize].values.iter().map(|inst| above.contains(inst.as_str())).collect(),
        None => Vec::new(),
    };
    let comparator_compare_idx = prop_index
        .get("MODE_COMPARATOR")
        .and_then(|&pid| props[pid as usize].values.iter().position(|v| v == "compare"))
        .map_or(u32::MAX, |i| i as u32);
    let light_dampening = t.light_dampening;
    let mut face_occ = vec![[0u8; 6]; states.len()];
    for &(s, f) in &t.face_occ {
        face_occ[s as usize] = f;
    }
    assert!(raw.face_shape_count <= 64, "face shape matrix supports at most 64 shapes");
    let mut face_occludes = vec![0u64; raw.face_shape_count.max(1)];
    for &[a, b] in &raw.face_occludes {
        face_occludes[a as usize] |= 1u64 << b;
        face_occludes[b as usize] |= 1u64 << a;
    }
    BlockData {
        light_dampening,
        face_occ,
        face_occludes,
        daylight: t.daylight,
        transforms: t.transforms.iter().map(|x| x.map(|s| s as u16)).collect(),
        jukebox_songs: raw.jukebox_songs.into_iter().collect(),
        hot,
        power,
        facing,
        hfacing,
        instrument_works_above,
        comparator_compare_idx,
        prop_table,
        item_max_stack: raw.item_max_stack.into_iter().collect(),
        be_types,
        block_be_type,
        version: raw.version,
        data_version: raw.data_version,
        blocks,
        states,
        props,
        tags: raw.tags,
        fluids: raw.fluids,
        hooks: raw.hooks,
        by_name,
        tag_by_name,
        collision_shapes,
    }
}

static DATA: OnceLock<BlockData> = OnceLock::new();

#[inline]
pub fn data() -> &'static BlockData {
    DATA.get_or_init(load)
}

impl BlockData {
    #[inline]
    pub fn info(&self, s: State) -> &StateInfo {
        &self.states[s as usize]
    }
    #[inline]
    pub fn collision(&self, s: State) -> &[crate::shape::Aabb] {
        &self.collision_shapes[self.states[s as usize].collision as usize]
    }
    #[inline]
    pub fn block_of(&self, s: State) -> BlockId {
        self.hot[s as usize].block
    }
    #[inline]
    pub fn binfo(&self, b: BlockId) -> &BlockInfo {
        &self.blocks[b as usize]
    }
    #[inline]
    pub fn kind(&self, s: State) -> Kind {
        self.hot[s as usize].kind
    }
    #[inline]
    pub fn bflags(&self, b: BlockId) -> u32 {
        self.blocks[b as usize].bflags
    }
    #[inline]
    pub fn flags(&self, s: State) -> u16 {
        self.hot[s as usize].flags
    }
    #[inline]
    pub fn has_tag(&self, b: BlockId, tag: u16) -> bool {
        self.blocks[b as usize].tags.binary_search(&tag).is_ok()
    }

    /// Value index of `p` in state `s`, or None if the block has no such property.
    #[inline]
    pub fn get_idx(&self, s: State, p: PropId) -> Option<u32> {
        let h = self.hot[s as usize];
        let slot = self.prop_table[h.block as usize * self.props.len() + p as usize];
        if slot.mask == 0 {
            return None;
        }
        Some(((h.vals >> slot.shift) & slot.mask as u16) as u32)
    }
    #[inline]
    pub fn with_idx(&self, s: State, p: PropId, v: u32) -> State {
        let h = self.hot[s as usize];
        let slot = self.prop_table[h.block as usize * self.props.len() + p as usize];
        if slot.mask == 0 {
            panic!("block {} has no property {}", self.blocks[h.block as usize].name, self.props[p as usize].field);
        }
        let cur = ((h.vals >> slot.shift) & slot.mask as u16) as u32;
        s - cur * slot.stride as u32 + v * slot.stride as u32
    }
    #[inline]
    pub fn has_prop(&self, s: State, p: PropId) -> bool {
        self.get_idx(s, p).is_some()
    }
    #[inline]
    pub fn get_bool(&self, s: State, p: PropId) -> bool {
        // boolean properties list "true" first
        self.get_idx(s, p).expect("missing bool property") == 0
    }
    #[inline]
    pub fn with_bool(&self, s: State, p: PropId, v: bool) -> State {
        self.with_idx(s, p, if v { 0 } else { 1 })
    }
    #[inline(always)]
    pub fn get_int(&self, s: State, p: PropId) -> i32 {
        if p == prop::POWER {
            let v = self.power[s as usize];
            if v >= 0 {
                return v as i32;
            }
        }
        self.get_int_slow(s, p)
    }
    #[inline(never)]
    fn get_int_slow(&self, s: State, p: PropId) -> i32 {
        let i = self.get_idx(s, p).expect("missing int property");
        self.props[p as usize].ints[i as usize].expect("not an int property")
    }
    /// `SIGNAL_*` bits of the state's kind.
    #[inline(always)]
    pub fn signal_bits(&self, s: State) -> u8 {
        self.hot[s as usize].signal
    }
    pub fn with_int(&self, s: State, p: PropId, v: i32) -> State {
        let pi = &self.props[p as usize];
        let idx = pi.ints.iter().position(|x| *x == Some(v)).expect("int value out of range");
        self.with_idx(s, p, idx as u32)
    }
    #[inline(always)]
    pub fn get_dir(&self, s: State, p: PropId) -> Dir {
        let v = if p == prop::HORIZONTAL_FACING {
            self.hfacing[s as usize]
        } else if p == prop::FACING {
            self.facing[s as usize]
        } else {
            -1
        };
        if v >= 0 {
            return Dir::ALL_ORDINAL[v as usize];
        }
        self.get_dir_slow(s, p)
    }
    #[inline(never)]
    fn get_dir_slow(&self, s: State, p: PropId) -> Dir {
        let i = self.get_idx(s, p).expect("missing direction property");
        self.props[p as usize].dirs[i as usize].expect("not a direction property")
    }
    pub fn with_dir(&self, s: State, p: PropId, d: Dir) -> State {
        let pi = &self.props[p as usize];
        let idx = pi.dirs.iter().position(|x| *x == Some(d)).expect("direction not allowed");
        self.with_idx(s, p, idx as u32)
    }
    pub fn get_str(&self, s: State, p: PropId) -> &str {
        let i = self.get_idx(s, p).expect("missing property");
        &self.props[p as usize].values[i as usize]
    }
    pub fn with_str(&self, s: State, p: PropId, v: &str) -> State {
        let pi = &self.props[p as usize];
        let idx = pi.values.iter().position(|x| x == v).expect("value not allowed");
        self.with_idx(s, p, idx as u32)
    }
    /// `state.cycle(property)`: next value in the property's value order.
    pub fn cycle(&self, s: State, p: PropId) -> State {
        let i = self.get_idx(s, p).expect("missing property");
        let n = self.props[p as usize].values.len() as u32;
        self.with_idx(s, p, (i + 1) % n)
    }

    pub fn default_state(&self, b: BlockId) -> State {
        self.blocks[b as usize].default
    }

    /// `minecraft:repeater[delay=2,facing=north,...]` (properties in game order).
    pub fn format_state(&self, s: State) -> String {
        let b = &self.blocks[self.states[s as usize].block as usize];
        if b.props.is_empty() {
            return b.name.clone();
        }
        let mut out = b.name.clone();
        out.push('[');
        for (k, &(pid, stride, count)) in b.props.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            let p = &self.props[pid as usize];
            out.push_str(&p.name);
            out.push('=');
            out.push_str(&p.values[(((s - b.first) / stride) % count) as usize]);
        }
        out.push(']');
        out
    }

    /// Parse `name[prop=value,...]`; unspecified properties take the block's default.
    pub fn parse_state(&self, text: &str) -> Result<State, String> {
        let text = text.trim();
        let (name, props) = match text.find('[') {
            Some(i) => {
                if !text.ends_with(']') {
                    return Err(format!("malformed block state '{}'", text));
                }
                (&text[..i], &text[i + 1..text.len() - 1])
            }
            None => (text, ""),
        };
        let b = *self.by_name.get(name).ok_or_else(|| format!("unknown block '{}'", name))?;
        let bi = &self.blocks[b as usize];
        let mut s = bi.default;
        for kv in props.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("malformed property '{}'", kv))?;
            let (k, v) = (k.trim(), v.trim());
            let pid = bi
                .props
                .iter()
                .map(|x| x.0)
                .find(|&pid| self.props[pid as usize].name == k)
                .ok_or_else(|| format!("block {} has no property '{}'", bi.name, k))?;
            let p = &self.props[pid as usize];
            let idx = p.values.iter().position(|x| x == v).ok_or_else(|| {
                format!("invalid value '{}' for {}.{} (allowed: {})", v, bi.name, k, p.values.join(", "))
            })?;
            s = self.with_idx(s, pid, idx as u32);
        }
        Ok(s)
    }

    pub fn block_field_i64(&self, b: BlockId, key: &str) -> Option<i64> {
        self.blocks[b as usize].fields.get(key).and_then(|v| v.as_i64())
    }
    pub fn block_field_bool(&self, b: BlockId, key: &str, sub: &str) -> Option<bool> {
        self.blocks[b as usize].fields.get(key).and_then(|v| v.get(sub)).and_then(|v| v.as_bool())
    }
}

// Convenience shortcuts used across block behaviour code.
#[inline]
pub fn is_block(s: State, b: BlockId) -> bool {
    data().block_of(s) == b
}

pub const AIR: State = crate::gen::st::AIR;

#[allow(unused)]
pub(crate) fn _assert_consts() {
    let _ = (blk::AIR, prop::POWER);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every table the loader builds, as text (maps sorted), for comparing loaders.
    pub(crate) fn fingerprint(d: &BlockData) -> String {
        fn sorted<K: Ord + std::fmt::Debug, V: std::fmt::Debug>(m: impl Iterator<Item = (K, V)>) -> String {
            let mut v: Vec<(K, V)> = m.collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            format!("{:?}", v)
        }
        let daylight: Vec<(u32, u8)> = d.daylight.iter().map(|x| (x.0.to_bits(), x.1)).collect();
        let speeds: Vec<u32> = d.states.iter().map(|s| s.destroy_speed.to_bits()).collect();
        let shapes: Vec<Vec<[u64; 6]>> = d
            .collision_shapes
            .iter()
            .map(|v| v.iter().map(|b| [b.min[0].to_bits(), b.min[1].to_bits(), b.min[2].to_bits(), b.max[0].to_bits(), b.max[1].to_bits(), b.max[2].to_bits()]).collect())
            .collect();
        [
            format!("{:?} {:?}", d.version, d.data_version),
            format!("{:?}", d.blocks),
            format!("{:?}", d.states),
            format!("{:?}", speeds),
            format!("{:?}", d.props),
            format!("{:?} {:?} {:?}", d.tags, d.fluids, d.hooks),
            sorted(d.by_name.iter()),
            sorted(d.tag_by_name.iter()),
            format!("{:?}", shapes),
            sorted(d.item_max_stack.iter()),
            format!("{:?}", d.transforms),
            sorted(d.jukebox_songs.iter()),
            format!("{:?}", d.be_types),
            format!("{:?}", d.hot),
            format!("{:?}", d.power),
            format!("{:?}", d.prop_table),
            format!("{:?}", d.block_be_type),
            format!("{:?}", d.light_dampening),
            format!("{:?}", d.face_occ),
            format!("{:?}", d.face_occludes),
            format!("{:?}", daylight),
        ]
        .join("\n")
    }

    /// The facing lookup tables agree with the general property path for every state, and the
    /// note-block instrument table with the per-block instrument fields.
    #[test]
    fn fast_tables_match_properties() {
        let d = data();
        for s in 0..d.states.len() as State {
            for p in [prop::FACING, prop::HORIZONTAL_FACING] {
                if d.has_prop(s, p) {
                    assert_eq!(d.get_dir(s, p), d.get_dir_slow(s, p), "{}", d.format_state(s));
                }
            }
        }
        let pi = &d.props[prop::NOTEBLOCK_INSTRUMENT as usize];
        for (i, inst) in pi.values.iter().enumerate() {
            let want = d.blocks.iter().any(|b| {
                b.fields.get("instrument").and_then(|v| v.as_str()) == Some(inst.as_str())
                    && b.fields.get("instrument_above").and_then(|v| v.as_bool()) == Some(true)
            });
            assert_eq!(d.instrument_works_above[i], want, "{}", inst);
        }
        assert!(d.instrument_works_above.iter().any(|&x| x) && d.instrument_works_above.iter().any(|&x| !x));
        let modes = &d.props[prop::MODE_COMPARATOR as usize].values;
        assert_eq!(modes, &["compare", "subtract"], "comparator_compare_mode assumes exactly these modes");
        assert_eq!(modes[d.comparator_compare_idx as usize], "compare");
    }

    /// With REDSIM_FINGERPRINT=path, writes the loaded tables there (to compare loaders).
    #[test]
    fn write_fingerprint() {
        if let Ok(path) = std::env::var("REDSIM_FINGERPRINT") {
            std::fs::write(path, fingerprint(data())).unwrap();
        }
    }

    /// Packed property values give the same answers as the state-id arithmetic, for every
    /// property of every state, and `with_idx` lands on the right state.
    #[test]
    fn packed_properties_match_state_ids() {
        let d = data();
        for b in &d.blocks {
            for k in 0..b.n {
                let s = b.first + k;
                for &(pid, stride, count) in &b.props {
                    let want = (k / stride) % count;
                    assert_eq!(d.get_idx(s, pid), Some(want), "{} {}", b.name, d.props[pid as usize].field);
                    for v in 0..count {
                        assert_eq!(d.with_idx(s, pid, v), s - want * stride + v * stride);
                    }
                }
                for p in 0..d.props.len() as PropId {
                    if !b.props.iter().any(|x| x.0 == p) {
                        assert_eq!(d.get_idx(s, p), None);
                    }
                }
                assert_eq!(d.block_of(s) as usize, d.blocks.iter().position(|x| std::ptr::eq(x, b)).unwrap());
            }
        }
    }
}
