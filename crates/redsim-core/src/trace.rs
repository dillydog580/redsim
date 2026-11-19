//! Causal event trace: every block change, neighbour/shape update, scheduled tick
//! and block event, each linked to the event that caused it. This is what lets a
//! user (or an agent) ask "why did this block change at tick N?" and get the exact
//! chain back to the input that started it.

use crate::data::{data, BlockId, State};
use crate::pos::{BlockPos, Dir};
use serde::Serialize;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Phase {
    /// Between ticks: player actions / API calls.
    Action,
    BlockTicks,
    BlockEvents,
    BlockEntities,
}

/// Index of the trace event that caused the current activity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Cause {
    #[default]
    None,
    Event(u32),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceKind {
    /// External input (API call / player action).
    Input { what: String },
    SetBlock { old: State, new: State, flags: i32 },
    NeighborUpdate { block: BlockId, src: Option<BlockPos>, moved: bool },
    ShapeUpdate { dir: Dir, src: BlockPos },
    ScheduleTick { block: BlockId, delay: i32, priority: i8 },
    RunTick { block: BlockId },
    QueueBlockEvent { block: BlockId, a: i32, b: i32 },
    RunBlockEvent { block: BlockId, a: i32, b: i32 },
    BlockEntityTick,
    /// A decision point inside block logic, with a human-readable reason.
    Note { text: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct TraceEvent {
    pub id: u32,
    pub tick: i64,
    pub phase: Phase,
    pub pos: BlockPos,
    pub kind: TraceKind,
    pub cause: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub events: Vec<TraceEvent>,
    pub limit: usize,
}

impl Trace {
    pub fn new(limit: usize) -> Self {
        Trace { events: Vec::new(), limit }
    }
    pub fn push(&mut self, tick: i64, phase: Phase, kind: TraceKind, pos: BlockPos, cause: Cause) -> Cause {
        if self.events.len() >= self.limit {
            return Cause::None;
        }
        let id = self.events.len() as u32;
        let cause = match cause {
            Cause::None => None,
            Cause::Event(i) => Some(i),
        };
        self.events.push(TraceEvent { id, tick, phase, pos, kind, cause });
        Cause::Event(id)
    }

    /// Human readable one-line description of an event.
    pub fn describe(&self, e: &TraceEvent) -> String {
        let d = data();
        let bname = |b: BlockId| d.binfo(b).short.clone();
        let what = match &e.kind {
            TraceKind::Input { what } => format!("input: {}", what),
            TraceKind::SetBlock { old, new, flags } => {
                format!("set {} -> {} (flags {})", d.format_state(*old), d.format_state(*new), flags)
            }
            TraceKind::NeighborUpdate { block, src, moved } => match src {
                Some(s) => format!("neighbor update from {} at {}{}", bname(*block), s, if *moved { " (moved)" } else { "" }),
                None => format!("neighbor update from {}{}", bname(*block), if *moved { " (moved)" } else { "" }),
            },
            TraceKind::ShapeUpdate { dir, src } => format!("shape update from {} (side {})", src, dir.name()),
            TraceKind::ScheduleTick { block, delay, priority } => {
                format!("schedule {} tick in {} gt (priority {})", bname(*block), delay, priority)
            }
            TraceKind::RunTick { block } => format!("scheduled tick of {}", bname(*block)),
            TraceKind::QueueBlockEvent { block, a, b } => format!("queue block event {} ({}, {})", bname(*block), a, b),
            TraceKind::RunBlockEvent { block, a, b } => format!("block event {} ({}, {})", bname(*block), a, b),
            TraceKind::BlockEntityTick => "block entity tick".to_string(),
            TraceKind::Note { text } => text.clone(),
        };
        format!("[t={} {:?} #{}] {} @ {}", e.tick, e.phase, e.id, what, e.pos)
    }

    /// The chain of causes leading to event `id` (oldest first).
    pub fn chain(&self, id: u32) -> Vec<&TraceEvent> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur {
            let e = &self.events[i as usize];
            out.push(e);
            cur = e.cause;
        }
        out.reverse();
        out
    }
}
