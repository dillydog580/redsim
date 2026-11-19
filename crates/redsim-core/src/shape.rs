//! Face-support tests on collision shapes (`SupportType` FULL / CENTER / RIGID),
//! needed where the shape is dynamic: a block being moved by a piston (block 36)
//! supports dust, torches, levers... according to its current offset.
//!
//! Semantics follow `VoxelShape.getFaceShape`: the face is the cross-section of the
//! shape at 0.9999999 (positive side) or 1.0E-7 (negative side) along the axis,
//! extruded over the whole block. FULL means that cross-section is exactly the unit
//! square; CENTER/RIGID mean the support pattern's projection lies inside it.

use crate::pos::{Axis, Dir};

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    pub fn moved(&self, d: [f64; 3]) -> Aabb {
        Aabb {
            min: [self.min[0] + d[0], self.min[1] + d[1], self.min[2] + d[2]],
            max: [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]],
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Full,
    Center,
    Rigid,
}

pub type Rect = [f64; 4]; // u0, v0, u1, v1 (plane coordinates: Y->(x,z), X->(y,z), Z->(x,y))

fn axes(a: Axis) -> (usize, usize, usize) {
    // (axis index, u index, v index): Y -> (x, z), X -> (y, z), Z -> (x, y)
    match a {
        Axis::X => (0, 1, 2),
        Axis::Y => (1, 0, 2),
        Axis::Z => (2, 0, 1),
    }
}

fn face_rects(boxes: &[Aabb], d: Dir) -> Vec<Rect> {
    let (ai, ui, vi) = axes(d.axis());
    let c = if d.positive() { 0.9999999 } else { 1.0e-7 };
    boxes
        .iter()
        .filter(|b| b.min[ai] <= c && c < b.max[ai])
        .map(|b| [b.min[ui], b.min[vi], b.max[ui], b.max[vi]])
        .filter(|r| r[2] > r[0] && r[3] > r[1])
        .collect()
}

/// Is every point of `target` (union of rects) covered by `cover` (union of rects)?
fn covers(cover: &[Rect], target: &[Rect]) -> bool {
    let mut us: Vec<f64> = Vec::new();
    let mut vs: Vec<f64> = Vec::new();
    for r in cover.iter().chain(target.iter()) {
        us.push(r[0]);
        us.push(r[2]);
        vs.push(r[1]);
        vs.push(r[3]);
    }
    us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    us.dedup();
    vs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    vs.dedup();
    for i in 0..us.len().saturating_sub(1) {
        for j in 0..vs.len().saturating_sub(1) {
            let cu = (us[i] + us[i + 1]) * 0.5;
            let cv = (vs[j] + vs[j + 1]) * 0.5;
            let inside = |r: &Rect| r[0] <= cu && cu < r[2] && r[1] <= cv && cv < r[3];
            if target.iter().any(inside) && !cover.iter().any(inside) {
                return false;
            }
        }
    }
    true
}

const UNIT: Rect = [0.0, 0.0, 1.0, 1.0];

fn support_rects(ty: Support, a: Axis) -> Vec<Rect> {
    let s = 1.0 / 16.0;
    match ty {
        Support::Full => vec![UNIT],
        // Block.column(2, 0, 10): x,z in [7,9], y in [0,10]
        Support::Center => match a {
            Axis::Y => vec![[7.0 * s, 7.0 * s, 9.0 * s, 9.0 * s]],
            Axis::X => vec![[0.0, 7.0 * s, 10.0 * s, 9.0 * s]], // (y, z)
            Axis::Z => vec![[7.0 * s, 0.0, 9.0 * s, 10.0 * s]], // (x, y)
        },
        // block minus column(12, 0, 16): the outer ring of width 2/16 in x/z
        Support::Rigid => match a {
            Axis::Y => vec![
                [0.0, 0.0, 2.0 * s, 1.0],
                [14.0 * s, 0.0, 1.0, 1.0],
                [0.0, 0.0, 1.0, 2.0 * s],
                [0.0, 14.0 * s, 1.0, 1.0],
            ],
            _ => vec![UNIT],
        },
    }
}

/// `state.isFaceSturdy(level, pos, d, ty)` for a block whose support shape is `boxes`.
pub fn face_supports(boxes: &[Aabb], d: Dir, ty: Support) -> bool {
    let face = face_rects(boxes, d);
    if face.is_empty() {
        return false;
    }
    match ty {
        Support::Full => {
            // face must equal the unit square: cover it and not stick out of it
            if !covers(&face, &[UNIT]) {
                return false;
            }
            face.iter().all(|r| r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= 1.0 && r[3] <= 1.0)
        }
        _ => covers(&face, &support_rects(ty, d.axis())),
    }
}

/// Does the face cross-section of `boxes` on side `d` cover all of `target` (plane coords)?
pub fn face_covers(boxes: &[Aabb], d: Dir, target: &[Rect]) -> bool {
    let face = face_rects(boxes, d);
    covers(&face, target)
}
