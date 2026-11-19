//! Exact iteration order of `java.util.HashMap` / `HashSet` keyed by `BlockPos`.
//!
//! The game iterates hash sets in two redstone-critical places (redstone wire
//! neighbour updates and piston "delete after move" handling). Their iteration
//! order depends on `BlockPos.hashCode()`, which is why redstone can behave
//! differently at different coordinates. This reproduces OpenJDK's HashMap:
//! bucket = (h ^ h>>>16) & (cap-1), chained in insertion order, resize when size
//! exceeds 0.75*cap, and resize instead of treeify while cap < 64.

use crate::pos::BlockPos;

#[inline]
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// Insertion-ordered chains in a power-of-two table, like OpenJDK's HashMap.
#[derive(Clone, Debug)]
pub struct JavaHashSet<V: Copy> {
    buckets: Vec<Vec<(BlockPos, V)>>,
    size: usize,
}

impl<V: Copy> JavaHashSet<V> {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }
    /// `cap` must be a power of two (HashMap's default initial capacity is 16).
    pub fn with_capacity(cap: usize) -> Self {
        JavaHashSet { buckets: vec![Vec::new(); cap], size: 0 }
    }
    #[inline]
    fn index(&self, p: BlockPos) -> usize {
        (spread(p.java_hash()) as usize) & (self.buckets.len() - 1)
    }
    /// `put`/`add`: keeps the original position if the key exists (value replaced).
    pub fn insert(&mut self, p: BlockPos, v: V) -> bool {
        let i = self.index(p);
        let chain = &mut self.buckets[i];
        for e in chain.iter_mut() {
            if e.0 == p {
                e.1 = v;
                return false;
            }
        }
        let bin_count_before = chain.len();
        chain.push((p, v));
        // treeifyBin when the chain already had >= 8 nodes: resize while table < 64
        if bin_count_before >= 8 && self.buckets.len() < 64 {
            self.resize();
        }
        self.size += 1;
        if self.size > self.buckets.len() * 3 / 4 {
            self.resize();
        }
        true
    }
    pub fn remove(&mut self, p: BlockPos) -> bool {
        let i = self.index(p);
        let chain = &mut self.buckets[i];
        if let Some(k) = chain.iter().position(|e| e.0 == p) {
            chain.remove(k);
            self.size -= 1;
            true
        } else {
            false
        }
    }
    fn resize(&mut self) {
        let new_cap = self.buckets.len() * 2;
        let mut nb: Vec<Vec<(BlockPos, V)>> = vec![Vec::new(); new_cap];
        for chain in self.buckets.drain(..) {
            for e in chain {
                let i = (spread(e.0.java_hash()) as usize) & (new_cap - 1);
                nb[i].push(e);
            }
        }
        self.buckets = nb;
    }
    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    /// Entries in Java iteration order.
    pub fn iter(&self) -> impl Iterator<Item = &(BlockPos, V)> {
        self.buckets.iter().flat_map(|c| c.iter())
    }
}

impl<V: Copy> Default for JavaHashSet<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Order in which `Sets.newHashSet()` filled with `items` (in this insertion order) iterates.
pub fn hashset_order(items: &[BlockPos]) -> Vec<BlockPos> {
    let mut s: JavaHashSet<()> = JavaHashSet::new();
    for &p in items {
        s.insert(p, ());
    }
    s.iter().map(|e| e.0).collect()
}

/// Allocation-free `hashset_order` for exactly 7 distinct positions (the dust update set):
/// 7 entries never trigger a resize (threshold 12) or treeification, so the order is
/// bucket index, then insertion order within a bucket.
#[inline]
pub fn hashset_order7(items: &[BlockPos; 7]) -> [BlockPos; 7] {
    let mut keyed = [(0u32, 0u8); 7];
    for (i, p) in items.iter().enumerate() {
        keyed[i] = (spread(p.java_hash()) & 15, i as u8);
    }
    // stable insertion sort by bucket
    for i in 1..7 {
        let mut j = i;
        while j > 0 && keyed[j - 1].0 > keyed[j].0 {
            keyed.swap(j - 1, j);
            j -= 1;
        }
    }
    let mut out = [items[0]; 7];
    for (k, &(_, i)) in keyed.iter().enumerate() {
        out[k] = items[i as usize];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order7_matches_general() {
        for x in -40..40 {
            for y in [-64, -1, 0, 1, 63, 200] {
                for z in -40..40 {
                    let c = BlockPos::new(x * 7, y, z * 13);
                    let mut items = [c; 7];
                    for (i, d) in crate::pos::Dir::ALL.iter().enumerate() {
                        items[i + 1] = c.rel(*d);
                    }
                    assert_eq!(hashset_order(&items), hashset_order7(&items).to_vec());
                }
            }
        }
    }
}
