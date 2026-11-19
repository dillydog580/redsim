//! Python bindings for redsim-core (low-level; the friendly API lives in python/redsim).

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use redsim_core::data::{data, State};
use redsim_core::pos::BlockPos;
use redsim_core::trace::{Trace, TraceKind};
use redsim_core::world::{BeData, World};

fn state_arg(v: &Bound<'_, PyAny>) -> PyResult<State> {
    if let Ok(i) = v.extract::<u32>() {
        if (i as usize) < data().states.len() {
            return Ok(i);
        }
        return Err(PyValueError::new_err(format!("state id {} out of range", i)));
    }
    let s: String = v.extract()?;
    data().parse_state(&s).map_err(PyValueError::new_err)
}

#[pyclass(name = "World", module = "redsim._redsim")]
#[derive(Clone)]
struct PyWorld {
    w: World,
}

#[pymethods]
impl PyWorld {
    #[new]
    fn new() -> Self {
        PyWorld { w: World::new() }
    }

    #[getter]
    fn game_time(&self) -> i64 {
        self.w.game_time
    }
    #[setter]
    fn set_game_time(&mut self, t: i64) {
        self.w.game_time = t;
    }

    /// The overworld day clock (0 = sunrise, 6000 = noon, 18000 = midnight; 24000 per day).
    #[getter]
    fn day_time(&self) -> i64 {
        self.w.day_time
    }
    #[setter]
    fn set_day_time(&mut self, t: i64) {
        self.w.day_time = t;
    }
    /// The advance_time game rule: whether the day clock moves with the game ticks.
    #[getter]
    fn advance_time(&self) -> bool {
        self.w.advance_time
    }
    #[setter]
    fn set_advance_time(&mut self, v: bool) {
        self.w.advance_time = v;
    }

