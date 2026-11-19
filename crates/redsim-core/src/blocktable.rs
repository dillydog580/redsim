//! The block table: how `data/blocks.json` (generated from the game by tools/gen_blockdata.py)
//! is parsed, packed into a compact binary at build time (build.rs) and unpacked at run time.
//!
//! This file is shared by build.rs and the library, so it uses only std, serde and serde_json.
//! The binary keeps the small structured part (blocks, properties, tags, shapes...) as JSON
//! and stores the large per-state arrays as columns, delta-coded where that makes them
//! repetitive; build.rs then deflates it. `decode(encode(t)) == t` is checked at build time.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct RawProp {
    pub field: String,
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct RawBlock {
    pub name: String,
    pub first: u32,
    pub n: u32,
    pub default: u32,
    pub props: Vec<String>,
    pub classes: Vec<String>,
    pub hooks: u64,
    pub tags: Vec<u16>,
    pub fields: serde_json::Value,
}

/// The small, structured part of the table.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct Meta {
    pub version: String,
    pub data_version: i32,
    pub props: Vec<RawProp>,
    pub tags: Vec<String>,
    pub fluids: Vec<String>,
    pub hooks: Vec<String>,
    pub blocks: Vec<RawBlock>,
    pub collision_shapes: Vec<Vec<[f64; 6]>>,
    pub item_max_stack: BTreeMap<String, i32>,
    #[serde(default)]
    pub jukebox_songs: BTreeMap<String, (i32, i64)>,
    #[serde(default)]
    pub face_shape_count: usize,
    #[serde(default)]
    pub face_occludes: Vec<[u32; 2]>,
}

/// One state's facts: (flags, sturdy full, sturdy center, sturdy rigid, push reaction,
/// destroy speed, fluid, light emission, collision shape).
pub type StateRow = (u16, u8, u8, u8, u8, f32, u8, u8, u16);

/// The whole table, in the forms the simulator uses.
#[derive(Clone, PartialEq, Debug)]
pub struct Table {
    pub meta: Meta,
    pub states: Vec<StateRow>,
    /// per state: [rotate cw90, rotate 180, rotate ccw90, mirror left_right, mirror front_back]
    pub transforms: Vec<[u32; 5]>,
    /// per state: `getLightDampening()` (all 0 when the table has none)
    pub light_dampening: Vec<u8>,
    /// (state, face occlusion shape id per direction) for states with any occluding face
    pub face_occ: Vec<(u32, [u8; 6])>,
    /// per overworld day tick: (cos term of the daylight detector, sky darkening)
    pub daylight: Vec<(f32, u8)>,
}

/// `data/blocks.json` as written by tools/gen_blockdata.py.
#[derive(Deserialize)]
struct Json {
    #[serde(flatten)]
    meta: Meta,
    states: Vec<StateRow>,
    #[serde(default)]
    transforms: Vec<[u32; 5]>,
    #[serde(default)]
    light_dampening: String,
    #[serde(default)]
    face_occ: Vec<Vec<u32>>,
    #[serde(default)]
    daylight: String,
}

/// Parses the generator's JSON.
pub fn from_json(text: &str) -> Table {
    let j: Json = serde_json::from_str(text).expect("block table JSON");
    let n = j.states.len();
    let light_dampening = if j.light_dampening.len() == n {
        j.light_dampening.bytes().map(|c| (c as char).to_digit(16).unwrap_or(0) as u8).collect()
    } else {
        vec![0; n]
    };
    let face_occ = j
        .face_occ
        .iter()
        .map(|e| {
            let mut f = [0u8; 6];
            for i in 0..6 {
                f[i] = e[i + 1] as u8;
            }
            (e[0], f)
        })
        .collect();
    let daylight = j
        .daylight
        .as_bytes()
        .chunks(9)
        .filter(|c| c.len() == 9)
        .map(|c| {
            let txt = std::str::from_utf8(c).unwrap();
            let bits = u32::from_str_radix(&txt[..8], 16).unwrap_or(0);
            (f32::from_bits(bits), u8::from_str_radix(&txt[8..], 16).unwrap_or(0))
        })
        .collect();
    Table { meta: j.meta, states: j.states, transforms: j.transforms, light_dampening, face_occ, daylight }
}

const MAGIC: &[u8; 4] = b"RSBT";
const FORMAT: u32 = 1;

struct Writer(Vec<u8>);

