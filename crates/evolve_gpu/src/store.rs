//! The big vertex buffer and the big index buffer, and the ranges of them that are handed out (plan D17, task 2.1).
//!
//! The vertices of every shape live in one buffer; the index lists of the **chunks** (runs of consecutive draw items drawn
//! with one draw call) live in the other. Both are [`GrowableBuffer`]s: a buffer plus a [`RangeAlloc`] that hands out
//! ranges of it. When a request does not fit, the buffer is replaced by a larger one (the old contents are copied on the
//! card) and the store's `generation` goes up, which tells the draw list that anything that refers to the old buffer or to
//! absolute positions in it must be looked at again. [`GrowableBuffer::compact`] does the same to close the gaps left by
//! freed ranges, and shrinks the buffer when it can. All memory comes from the tracked allocator (`alloc.rs`), so the
//! budget of the `Geometry` category holds, and running out of it is an error the caller can act on.
//!
//! Data put in with `insert` is **staged**: it waits on the CPU until [`GrowableBuffer::flush`], which sends it in as few
//! writes as possible, joining ranges that lie next to each other (a first frame inserts thousands of small ranges that
//! sit one after the other, and thousands of tiny writes were the slowest thing in the OpenGL backend: seconds, and an
//! occasional timeout). Anything that reads the buffer or copies it on the card (growing, compacting) flushes first.
//! Two ranges are joined only when the second one's block starts where the first one's block ends: the bytes between their
//! data are then the first block's own padding. Anything wider may be another block's live data, and joining across it
//! once wrote zeros over the vertices of shapes that were already on the card (provinces vanished while zooming).

use crate::alloc::{Alloc, AllocError, Category, Tracked};
use crate::ranges::{Block, RangeAlloc};
use crate::vertex::Vertex;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// A buffer with ranges handed out in it, growing and compacting as needed.
pub struct GrowableBuffer {
    alloc: Alloc,
    queue: wgpu::Queue,
    label: &'static str,
    usage: wgpu::BufferUsages,
    element_size: u64,
    buffer: Tracked<wgpu::Buffer>,
    ranges: RangeAlloc,
    /// Changes whenever the buffer is replaced (growing, compacting).
    generation: u64,
    /// Bytes written with `insert` so far (not counting copies on the card when growing or compacting).
    written: u64,
    /// Data that was inserted and not yet sent: its block (where it goes, in elements, and how much is reserved for it) and
    /// what it is.
    staged: BTreeMap<u32, (Block, Vec<u8>)>,
}

impl GrowableBuffer {
    pub fn new(
        alloc: &Alloc,
        queue: &wgpu::Queue,
        label: &'static str,
        usage: wgpu::BufferUsages,
        element_size: u64,
        capacity: u32,
    ) -> Result<GrowableBuffer, AllocError> {
        let usage = usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let buffer = create(alloc, label, usage, element_size, capacity)?;
        Ok(GrowableBuffer {
            alloc: alloc.clone(),
            queue: queue.clone(),
            label,
            usage,
            element_size,
            buffer,
            ranges: RangeAlloc::new(capacity),
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            written: 0,
            staged: BTreeMap::new(),
        })
    }

