//! A sub-allocator for ranges of one big buffer (plan D17, task 2.1).
//!
//! The geometry of all shapes lives in one vertex buffer and one index buffer. Shapes change size when they are edited, so
//! the ranges of the buffers are handed out in **size classes** (four steps per doubling, so at most about 19% is wasted
//! and a shape that grows a little usually fits where it was), and a freed range goes on the **free list of its class**
//! to be reused by the next request of that class. Space at the end of the buffer is taken by moving a pointer (`top`).
//! When more than 30% of what has been handed out is free, [`RangeAlloc::needs_compaction`] says so, and
//! [`RangeAlloc::plan_compaction`] gives the moves that close the gaps.
//!
//! This is only bookkeeping about numbers; the buffers themselves are in `store.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A range of elements (vertices or indices) that was handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// First element.
    pub start: u32,
    /// Elements asked for (the part that holds data).
    pub len: u32,
    /// Elements reserved: `len` rounded up to its size class.
    size: u32,
}

impl Block {
    pub const EMPTY: Block = Block { start: 0, len: 0, size: 0 };

    pub fn end(&self) -> u32 {
        self.start + self.len
    }

    pub fn reserved(&self) -> u32 {
        self.size
    }
}

/// The smallest size class that holds `len` elements: 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, ...
pub fn class_size(len: u32) -> u32 {
    if len <= 64 {
        return 64;
    }
    let p = 31 - (len - 1).leading_zeros(); // floor(log2(len - 1)), at least 6 here
    let step = 1u64 << (p - 2);
    let size = (len as u64).div_ceil(step) * step;
    u32::try_from(size).expect("a range of more than 4 billion elements")
}

/// What to move to close the gaps: `(what, old block, new block)`, in the order to do it (lowest first, so that a move
/// never overwrites something that has not been moved yet).
#[derive(Debug, PartialEq, Eq)]
pub struct CompactionPlan<K> {
    pub moves: Vec<(K, Block, Block)>,
    top: u32,
}

#[derive(Debug)]
pub struct RangeAlloc {
    capacity: u32,
    top: u32,
    /// Free blocks by size class: the start of each.
    free: HashMap<u32, BTreeSet<u32>>,
    /// The same free blocks by where they end (end -> size): the one that touches `top`, if any, is the one that ends last.
    ends: BTreeMap<u32, u32>,
    free_elements: u32,
}