    /// Settled sky light level at a position (0-15), as daylight detectors read it.
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.sky_light(BlockPos::new(x, y, z))
    }

    /// Place a block exactly as given: no updates, no onPlace (like pasting a schematic).
    fn place(&mut self, x: i32, y: i32, z: i32, state: &Bound<'_, PyAny>) -> PyResult<()> {
        let s = state_arg(state)?;
        self.w.place_raw(BlockPos::new(x, y, z), s);
        Ok(())
    }

    /// `level.setBlock(pos, state, flags)` with the game's semantics (default flags 3).
    #[pyo3(signature = (x, y, z, state, flags=3))]
    fn set_block(&mut self, x: i32, y: i32, z: i32, state: &Bound<'_, PyAny>, flags: i32) -> PyResult<bool> {
        let s = state_arg(state)?;
        Ok(self.w.set_block(BlockPos::new(x, y, z), s, flags))
    }

    fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        self.w.get(BlockPos::new(x, y, z))
    }

    fn get_str(&self, x: i32, y: i32, z: i32) -> String {
        data().format_state(self.w.get(BlockPos::new(x, y, z)))
    }

    /// Run `n` game ticks.
    #[pyo3(signature = (n=1))]
    fn tick(&mut self, py: Python<'_>, n: u32) {
        let w = &mut self.w;
        py.detach(|| w.tick_many(n as u64));
    }

    /// Player right-click with empty hand. Returns False if not a usable block.
    fn use_block(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.w.use_block(BlockPos::new(x, y, z))
    }

    /// `level.destroyBlock(pos, false)`
    fn destroy_block(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.w.destroy_block(BlockPos::new(x, y, z), false, 512)
    }

    /// Block state ids of the box [x0..x1]x[y0..y1]x[z0..z1] (inclusive) as little-endian u32
    /// bytes, ordered y, z, x (x fastest).
    fn region<'py>(&self, py: Python<'py>, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> Bound<'py, PyBytes> {
        let mut out: Vec<u8> = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1) * (z1 - z0 + 1) * 4).max(0) as usize);
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    out.extend_from_slice(&self.w.get(BlockPos::new(x, y, z)).to_le_bytes());
                }
            }
        }
        PyBytes::new(py, &out)
    }

    /// All non-air blocks as (x, y, z, state).
    fn blocks(&self) -> Vec<(i32, i32, i32, u32)> {
        let mut v = Vec::new();
        self.w.storage().for_each_non_air(|p, s| v.push((p.x, p.y, p.z, s)));
        v
    }

    /// Block entity summary at pos, in the same format the oracle reports.
    fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<String> {
        self.w.block_entity(BlockPos::new(x, y, z)).and_then(|be| match &be.data {
            BeData::Comparator { output } => Some(format!("comparator:{}", output)),
            BeData::Moving(m) => Some(format!(
                "moving:{}:{}:{}:{}:{}",
                m.moved_state,
                m.direction.name(),
                m.extending,
                m.source,
                fmt_float(m.progress)
            )),
            BeData::Container(c) => {
                let mut items: Vec<&redsim_core::world::ItemStack> = c.items.iter().filter(|i| i.count > 0).collect();
                items.sort_by_key(|i| i.slot);
                let mut out = String::from("container:");
                for i in items {
                    out.push_str(&format!("{}={}x{};", i.slot, i.id, i.count));
                }
                Some(out)
            }
            BeData::Lectern { page, pages } => Some(format!("lectern:{}/{}", page, pages)),
            BeData::Bookshelf { last_slot } => Some(format!("bookshelf:{}", last_slot)),
            BeData::Jukebox { item, ticks, playing } => {
                Some(format!("jukebox:{}:{}:{}", item.clone().unwrap_or_default(), playing, if *playing { *ticks } else { 0 }))
            }
            BeData::Sculk(d) => Some(format!(
                "sculk:{}:{}:{}",
                d.last_frequency,
                d.current.map(|v| v.event.name()).unwrap_or_default(),
                if d.current.is_some() { d.travel } else { 0 }
            )),
            BeData::Generic | BeData::Daylight => None,
        })
    }

    fn block_entity_positions(&self) -> Vec<(i32, i32, i32)> {
        let mut v: Vec<(i32, i32, i32)> = self.w.bes_positions().into_iter().map(|p| (p.x, p.y, p.z)).collect();
        v.sort();
        v
    }

    fn set_comparator_output(&mut self, x: i32, y: i32, z: i32, value: i32) -> PyResult<()> {
        if self.w.set_comparator_output(BlockPos::new(x, y, z), value) {
            Ok(())
        } else {
            Err(PyKeyError::new_err("no comparator block entity at that position"))
        }
    }

    /// Set a container's items: list of (slot, item_id, count).
    fn set_container_items(&mut self, x: i32, y: i32, z: i32, items: Vec<(u32, String, i32)>) -> PyResult<()> {
        let items = items
            .into_iter()
            .map(|(slot, id, count)| redsim_core::world::ItemStack {
                slot,
                id: if id.contains(':') { id } else { format!("minecraft:{}", id) },
                count,
            })
            .collect();
        if self.w.set_container_items(BlockPos::new(x, y, z), items) {
            Ok(())
        } else {
            Err(PyKeyError::new_err("no container block entity at that position"))
        }
    }

    fn container_items(&self, x: i32, y: i32, z: i32) -> Option<Vec<(u32, String, i32)>> {
        match self.w.block_entity(BlockPos::new(x, y, z)) {
            Some(be) => match &be.data {
                BeData::Container(c) => Some(c.items.iter().map(|i| (i.slot, i.id.clone(), i.count)).collect()),
                _ => None,
            },
            None => None,
        }
    }

    /// Lectern with a book of `pages` pages open at `page` (raw, like a paste).
    fn set_lectern(&mut self, x: i32, y: i32, z: i32, page: i32, pages: i32) -> bool {
        self.w.set_lectern(BlockPos::new(x, y, z), page, pages)
    }
    /// A player turning a lectern's page (pulses its signal like the game).
    fn lectern_turn_page(&mut self, x: i32, y: i32, z: i32, page: i32) -> bool {
        self.w.lectern_turn_page(BlockPos::new(x, y, z), page)
    }
    fn set_bookshelf_slot(&mut self, x: i32, y: i32, z: i32, slot: i32) -> bool {
        self.w.set_bookshelf_slot(BlockPos::new(x, y, z), slot)
    }
    #[pyo3(signature = (x, y, z, disc, ticks=0))]
    fn set_jukebox(&mut self, x: i32, y: i32, z: i32, disc: Option<String>, ticks: i64) -> bool {
        self.w.set_jukebox(BlockPos::new(x, y, z), disc.map(|d| if d.contains(':') { d } else { format!("minecraft:{}", d) }), ticks)
    }
    fn set_crafter_disabled(&mut self, x: i32, y: i32, z: i32, mask: u32) -> bool {
        self.w.set_crafter_disabled(BlockPos::new(x, y, z), mask)
    }
    fn strike_lightning_rod(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.w.strike_lightning_rod(BlockPos::new(x, y, z))
    }

    fn warnings(&self) -> Vec<(i32, i32, i32, String)> {
        let mut v: Vec<(i32, i32, i32, String)> =
            self.w.warnings.iter().map(|w| (w.pos.x, w.pos.y, w.pos.z, w.message.clone())).collect();
        v.sort();
        v
    }
    fn clear_warnings(&mut self) {
        self.w.warnings.clear();
    }

    fn is_idle(&self) -> bool {
        self.w.is_idle()
    }

    /// Pending scheduled ticks: (x, y, z, block, trigger_tick, priority, order)
    fn pending_ticks(&self) -> Vec<(i32, i32, i32, String, i64, i8, i64)> {
        self.w
            .pending_ticks()
            .into_iter()
            .map(|t| (t.pos.x, t.pos.y, t.pos.z, data().binfo(t.block).name.clone(), t.trigger, t.priority, t.sub))
            .collect()
    }
    fn pending_block_events(&self) -> Vec<(i32, i32, i32, String, i32, i32)> {
        self.w
            .pending_block_events()
            .into_iter()
            .map(|e| (e.pos.x, e.pos.y, e.pos.z, data().binfo(e.block).name.clone(), e.a, e.b))
            .collect()
    }

    /// Start recording a causal trace (up to `limit` events).
    #[pyo3(signature = (limit=2_000_000))]
    fn enable_trace(&mut self, limit: usize) {
        self.w.trace = Some(Trace::new(limit));
    }
    fn disable_trace(&mut self) {
        self.w.trace = None;
    }
    /// Record an external input in the trace (shown as the root cause of what follows).
    fn trace_input(&mut self, x: i32, y: i32, z: i32, what: String) {
        self.w.begin_input(BlockPos::new(x, y, z), what);
    }
    fn end_input(&mut self) {
        self.w.end_input();
    }
    /// Trace events as JSON.
    fn trace_json(&self) -> String {
        match &self.w.trace {
            Some(t) => serde_json::to_string(&t.events).unwrap(),
            None => "[]".to_string(),
        }
    }
    fn trace_len(&self) -> usize {
        self.w.trace.as_ref().map_or(0, |t| t.events.len())
    }
    fn trace_describe(&self, id: u32) -> String {
        match &self.w.trace {
            Some(t) => t.describe(&t.events[id as usize]),
            None => String::new(),
        }
    }
    /// Cause chain (event ids, oldest first) for event `id`.
    fn trace_chain(&self, id: u32) -> Vec<u32> {
        match &self.w.trace {
            Some(t) => t.chain(id).into_iter().map(|e| e.id).collect(),
            None => vec![],
        }
    }
    /// Ids of SetBlock events at a position.
    fn trace_sets_at(&self, x: i32, y: i32, z: i32) -> Vec<u32> {
        let p = BlockPos::new(x, y, z);
        match &self.w.trace {
            Some(t) => t
                .events
                .iter()
                .filter(|e| e.pos == p && matches!(e.kind, TraceKind::SetBlock { .. }))
                .map(|e| e.id)
                .collect(),
            None => vec![],
        }
    }

    /// Player-like placement: setBlock(flags 11) + setPlacedBy (diodes, pistons).
    fn place_like_player(&mut self, x: i32, y: i32, z: i32, state: &Bound<'_, PyAny>) -> PyResult<bool> {
        let s = state_arg(state)?;
        Ok(self.w.place_like_player(BlockPos::new(x, y, z), s))
    }

    /// Recompute a block's shape from its neighbours (connections etc.) without updates.
    /// Observers are left alone (their shape update would schedule a pulse) and blocks
    /// that could not survive are kept.
    fn reshape(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.w.reshape(BlockPos::new(x, y, z))
    }

    fn dust_inputs(&self, x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32)> {
        self.w.dust_inputs(BlockPos::new(x, y, z)).into_iter().map(|p| (p.x, p.y, p.z)).collect()
    }
    fn dust_outputs(&self, x: i32, y: i32, z: i32) -> Vec<(i32, i32, i32)> {
        self.w.dust_outputs(BlockPos::new(x, y, z)).into_iter().map(|p| (p.x, p.y, p.z)).collect()
    }

    /// The shape a player-placed dust at pos would get (game's getStateForPlacement).
    #[pyo3(signature = (x, y, z, power=0))]
    fn dust_placement_state(&self, x: i32, y: i32, z: i32, power: i32) -> u32 {
        self.w.dust_placement_state(BlockPos::new(x, y, z), power)
    }

    /// The state the game's getStateForPlacement gives at pos for blocks whose placed shape depends
    /// only on neighbours (tripwire connections); other states are returned unchanged.
    fn tripwire_placement_state(&self, x: i32, y: i32, z: i32, state: u32) -> u32 {
        self.w.tripwire_placement_state(BlockPos::new(x, y, z), state)
    }

    fn run_until_idle(&mut self, py: Python<'_>, max_ticks: u64) -> u64 {
        let w = &mut self.w;
        py.detach(|| w.run_until_idle(max_ticks))
    }

    /// Send neighbor updates to the 6 neighbours of pos (like a block change there).
    fn update_neighbors(&mut self, x: i32, y: i32, z: i32) {
        let p = BlockPos::new(x, y, z);
        let b = data().block_of(self.w.get(p));
        self.w.update_neighbors_at(p, b);
    }

    /// Send one neighbor update to the block at pos.
    fn update_block(&mut self, x: i32, y: i32, z: i32) {
        let p = BlockPos::new(x, y, z);
        self.w.neighbor_changed(p, redsim_core::gen::blk::AIR);
    }

    fn schedule_tick(&mut self, x: i32, y: i32, z: i32, block: &str, delay: i32, priority: i8) -> PyResult<()> {
        let b = *data().by_name.get(block).ok_or_else(|| PyValueError::new_err(format!("unknown block {}", block)))?;
        self.w.schedule_tick(BlockPos::new(x, y, z), b, delay, priority);
        Ok(())
    }

    /// Strongest signal arriving at pos from its neighbours (`getBestNeighborSignal`).
    fn received_power(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.get_best_neighbor_signal(BlockPos::new(x, y, z))
    }
    /// Strong power into the block at pos (`getDirectSignalTo`).
    fn strong_power(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.get_direct_signal_to(BlockPos::new(x, y, z))
    }
    /// `hasNeighborSignal(pos)`: would a lamp/piston/door here be powered?
    fn is_powered(&self, x: i32, y: i32, z: i32) -> bool {
        self.w.has_neighbor_signal(BlockPos::new(x, y, z))
    }
    /// Weak power emitted by the block at pos toward direction `dir` ("north", ...).
    fn signal_from(&self, x: i32, y: i32, z: i32, dir: &str) -> PyResult<i32> {
        let d = redsim_core::pos::Dir::from_name(dir).ok_or_else(|| PyValueError::new_err("bad direction"))?;
        Ok(self.w.get_signal(BlockPos::new(x, y, z), d))
    }

    fn piston_diagnose(&self, x: i32, y: i32, z: i32) -> String {
        self.w.piston_diagnose(BlockPos::new(x, y, z))
    }

    /// Start/stop recording every block write.
    fn record_changes(&mut self, on: bool) {
        self.w.changes = if on { Some(Vec::new()) } else { None };
    }
    /// Take recorded writes: (tick, phase, x, y, z, old, new); phase 0=action 1=block ticks 2=block events 3=block entities.
    fn take_changes(&mut self) -> Vec<(i64, u8, i32, i32, i32, u32, u32)> {
        match &mut self.w.changes {
            Some(c) => std::mem::take(c)
                .into_iter()
                .map(|c| {
                    let ph = match c.phase {
                        redsim_core::trace::Phase::Action => 0,
                        redsim_core::trace::Phase::BlockTicks => 1,
                        redsim_core::trace::Phase::BlockEvents => 2,
                        redsim_core::trace::Phase::BlockEntities => 3,
                    };
                    (c.tick, ph, c.pos.x, c.pos.y, c.pos.z, c.old, c.new)
                })
                .collect(),
            None => vec![],
        }
    }

    fn copy(&self) -> PyWorld {
        self.clone()
    }
}