    /// Bytes the CPU has sent to this buffer so far.
    pub fn written_bytes(&self) -> u64 {
        self.written
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn ranges(&self) -> &RangeAlloc {
        &self.ranges
    }

    pub fn capacity(&self) -> u32 {
        self.ranges.capacity()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Bytes this buffer takes on the card.
    pub fn bytes(&self) -> u64 {
        self.buffer.bytes()
    }

    /// Puts `data` (a whole number of elements) in a new range, growing the buffer when it does not fit.
    pub fn insert(&mut self, data: &[u8]) -> Result<Block, AllocError> {
        assert!((data.len() as u64).is_multiple_of(self.element_size), "whole elements only");
        let len = (data.len() as u64 / self.element_size) as u32;
        let block = match self.ranges.alloc(len) {
            Some(block) => block,
            None => {
                let needed = self.ranges.top() as u64 + crate::ranges::class_size(len) as u64;
                let target = (self.capacity() as u64 * 2).max(needed).min(u32::MAX as u64) as u32;
                self.resize(target)?;
                self.ranges.alloc(len).expect("the buffer was made large enough")
            }
        };
        if len > 0 {
            self.staged.insert(block.start, (block, data.to_vec()));
            self.written += data.len() as u64;
        }
        Ok(block)
    }

    /// Makes sure that `extra` more elements fit after what is handed out now, replacing the buffer once, by one just large
    /// enough (and a little more), when they do not. Growing by doubling on demand, as `insert` does by itself, can
    /// overshoot by half and goes through several buffers that exist next to each other for a moment; a caller that knows
    /// what is coming says so here first.
    pub fn reserve(&mut self, extra: u64) -> Result<(), AllocError> {
        let needed = self.ranges.top() as u64 + extra;
        if needed <= self.capacity() as u64 {
            return Ok(());
        }
        let target = (needed + needed / 8).min(u32::MAX as u64) as u32;
        self.resize(target)
    }

    /// Writes `data` (a whole number of elements) at element `start`, inside a range that is handed out and already on the card: a
    /// part of an index list that changed without the list changing its size. What is staged is sent first, so the writes happen in
    /// the order of the calls.
    pub fn overwrite(&mut self, start: u32, data: &[u8]) {
        assert!((data.len() as u64).is_multiple_of(self.element_size), "whole elements only");
        self.flush();
        self.queue.write_buffer(&self.buffer, start as u64 * self.element_size, data);
        self.written += data.len() as u64;
    }

    pub fn remove(&mut self, block: Block) {
        // Something inserted and freed before it was sent must not be written over what takes its place.
        self.staged.remove(&block.start);
        self.ranges.free(block);
    }

    /// Sends the staged data to the card.
    pub fn flush(&mut self) {
        if self.staged.is_empty() {
            return;
        }
        let staged = std::mem::take(&mut self.staged);
        let element = self.element_size;
        let mut run_start = staged.values().next().expect("not empty").0.start as u64 * element;
        let mut run: Vec<u8> = Vec::new();
        // Where the block of the last range in the run ends: only a range whose block starts right there joins the run.
        let mut run_reserved_end = 0u64;
        for (block, data) in staged.into_values() {
            let at = block.start as u64 * element;
            if !run.is_empty() && at != run_reserved_end {
                self.queue.write_buffer(&self.buffer, run_start, &run);
                run.clear();
            }
            run_reserved_end = (block.start as u64 + block.reserved() as u64) * element;
            if run.is_empty() {
                run_start = at;
            } else {
                run.resize((at - run_start) as usize, 0);
            }
            run.extend_from_slice(&data);
        }
        if !run.is_empty() {
            self.queue.write_buffer(&self.buffer, run_start, &run);
        }
    }

    /// Replaces the buffer by one of `capacity` elements and copies everything up to `top` over. Blocks keep their places.
    fn resize(&mut self, capacity: u32) -> Result<(), AllocError> {
        self.flush();
        let new = create(&self.alloc, self.label, self.usage, self.element_size, capacity)?;
        let used = self.ranges.top() as u64 * self.element_size;
        if used > 0 {
            let mut encoder = self.alloc_encoder();
            encoder.copy_buffer_to_buffer(&self.buffer, 0, &new, 0, used);
            self.queue.submit([encoder.finish()]);
        }
        self.buffer = new;
        self.ranges.grow(capacity);
        self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Packs the live blocks to the start of a new buffer, and returns where each one went. The new buffer is only as
    /// large as needed (but never below `min_capacity` elements), so this also gives memory back. Fails, and changes
    /// nothing, when the new buffer cannot be had. During the call the old and the new buffer both exist.
    pub fn compact<K: Copy>(&mut self, live: impl IntoIterator<Item = (K, Block)>, min_capacity: u32) -> Result<Vec<(K, Block)>, AllocError> {
        self.flush();
        let plan = self.ranges.plan_compaction(live);
        let used: u32 = plan.moves.iter().map(|(_, _, new)| new.reserved()).sum();
        let capacity = ((used as u64 + used as u64 / 4).max(min_capacity as u64).min(u32::MAX as u64) as u32).max(used);
        let new = create(&self.alloc, self.label, self.usage, self.element_size, capacity)?;
        if !plan.moves.is_empty() {
            let mut encoder = self.alloc_encoder();
            // Blocks that lie next to each other now also lie next to each other afterwards: they are copied as one (with their slack,
            // which is inside the buffer too). Thousands of small copies cost the processor more than the card.
            let mut run: Option<(u64, u64, u64)> = None; // (from, to, elements)
            let copy = |encoder: &mut wgpu::CommandEncoder, run: (u64, u64, u64)| {
                encoder.copy_buffer_to_buffer(&self.buffer, run.0 * self.element_size, &new, run.1 * self.element_size, run.2 * self.element_size);
            };
            for (_, old, moved) in &plan.moves {
                if old.len == 0 {
                    continue;
                }
                let (from, to, size) = (old.start as u64, moved.start as u64, old.reserved() as u64);
                match &mut run {
                    Some((run_from, run_to, elements)) if *run_from + *elements == from && *run_to + *elements == to => *elements += size,
                    _ => {
                        if let Some(done) = run.replace((from, to, size)) {
                            copy(&mut encoder, done);
                        }
                    }
                }
            }
            if let Some(done) = run {
                copy(&mut encoder, done);
            }
            self.queue.submit([encoder.finish()]);
        }
        self.buffer = new;
        self.ranges.apply_compaction(&plan);
        self.ranges.set_capacity(capacity);
        self.generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        Ok(plan.moves.into_iter().map(|(key, _, new)| (key, new)).collect())
    }

    fn alloc_encoder(&self) -> wgpu::CommandEncoder {
        self.alloc.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(self.label) })
    }
}

fn create(alloc: &Alloc, label: &'static str, usage: wgpu::BufferUsages, element_size: u64, capacity: u32) -> Result<Tracked<wgpu::Buffer>, AllocError> {
    alloc.create_buffer(
        Category::Geometry,
        &wgpu::BufferDescriptor { label: Some(label), size: (capacity as u64 * element_size).max(4), usage, mapped_at_creation: false },
    )
}

/// The vertex buffer and the index buffer of the renderer.
pub struct GeometryStore {
    pub vertices: GrowableBuffer,
    pub indices: GrowableBuffer,
}

/// A start capacity that holds a small scene without growing, in elements.
const START_VERTICES: u32 = 1 << 14;
const START_INDICES: u32 = 1 << 15;

impl GeometryStore {
    pub fn new(alloc: &Alloc, queue: &wgpu::Queue) -> Result<GeometryStore, AllocError> {
        Ok(GeometryStore {
            vertices: GrowableBuffer::new(alloc, queue, "vertices", wgpu::BufferUsages::VERTEX, Vertex::SIZE, START_VERTICES)?,
            indices: GrowableBuffer::new(alloc, queue, "indices", wgpu::BufferUsages::INDEX, 4, START_INDICES)?,
        })
    }