impl RangeAlloc {
    pub fn new(capacity: u32) -> RangeAlloc {
        RangeAlloc { capacity, top: 0, free: HashMap::new(), ends: BTreeMap::new(), free_elements: 0 }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// End of the part of the buffer that has ever held data and is not yet given back.
    pub fn top(&self) -> u32 {
        self.top
    }

    /// Elements reserved by live blocks.
    pub fn used(&self) -> u32 {
        self.top - self.free_elements
    }

    /// Elements in free blocks below `top`.
    pub fn free_elements(&self) -> u32 {
        self.free_elements
    }

    /// Hands out a block for `len` elements, or `None` when there is no room (the caller grows the buffer, or compacts).
    pub fn alloc(&mut self, len: u32) -> Option<Block> {
        if len == 0 {
            return Some(Block::EMPTY);
        }
        let size = class_size(len);
        if let Some(start) = self.free.get_mut(&size).and_then(BTreeSet::pop_last) {
            self.ends.remove(&(start + size));
            self.free_elements -= size;
            return Some(Block { start, len, size });
        }
        let end = self.top.checked_add(size).filter(|&end| end <= self.capacity)?;
        let block = Block { start: self.top, len, size };
        self.top = end;
        Some(block)
    }

    /// Gives a block back. Freeing the same block twice is a bug in the caller (and is caught in debug builds).
    pub fn free(&mut self, block: Block) {
        if block.size == 0 {
            return;
        }
        debug_assert!(block.start + block.size <= self.top, "a block that was never handed out");
        debug_assert!(!self.free.get(&block.size).is_some_and(|list| list.contains(&block.start)), "freed twice");
        self.free.entry(block.size).or_default().insert(block.start);
        self.ends.insert(block.start + block.size, block.size);
        self.free_elements += block.size;
        self.release_top();
    }

    /// Lowers `top` over free blocks that touch it, so that freeing the last things leaves a clean buffer.
    fn release_top(&mut self) {
        // (Blocks do not overlap and none ends past `top`, so the block that ends last is the only one that can touch it.)
        while let Some((&end, &size)) = self.ends.last_key_value() {
            if end != self.top {
                return;
            }
            self.ends.remove(&end);
            self.free.get_mut(&size).expect("a free block of this size").remove(&(end - size));
            self.free_elements -= size;
            self.top = end - size;
        }
    }

    /// Makes room for `new_capacity` elements. Blocks stay where they are.
    pub fn grow(&mut self, new_capacity: u32) {
        assert!(new_capacity >= self.capacity, "growing only");
        self.capacity = new_capacity;
    }

    /// Share of the handed-out space (up to `top`) that is free, 0 to 1.
    pub fn fragmentation(&self) -> f32 {
        if self.top == 0 {
            0.0
        } else {
            self.free_elements as f32 / self.top as f32
        }
    }

    /// True when more than 30% of the space up to `top` is free (the plan's rule).
    pub fn needs_compaction(&self) -> bool {
        self.free_elements as u64 * 10 > self.top as u64 * 3
    }

    /// The moves that pack all `live` blocks to the start of the buffer, in order of position, keeping their sizes.
    /// Nothing changes until [`RangeAlloc::apply_compaction`].
    pub fn plan_compaction<K>(&self, live: impl IntoIterator<Item = (K, Block)>) -> CompactionPlan<K> {
        let mut blocks: Vec<(K, Block)> = live.into_iter().filter(|(_, b)| b.size > 0).collect();
        blocks.sort_by_key(|(_, b)| b.start);
        let mut next = 0u32;
        let moves = blocks
            .into_iter()
            .map(|(key, old)| {
                let new = Block { start: next, ..old };
                next += old.size;
                (key, old, new)
            })
            .collect();
        CompactionPlan { moves, top: next }
    }

    /// Sets the capacity after a compaction, when the buffer behind it was replaced by one of another size. Everything
    /// handed out must fit.
    pub fn set_capacity(&mut self, capacity: u32) {
        assert!(capacity >= self.top, "the capacity cannot be below what is in use");
        self.capacity = capacity;
    }

    /// Takes the plan as done: no free blocks are left.
    pub fn apply_compaction<K>(&mut self, plan: &CompactionPlan<K>) {
        self.top = plan.top;
        self.free.clear();
        self.ends.clear();
        self.free_elements = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Giving back the blocks of a whole zoom step (tens of thousands, in the order they lie) used to look through every free block each
    /// time (17 ms of a 45 ms frame on the map). It must be quick and leave a clean buffer.
    #[test]
    fn giving_back_tens_of_thousands_of_blocks_is_quick_and_leaves_a_clean_buffer() {
        let mut ranges = RangeAlloc::new(u32::MAX / 2);
        let blocks: Vec<Block> = (0..60_000).map(|i| ranges.alloc(64 + (i % 5) * 20).unwrap()).collect();
        let started = std::time::Instant::now();
        for block in &blocks {
            ranges.free(*block);
        }
        assert!(started.elapsed().as_secs_f64() < 1.0, "{:?} for 60,000 blocks", started.elapsed());
        assert_eq!((ranges.top(), ranges.used(), ranges.free_elements()), (0, 0, 0), "the last free lowered the top over all of them");
        // Backwards, every free touches the top.
        let blocks: Vec<Block> = (0..1000).map(|_| ranges.alloc(100).unwrap()).collect();
        for block in blocks.iter().rev() {
            ranges.free(*block);
            assert_eq!(ranges.top(), block.start);
        }
    }

    #[test]
    fn size_classes_are_four_steps_per_doubling_and_waste_less_than_a_fifth() {
        let classes: Vec<u32> = [1, 64, 65, 80, 81, 96, 112, 113, 128, 129, 161, 257, 1000, 1025].map(class_size).to_vec();
        assert_eq!(classes, [64, 64, 80, 80, 96, 96, 112, 128, 128, 160, 192, 320, 1024, 1280]);
        for len in (65..200_000).step_by(37) {
            let size = class_size(len);
            assert!(size >= len);
            assert!((size - len) as f32 / size as f32 <= 0.2, "{len} -> {size}");
            assert_eq!(class_size(size), size, "a class size is its own class");
        }
    }

    #[test]
    fn blocks_are_handed_out_one_after_the_other_and_a_freed_block_is_reused_by_its_class() {
        let mut ranges = RangeAlloc::new(10_000);
        let a = ranges.alloc(100).unwrap();
        let b = ranges.alloc(100).unwrap();
        let c = ranges.alloc(100).unwrap();
        assert_eq!((a.start, b.start, c.start), (0, 112, 224));
        assert_eq!(ranges.top(), 336);
        ranges.free(b);
        assert_eq!((ranges.free_elements(), ranges.used()), (112, 224));
        let again = ranges.alloc(105).unwrap();
        assert_eq!(again.start, 112, "the same class takes the freed block");
        assert_eq!(again.len, 105);
        assert_eq!(ranges.free_elements(), 0);
        // A different class does not.
        ranges.free(again);
        let other = ranges.alloc(30).unwrap();
        assert_eq!(other.start, 336, "a 64-class block does not fit in a 112-class hole by this scheme");
    }

    #[test]
    fn freeing_the_last_blocks_lowers_the_top_over_the_holes_that_touch_it() {
        let mut ranges = RangeAlloc::new(10_000);
        let blocks: Vec<Block> = (0..4).map(|_| ranges.alloc(100).unwrap()).collect();
        ranges.free(blocks[1]);
        ranges.free(blocks[2]);
        assert_eq!(ranges.top(), 448);
        ranges.free(blocks[3]);
        assert_eq!(ranges.top(), 112, "the last block and the two holes before it are gone");
        assert_eq!((ranges.free_elements(), ranges.used()), (0, 112));
        ranges.free(blocks[0]);
        assert_eq!(ranges.top(), 0);
    }

    #[test]
    fn a_request_that_does_not_fit_gets_none_and_growing_makes_room() {
        let mut ranges = RangeAlloc::new(200);
        assert!(ranges.alloc(100).is_some());
        assert!(ranges.alloc(100).is_none(), "112 + 112 is more than 200");
        ranges.grow(400);
        assert!(ranges.alloc(100).is_some());
        assert_eq!(ranges.capacity(), 400);
        assert_eq!(ranges.alloc(0), Some(Block::EMPTY), "an empty request needs no room");
    }

    #[test]
    fn compaction_is_wanted_above_thirty_percent_free_and_closes_every_gap() {
        let mut ranges = RangeAlloc::new(100_000);
        let mut live: Vec<(usize, Block)> = (0..10).map(|i| (i, ranges.alloc(1000).unwrap())).collect();
        assert!(!ranges.needs_compaction());
        for i in [1, 3, 5] {
            ranges.free(live.iter().find(|(k, _)| *k == i).unwrap().1);
        }
        live.retain(|(k, _)| ![1, 3, 5].contains(k));
        assert!(!ranges.needs_compaction(), "3 of 10 blocks is exactly 30%, not more");
        ranges.free(live.iter().find(|(k, _)| *k == 7).unwrap().1);
        live.retain(|(k, _)| *k != 7);
        assert!(ranges.needs_compaction());
        assert!(ranges.fragmentation() > 0.3);

        let plan = ranges.plan_compaction(live.iter().copied());
        assert_eq!(plan.moves.len(), 6);
        let mut next = 0;
        for (_, old, new) in &plan.moves {
            assert_eq!(new.start, next, "packed one after the other");
            assert_eq!((new.len, new.reserved()), (old.len, old.reserved()));
            assert!(new.start <= old.start, "blocks only move down, so a move never overwrites a block still to be moved");
            next += new.reserved();
        }
        ranges.apply_compaction(&plan);
        assert_eq!((ranges.top(), ranges.free_elements(), ranges.used()), (next, 0, next));
        assert!(!ranges.needs_compaction());
        // New blocks follow the packed ones.
        assert_eq!(ranges.alloc(1000).unwrap().start, next);
    }

    /// Random allocation, freeing and compaction against a model: ranges never overlap, stay inside the buffer, the
    /// bookkeeping adds up, and data survives a compaction.
    #[test]
    fn random_use_keeps_the_ranges_apart_and_the_data_intact() {
        let mut seed = 99u64;
        let mut random = move |n: u32| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) as u32) % n
        };
        let mut ranges = RangeAlloc::new(4_000);
        let mut memory = vec![u32::MAX; 4_000];
        let mut live: Vec<(u32, Block)> = Vec::new();
        let mut next_id = 0;
        let mut compactions = 0;
        for step in 0..4_000 {
            match random(10) {
                0..=5 => {
                    let len = 1 + random(300);
                    let block = match ranges.alloc(len) {
                        Some(block) => block,
                        None if ranges.needs_compaction() || step % 7 == 0 => {
                            let plan = ranges.plan_compaction(live.iter().copied());
                            for (_, old, new) in &plan.moves {
                                memory.copy_within(old.start as usize..(old.start + old.len) as usize, new.start as usize);
                            }
                            for (id, block) in live.iter_mut() {
                                *block = plan.moves.iter().find(|(k, _, _)| k == id).unwrap().2;
                            }
                            ranges.apply_compaction(&plan);
                            compactions += 1;
                            continue;
                        }
                        None => continue,
                    };
                    assert!(block.start + block.reserved() <= ranges.capacity());
                    for i in 0..block.len {
                        memory[(block.start + i) as usize] = next_id;
                    }
                    live.push((next_id, block));
                    next_id += 1;
                }
                _ if !live.is_empty() => {
                    let (_, block) = live.swap_remove(random(live.len() as u32) as usize);
                    ranges.free(block);
                }
                _ => {}
            }
            // No two live blocks overlap, and all of them hold their own data.
            let mut spans: Vec<(u32, u32)> = live.iter().map(|(_, b)| (b.start, b.start + b.reserved())).collect();
            spans.sort();
            assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0), "overlap at step {step}: {spans:?}");
            assert_eq!(ranges.used(), live.iter().map(|(_, b)| b.reserved()).sum::<u32>(), "used() adds up at step {step}");
            for (id, block) in &live {
                assert!((0..block.len).all(|i| memory[(block.start + i) as usize] == *id), "the data of block {id} was damaged at step {step}");
            }
        }
        assert!(compactions > 0, "the test must have compacted at least once to mean anything");
    }
}
