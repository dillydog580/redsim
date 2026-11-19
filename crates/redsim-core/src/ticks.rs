//! Scheduled block ticks: an exact model of `LevelTicks` + `LevelChunkTicks`.
//!
//! Ticks live in per-chunk priority queues ordered by (triggerTick, priority,
//! subTickOrder). Each game tick, due ticks are merged across chunks by
//! (priority, subTickOrder) into a FIFO that is then run. A tick removed from its
//! chunk queue no longer counts for `hasScheduledTick`; `willTickThisTick` looks at
//! the not-yet-run part of the FIFO.

use crate::data::BlockId;
use crate::pos::BlockPos;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScheduledTick {
    pub block: BlockId,
    pub pos: BlockPos,
    pub trigger: i64,
    /// TickPriority value: -3 (EXTREMELY_HIGH) .. 3 (EXTREMELY_LOW)
    pub priority: i8,
    pub sub: i64,
    /// trace event that scheduled this tick (not part of the game's data)
    pub cause: u32,
}

impl ScheduledTick {
    #[inline]
    fn drain_cmp(&self, o: &Self) -> Ordering {
        self.trigger
            .cmp(&o.trigger)
            .then(self.priority.cmp(&o.priority))
            .then(self.sub.cmp(&o.sub))
    }
    #[inline]
    fn intra_cmp(&self, o: &Self) -> Ordering {
        self.priority.cmp(&o.priority).then(self.sub.cmp(&o.sub))
    }
}

/// Min-heap entry by DRAIN_ORDER.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Drain(ScheduledTick);
impl Ord for Drain {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.drain_cmp(&self.0)
    }
}
impl PartialOrd for Drain {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[derive(Default, Clone, Debug)]
pub struct ChunkTicks {
    queue: BinaryHeap<Drain>,
    per_pos: FxHashSet<(BlockPos, BlockId)>,
}

impl ChunkTicks {
    fn peek(&self) -> Option<ScheduledTick> {
        self.queue.peek().map(|d| d.0)
    }
    fn poll(&mut self) -> Option<ScheduledTick> {
        let t = self.queue.pop()?.0;
        self.per_pos.remove(&(t.pos, t.block));
        Some(t)
    }
}

/// Container queue entry ordered by the head tick's INTRA_TICK_DRAIN_ORDER (min first).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct ContainerRef {
    head: ScheduledTick,
    chunk: i64,
}
impl Ord for ContainerRef {
    fn cmp(&self, o: &Self) -> Ordering {
        o.head.intra_cmp(&self.head)
    }
}
impl PartialOrd for ContainerRef {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[derive(Default, Clone, Debug)]
pub struct LevelTicks {
    containers: FxHashMap<i64, ChunkTicks>,
    to_run: VecDeque<ScheduledTick>,
    to_run_set: FxHashSet<(BlockPos, BlockId)>,
    already_run: Vec<ScheduledTick>,
    pub max_per_tick: usize,
}

impl LevelTicks {
    pub fn new() -> Self {
        LevelTicks { max_per_tick: 65536, ..Default::default() }
    }

    pub fn schedule(&mut self, t: ScheduledTick) {
        let c = self.containers.entry(t.pos.chunk_key()).or_default();
        if c.per_pos.insert((t.pos, t.block)) {
            c.queue.push(Drain(t));
        }
    }

    pub fn has_scheduled_tick(&self, pos: BlockPos, block: BlockId) -> bool {
        self.containers
            .get(&pos.chunk_key())
            .map_or(false, |c| c.per_pos.contains(&(pos, block)))
    }

    pub fn will_tick_this_tick(&mut self, pos: BlockPos, block: BlockId) -> bool {
        if self.to_run_set.is_empty() && !self.to_run.is_empty() {
            for t in &self.to_run {
                self.to_run_set.insert((t.pos, t.block));
            }
        }
        self.to_run_set.contains(&(pos, block))
    }

    /// Collect the ticks due at `current` (LevelTicks.collectTicks).
    pub fn collect(&mut self, current: i64) {
        let max = self.max_per_tick;
        let mut heap: BinaryHeap<ContainerRef> = BinaryHeap::new();
        for (&k, c) in self.containers.iter() {
            if let Some(h) = c.peek() {
                if h.trigger <= current {
                    heap.push(ContainerRef { head: h, chunk: k });
                }
            }
        }
        while self.to_run.len() < max {
            let Some(top) = heap.pop() else { break };
            let c = self.containers.get_mut(&top.chunk).unwrap();
            let t = c.poll().unwrap();
            self.to_run.push_back(t);
            // drainFromCurrentContainer
            if self.to_run.len() < max {
                let next_other = heap.peek().map(|r| r.head);
                while self.to_run.len() < max {
                    let Some(n) = c.peek() else { break };
                    if n.trigger > current {
                        break;
                    }
                    if let Some(o) = next_other {
                        if n.intra_cmp(&o) == Ordering::Greater {
                            break;
                        }
                    }
                    c.poll();
                    self.to_run.push_back(n);
                }
            }
            if let Some(n) = c.peek() {
                if n.trigger <= current && self.to_run.len() < max {
                    heap.push(ContainerRef { head: n, chunk: top.chunk });
                }
            }
        }
    }

    /// Next tick to run this game tick (runCollectedTicks loop body).
    pub fn poll_to_run(&mut self) -> Option<ScheduledTick> {
        let t = self.to_run.pop_front()?;
        if !self.to_run_set.is_empty() {
            self.to_run_set.remove(&(t.pos, t.block));
        }
        self.already_run.push(t);
        Some(t)
    }

    pub fn cleanup(&mut self) {
        self.to_run.clear();
        self.to_run_set.clear();
        self.already_run.clear();
        // Empty chunk queues are kept (like the game's per-chunk containers), so a chunk that
        // schedules ticks every few game ticks does not reallocate its queue each time. Nothing
        // depends on which empty queues exist: merging orders ticks by (priority, sub-order).
        if self.containers.len() > 4096 {
            self.containers.retain(|_, c| !c.queue.is_empty());
        }
    }

    pub fn count(&self) -> usize {
        self.containers.values().map(|c| c.queue.len()).sum()
    }

    /// Earliest trigger tick of any pending tick.
    pub fn next_trigger(&self) -> Option<i64> {
        self.containers.values().filter_map(|c| c.peek().map(|t| t.trigger)).min()
    }

    pub fn all(&self) -> Vec<ScheduledTick> {
        let mut v: Vec<ScheduledTick> = self.containers.values().flat_map(|c| c.queue.iter().map(|d| d.0)).collect();
        v.sort_by(|a, b| a.drain_cmp(b));
        v
    }

    pub fn clear(&mut self) {
        self.containers.clear();
        self.cleanup();
    }
}