    /// Changes whenever either buffer was replaced; draw state that refers to a buffer or to absolute positions in it
    /// must be rebuilt then.
    pub fn generation(&self) -> (u64, u64) {
        (self.vertices.generation(), self.indices.generation())
    }

    /// Sends everything staged in either buffer to the card.
    pub fn flush(&mut self) {
        self.vertices.flush();
        self.indices.flush();
    }

    pub fn insert_vertices(&mut self, vertices: &[Vertex]) -> Result<Block, AllocError> {
        self.vertices.insert(bytemuck::cast_slice(vertices))
    }

    pub fn insert_indices(&mut self, indices: &[u32]) -> Result<Block, AllocError> {
        self.indices.insert(bytemuck::cast_slice(indices))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc::Budgets;
    use crate::device::Gpu;

    fn vertex(i: u32) -> Vertex {
        Vertex { point: [i as f32, -(i as f32)], normal: [0.5, 0.25], distance: i as f32 * 2.0, row: i }
    }

    fn vertices(first: u32, count: u32) -> Vec<Vertex> {
        (first..first + count).map(vertex).collect()
    }

    fn read_vertices(gpu: &Gpu, buffer: &mut GrowableBuffer, block: Block) -> Vec<Vertex> {
        buffer.flush();
        let bytes = gpu.read_buffer(buffer.buffer(), block.start as u64 * Vertex::SIZE, block.len as u64 * Vertex::SIZE);
        bytemuck::cast_slice(&bytes).to_vec()
    }

    /// Every block, read with one readback of the whole span instead of one per block (each readback waits for the card).
    fn read_blocks(gpu: &Gpu, buffer: &mut GrowableBuffer, blocks: &[Block]) -> Vec<Vec<Vertex>> {
        buffer.flush();
        let first = blocks.iter().map(|b| b.start).min().unwrap_or(0);
        let last = blocks.iter().map(|b| b.end()).max().unwrap_or(0);
        let bytes = gpu.read_buffer(buffer.buffer(), first as u64 * Vertex::SIZE, (last - first) as u64 * Vertex::SIZE);
        let all: &[Vertex] = bytemuck::cast_slice(&bytes);
        blocks.iter().map(|b| all[(b.start - first) as usize..(b.end() - first) as usize].to_vec()).collect()
    }

    #[test]
    fn what_is_inserted_is_what_the_card_holds() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let a = store.insert_vertices(&vertices(0, 100)).unwrap();
            let b = store.insert_vertices(&vertices(1000, 7)).unwrap();
            assert_eq!(read_vertices(gpu, &mut store.vertices, a), vertices(0, 100));
            assert_eq!(read_vertices(gpu, &mut store.vertices, b), vertices(1000, 7));
            let indices = store.insert_indices(&[3, 1, 4, 1, 5, 9]).unwrap();
            store.flush();
            let bytes = gpu.read_buffer(store.indices.buffer(), indices.start as u64 * 4, 24);
            assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), [3, 1, 4, 1, 5, 9]);
        });
    }

    #[test]
    fn the_buffer_grows_when_full_keeps_its_data_and_says_so() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let first_capacity = store.vertices.capacity();
            let before = store.vertices.generation();
            let mut blocks = Vec::new();
            for i in 0..40u32 {
                blocks.push((i, store.insert_vertices(&vertices(i * 1000, 1000)).unwrap()));
            }
            assert!(store.vertices.capacity() > first_capacity, "40 000 vertices do not fit in {first_capacity}");
            assert_ne!(store.vertices.generation(), before, "the draw list must learn that the buffer was replaced");
            let found = read_blocks(gpu, &mut store.vertices, &blocks.iter().map(|b| b.1).collect::<Vec<_>>());
            for ((i, _), data) in blocks.iter().zip(found) {
                assert_eq!(data, vertices(i * 1000, 1000), "block {i} survived the growth");
            }
            assert_eq!(gpu.alloc.used(Category::Geometry), store.vertices.bytes() + store.indices.bytes());
        });
    }

    #[test]
    fn reserving_room_replaces_the_buffer_once_by_one_just_large_enough_and_keeps_the_data() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let a = store.insert_vertices(&vertices(0, 100)).unwrap();
            let start_capacity = store.vertices.capacity();
            // Room that is there already: nothing happens.
            let generation = store.vertices.generation();
            store.vertices.reserve(1000).unwrap();
            assert_eq!((store.vertices.generation(), store.vertices.capacity()), (generation, start_capacity));
            // 1.5 million vertices in one step: one new buffer, not several doublings, and not twice as much as needed.
            store.vertices.reserve(1_500_000).unwrap();
            let capacity = store.vertices.capacity() as u64;
            assert_ne!(store.vertices.generation(), generation);
            let needed = store.vertices.ranges().top() as u64 + 1_500_000;
            assert!(capacity >= needed && capacity <= needed + needed / 8 + 1, "{capacity} for {needed} needed");
            assert_eq!(read_vertices(gpu, &mut store.vertices, a), vertices(0, 100), "what was in the buffer was copied over");
            // And what was reserved really fits without another replacement.
            let generation = store.vertices.generation();
            let big = store.insert_vertices(&vertices(7, 1_000_000)).unwrap();
            assert_eq!(store.vertices.generation(), generation);
            assert_eq!(gpu.alloc.used(Category::Geometry), store.vertices.bytes() + store.indices.bytes());
            store.vertices.remove(big);
        });
    }

    #[test]
    fn a_freed_range_is_reused_and_nothing_else_moves() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let a = store.insert_vertices(&vertices(0, 100)).unwrap();
            let b = store.insert_vertices(&vertices(500, 100)).unwrap();
            let generation = store.vertices.generation();
            store.vertices.remove(a);
            let c = store.insert_vertices(&vertices(900, 105)).unwrap();
            assert_eq!(c.start, a.start, "the same size class takes the hole");
            assert_eq!(store.vertices.generation(), generation, "no replacement was needed");
            assert_eq!(read_vertices(gpu, &mut store.vertices, b), vertices(500, 100));
            assert_eq!(read_vertices(gpu, &mut store.vertices, c), vertices(900, 105));
        });
    }

    #[test]
    fn compaction_packs_the_live_blocks_into_a_smaller_buffer_with_their_data() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let mut live: Vec<(u32, Block)> = (0..30u32).map(|i| (i, store.insert_vertices(&vertices(i * 100, 100)).unwrap())).collect();
            let big = store.vertices.bytes();
            // Free two thirds.
            for (i, block) in live.clone() {
                if i % 3 != 0 {
                    store.vertices.remove(block);
                    live.retain(|(k, _)| *k != i);
                }
            }
            assert!(store.vertices.ranges().needs_compaction());
            let before = store.vertices.generation();
            let moved = store.vertices.compact(live.iter().copied(), 64).unwrap();
            assert_ne!(store.vertices.generation(), before);
            assert_eq!(moved.len(), 10);
            let found = read_blocks(gpu, &mut store.vertices, &moved.iter().map(|m| m.1).collect::<Vec<_>>());
            for ((i, _), data) in moved.iter().zip(found) {
                assert_eq!(data, vertices(i * 100, 100), "block {i} after compaction");
            }
            assert!(!store.vertices.ranges().needs_compaction());
            assert_eq!(store.vertices.ranges().free_elements(), 0);
            assert!(store.vertices.bytes() < big, "memory was given back: {} -> {}", big, store.vertices.bytes());
            // The compacted buffer keeps working: new blocks go after the packed ones.
            let fresh = store.insert_vertices(&vertices(7000, 50)).unwrap();
            assert_eq!(read_vertices(gpu, &mut store.vertices, fresh), vertices(7000, 50));
            let found = read_blocks(gpu, &mut store.vertices, &moved.iter().map(|m| m.1).collect::<Vec<_>>());
            for ((i, _), data) in moved.iter().zip(found) {
                assert_eq!(data, vertices(i * 100, 100));
            }
        });
    }

    /// Thousands of small ranges are sent as a few merged writes; every one of them must still arrive where it belongs.
    #[test]
    fn many_small_ranges_are_staged_and_all_arrive_in_the_right_place() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let blocks: Vec<(u32, Block)> = (0..3000u32).map(|i| (i, store.insert_vertices(&vertices(i * 10, 1 + i % 7)).unwrap())).collect();
            store.flush();
            // Every one of the 3 000 ranges, in one readback.
            let found = read_blocks(gpu, &mut store.vertices, &blocks.iter().map(|b| b.1).collect::<Vec<_>>());
            for ((i, _), data) in blocks.iter().zip(found) {
                assert_eq!(data, vertices(i * 10, 1 + i % 7), "range {i}");
            }
        });
    }

    /// Blocks that stay next to each other are copied as one when the buffer is packed; every one must still arrive whole, also the
    /// ones with slack after them and the ones that follow a hole.
    #[test]
    fn packing_the_buffer_keeps_every_block_whole_whether_its_neighbours_stay_or_go() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let lens = [5u32, 100, 7, 64, 90, 3, 200, 65, 12, 80];
            let mut blocks: Vec<(u32, Block)> = lens.iter().enumerate().map(|(i, &len)| (i as u32, store.insert_vertices(&vertices(i as u32 * 1000, len)).unwrap())).collect();
            store.flush();
            // Holes at the start, in the middle (two together, and one alone) and at the end.
            for gone in [0u32, 3, 4, 6, 9] {
                let at = blocks.iter().position(|b| b.0 == gone).unwrap();
                store.vertices.remove(blocks.remove(at).1);
            }
            let moved = store.vertices.compact(blocks.iter().copied(), 1).unwrap();
            assert_eq!(moved.len(), 5);
            let found = read_blocks(gpu, &mut store.vertices, &moved.iter().map(|m| m.1).collect::<Vec<_>>());
            for ((i, _), data) in moved.iter().zip(found) {
                assert_eq!(data, vertices(i * 1000, lens[*i as usize]), "block {i}");
            }
            assert_eq!(moved.iter().map(|m| m.1.start).min(), Some(0), "packed to the start");
        });
    }

    /// Taking back thousands of ranges that were not sent yet must not look through everything staged for each one.
    #[test]
    fn thousands_of_ranges_freed_before_they_were_sent_cost_next_to_nothing() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let blocks: Vec<Block> = (0..60_000u32).map(|i| store.insert_vertices(&vertices(i, 3)).unwrap()).collect();
            let started = std::time::Instant::now();
            for block in blocks {
                store.vertices.remove(block);
            }
            assert!(started.elapsed().as_secs_f64() < 0.3, "{:?}", started.elapsed());
            store.flush();
            assert!(store.vertices.written_bytes() > 0);
        });
    }

    #[test]
    fn a_range_freed_before_it_was_sent_does_not_overwrite_what_took_its_place() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let keep = store.insert_vertices(&vertices(0, 100)).unwrap();
            let doomed = store.insert_vertices(&vertices(500, 100)).unwrap();
            store.vertices.remove(doomed); // never sent
            let replacement = store.insert_vertices(&vertices(900, 100)).unwrap();
            assert_eq!(replacement.start, doomed.start, "it took the freed place");
            assert_eq!(read_vertices(gpu, &mut store.vertices, keep), vertices(0, 100));
            assert_eq!(read_vertices(gpu, &mut store.vertices, replacement), vertices(900, 100), "the newer data won");
        });
    }

    /// Two new ranges in the holes on both sides of a block that is already on the card: sending them must not write over it
    /// (they were once joined into one write with zeros in between, and shapes vanished while zooming).
    #[test]
    fn sending_ranges_on_both_sides_of_a_live_block_leaves_that_block_alone() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            let before = store.insert_vertices(&vertices(0, 10)).unwrap();
            let live = store.insert_vertices(&vertices(100, 10)).unwrap();
            let after = store.insert_vertices(&vertices(200, 10)).unwrap();
            store.flush();
            store.vertices.remove(before);
            store.vertices.remove(after);
            let left = store.insert_vertices(&vertices(300, 9)).unwrap();
            let right = store.insert_vertices(&vertices(400, 9)).unwrap();
            assert!(left.end() <= live.start && right.start >= live.end(), "the new ranges took the holes around the live one");
            store.flush();
            let found = read_blocks(gpu, &mut store.vertices, &[left, live, right]);
            assert_eq!(found, [vertices(300, 9), vertices(100, 10), vertices(400, 9)]);
        });
    }

    #[test]
    fn staged_data_survives_growing_and_compacting_the_buffer() {
        Gpu::for_each_test_mode(|gpu| {
            let mut store = GeometryStore::new(&gpu.alloc, &gpu.queue).unwrap();
            // Enough that the buffer has to grow in the middle of staging, with nothing flushed in between.
            let blocks: Vec<(u32, Block)> = (0..60u32).map(|i| (i, store.insert_vertices(&vertices(i * 1000, 1000)).unwrap())).collect();
            let found = read_blocks(gpu, &mut store.vertices, &blocks.iter().map(|b| b.1).collect::<Vec<_>>());
            for ((i, _), data) in blocks.iter().zip(found) {
                assert_eq!(data, vertices(i * 1000, 1000), "block {i} after growing");
            }
            // Staged data and a compaction together.
            let extra = store.insert_vertices(&vertices(777, 40)).unwrap();
            let live: Vec<(u32, Block)> = blocks.iter().step_by(2).copied().chain([(999, extra)]).collect();
            let moved = store.vertices.compact(live, 64).unwrap();
            let found = read_blocks(gpu, &mut store.vertices, &moved.iter().map(|m| m.1).collect::<Vec<_>>());
            for ((i, _), data) in moved.iter().zip(found) {
                let expected = if *i == 999 { vertices(777, 40) } else { vertices(i * 1000, 1000) };
                assert_eq!(data, expected, "block {i} after compaction");
            }
        });
    }

    #[test]
    fn running_out_of_budget_is_an_error_that_changes_nothing_and_everything_is_given_back_at_the_end() {
        Gpu::for_each_test_mode(|gpu| {
            let alloc = gpu.alloc.clone();
            let before = alloc.snapshot();
            {
                alloc.set_budgets(Budgets::uniform(10_000_000).with(Category::Geometry, 1_000_000));
                let mut store = GeometryStore::new(&alloc, &gpu.queue).unwrap();
                let first = store.insert_vertices(&vertices(0, 1000)).unwrap();
                let mut failure = None;
                for i in 0..200u32 {
                    if let Err(error) = store.insert_vertices(&vertices(i, 5000)) {
                        failure = Some(error);
                        break;
                    }
                }
                let error = failure.expect("200 x 5000 vertices (24 MB) cannot fit in 1 MB");
                assert!(matches!(error, AllocError::OverBudget { category: Category::Geometry, .. }), "{error}");
                assert!(alloc.used(Category::Geometry) <= 1_000_000, "the budget held: {}", alloc.used(Category::Geometry));
                assert_eq!(read_vertices(gpu, &mut store.vertices, first), vertices(0, 1000), "what was there is still there");
            }
            alloc.set_budgets(Budgets::high());
            assert_eq!(alloc.snapshot(), before, "dropping the store gave every byte back");
        });
    }
}