fn fmt_float(f: f32) -> String {
    // Java's Float.toString for the values pistons use (0.0, 0.5, 1.0)
    if f == f.trunc() {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

#[pyfunction]
fn parse_state(s: &str) -> PyResult<u32> {
    data().parse_state(s).map_err(PyValueError::new_err)
}

#[pyfunction]
fn format_state(id: u32) -> PyResult<String> {
    if (id as usize) >= data().states.len() {
        return Err(PyValueError::new_err("state id out of range"));
    }
    Ok(data().format_state(id))
}

#[pyfunction]
fn block_name(id: u32) -> String {
    data().binfo(data().block_of(id)).name.clone()
}

#[pyfunction]
fn block_kind(id: u32) -> String {
    format!("{:?}", data().kind(id))
}

/// The game's own `state.rotate`/`state.mirror`: which = 0 cw90, 1 180, 2 ccw90, 3 mirror left_right (z), 4 front_back (x).
#[pyfunction]
fn state_transform(id: u32, which: usize) -> u32 {
    data().transforms[id as usize][which] as u32
}

/// The game's block entity type id for a state's block ("" if none).
#[pyfunction]
fn block_entity_type(id: u32) -> String {
    let b = data().binfo(data().block_of(id));
    b.fields.get("block_entity").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

#[pyfunction]
fn num_states() -> usize {
    data().states.len()
}

#[pyfunction]
fn mc_version() -> String {
    data().version.clone()
}

#[pyfunction]
fn data_version() -> i32 {
    data().data_version
}

/// (name, first_state, num_states, default_state, kind, properties[(name, values)])
#[pyfunction]
fn blocks_table() -> Vec<(String, u32, u32, u32, String, Vec<(String, Vec<String>)>)> {
    let d = data();
    d.blocks
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                b.first,
                b.n,
                b.default,
                format!("{:?}", b.kind),
                b.props.iter().map(|&(p, _, _)| (d.props[p as usize].name.clone(), d.props[p as usize].values.clone())).collect(),
            )
        })
        .collect()
}

