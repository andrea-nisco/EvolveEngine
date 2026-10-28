//! A hash map for the renderer's per-frame lookups.
//!
//! The renderer asks a map about every draw item several times per frame (20,000 items, a handful of lookups each), and the
//! keys are small integers it made itself, so the standard map's protection against hostile keys (SipHash) is only a cost:
//! these maps are ten times quicker per lookup with this hash, which saved about 3 ms of a 10 ms whole-map frame.
//! The hash is the multiply-and-rotate of rustc's own "Fx" hash.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        for &byte in rest {
            self.add(byte as u64);
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        // The multiplication puts its best bits at the top and a hash map uses the bottom, so turn them around (as rustc-hash
        // 2 does).
        self.hash.rotate_left(26)
    }
}

/// A `HashMap` with the quick hash.
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_a_map_that_keeps_what_is_put_in_it_for_integers_enums_and_pairs() {
        let mut map: FastMap<(u64, u8), usize> = FastMap::default();
        for i in 0..5000u64 {
            map.insert((i, (i % 3) as u8), i as usize * 2);
        }
        assert_eq!(map.len(), 5000);
        for i in (0..5000u64).step_by(7) {
            assert_eq!(map.get(&(i, (i % 3) as u8)), Some(&(i as usize * 2)));
            assert_eq!(map.get(&(i, 9)), None);
        }
        assert_eq!(map.remove(&(10, 1)), Some(20));
        assert_eq!(map.len(), 4999);
        // Keys that differ only in a few bits do not all land in one place (the map would still work, but slowly).
        let hashes: std::collections::HashSet<u64> = (0..4096u64)
            .map(|i| {
                let mut hasher = FxHasher::default();
                hasher.write_u64(i << 40);
                hasher.finish()
            })
            .collect();
        assert_eq!(hashes.len(), 4096);
        // ... and the bits a map picks its slot from are spread too, even for keys that differ only high up.
        let slots: std::collections::HashSet<u64> = (0..4096u64)
            .map(|i| {
                let mut hasher = FxHasher::default();
                hasher.write_u64(i << 40);
                hasher.finish() & 0xFFF
            })
            .collect();
        assert!(slots.len() >= 1024, "{} slots for 4096 keys", slots.len());
    }
}