impl Writer {
    fn u8s(&mut self, v: impl Iterator<Item = u8>) {
        self.0.extend(v);
    }
    fn u16s(&mut self, v: impl Iterator<Item = u16>) {
        for x in v {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn u32s(&mut self, v: impl Iterator<Item = u32>) {
        for x in v {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn u32(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
}

/// Packs the table (uncompressed).
pub fn encode(t: &Table) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.u32(FORMAT);
    let meta = serde_json::to_vec(&t.meta).expect("block table meta");
    w.u32(meta.len() as u32);
    w.0.extend_from_slice(&meta);
    let s = &t.states;
    w.u32(s.len() as u32);
    w.u16s(s.iter().map(|r| r.0));
    w.u8s(s.iter().map(|r| r.1));
    w.u8s(s.iter().map(|r| r.2));
    w.u8s(s.iter().map(|r| r.3));
    w.u8s(s.iter().map(|r| r.4));
    w.u32s(s.iter().map(|r| r.5.to_bits()));
    w.u8s(s.iter().map(|r| r.6));
    w.u8s(s.iter().map(|r| r.7));
    w.u16s(s.iter().map(|r| r.8));
    // transforms stay within a block, so (target - state) repeats across states
    w.u32(t.transforms.len() as u32);
    for k in 0..5 {
        w.u32s(t.transforms.iter().enumerate().map(|(i, x)| x[k].wrapping_sub(i as u32)));
    }
    w.u32(t.light_dampening.len() as u32);
    w.u8s(t.light_dampening.iter().copied());
    w.u32(t.face_occ.len() as u32);
    let mut prev = 0u32;
    w.u32s(t.face_occ.iter().map(|e| {
        let d = e.0.wrapping_sub(prev);
        prev = e.0;
        d
    }));
    for k in 0..6 {
        w.u8s(t.face_occ.iter().map(|e| e.1[k]));
    }
    // the daylight curve is smooth: successive float bit patterns differ little
    w.u32(t.daylight.len() as u32);
    let mut prev = 0u32;
    w.u32s(t.daylight.iter().map(|e| {
        let b = e.0.to_bits();
        let d = b.wrapping_sub(prev);
        prev = b;
        d
    }));
    w.u8s(t.daylight.iter().map(|e| e.1));
    w.0
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        s
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn u8s(&mut self, n: usize) -> &'a [u8] {
        self.take(n)
    }
    fn u16s(&mut self, n: usize) -> Vec<u16> {
        self.take(2 * n).chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }
    fn u32s(&mut self, n: usize) -> Vec<u32> {
        self.take(4 * n).chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }
}

/// Unpacks a table written by `encode`.
pub fn decode(b: &[u8]) -> Table {
    let mut r = Reader { b, at: 0 };
    assert_eq!(r.take(4), MAGIC, "not a redsim block table");
    assert_eq!(r.u32(), FORMAT, "unknown block table format");
    let mlen = r.u32() as usize;
    let meta: Meta = serde_json::from_slice(r.take(mlen)).expect("block table meta");
    let n = r.u32() as usize;
    let c0 = r.u16s(n);
    let c1 = r.u8s(n);
    let c2 = r.u8s(n);
    let c3 = r.u8s(n);
    let c4 = r.u8s(n);
    let c5 = r.u32s(n);
    let c6 = r.u8s(n);
    let c7 = r.u8s(n);
    let c8 = r.u16s(n);
    let states = (0..n).map(|i| (c0[i], c1[i], c2[i], c3[i], c4[i], f32::from_bits(c5[i]), c6[i], c7[i], c8[i])).collect();
    drop((c0, c5, c8));
    let nt = r.u32() as usize;
    let mut transforms = vec![[0u32; 5]; nt];
    for k in 0..5 {
        for (i, d) in r.u32s(nt).into_iter().enumerate() {
            transforms[i][k] = d.wrapping_add(i as u32);
        }
    }
    let nl = r.u32() as usize;
    let light_dampening = r.u8s(nl).to_vec();
    let nf = r.u32() as usize;
    let mut prev = 0u32;
    let ids: Vec<u32> = r
        .u32s(nf)
        .into_iter()
        .map(|d| {
            prev = prev.wrapping_add(d);
            prev
        })
        .collect();
    let mut face_occ: Vec<(u32, [u8; 6])> = ids.into_iter().map(|s| (s, [0u8; 6])).collect();
    for k in 0..6 {
        for (i, &v) in r.u8s(nf).iter().enumerate() {
            face_occ[i].1[k] = v;
        }
    }
    let nd = r.u32() as usize;
    let mut prev = 0u32;
    let bits: Vec<u32> = r
        .u32s(nd)
        .into_iter()
        .map(|d| {
            prev = prev.wrapping_add(d);
            prev
        })
        .collect();
    let dark = r.u8s(nd);
    let daylight = (0..nd).map(|i| (f32::from_bits(bits[i]), dark[i])).collect();
    assert_eq!(r.at, b.len(), "trailing bytes in block table");
    Table { meta, states, transforms, light_dampening, face_occ, daylight }
}