/// Per-state flags: (conductor, signal_source, analog, air, replaceable, has_be, solid, full_collision, sturdy_full, sturdy_center, sturdy_rigid, push)
#[pyfunction]
fn state_info(id: u32) -> (u16, u8, u8, u8, String, f32) {
    let s = data().info(id);
    (s.flags, s.sturdy_full, s.sturdy_center, s.sturdy_rigid, format!("{:?}", s.push), s.destroy_speed)
}

#[pymodule]
fn _redsim(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWorld>()?;
    m.add_function(wrap_pyfunction!(parse_state, m)?)?;
    m.add_function(wrap_pyfunction!(format_state, m)?)?;
    m.add_function(wrap_pyfunction!(block_name, m)?)?;
    m.add_function(wrap_pyfunction!(block_kind, m)?)?;
    m.add_function(wrap_pyfunction!(num_states, m)?)?;
    m.add_function(wrap_pyfunction!(block_entity_type, m)?)?;
    m.add_function(wrap_pyfunction!(state_transform, m)?)?;
    m.add_function(wrap_pyfunction!(mc_version, m)?)?;
    m.add_function(wrap_pyfunction!(data_version, m)?)?;
    m.add_function(wrap_pyfunction!(blocks_table, m)?)?;
    m.add_function(wrap_pyfunction!(state_info, m)?)?;
    Ok(())
}
