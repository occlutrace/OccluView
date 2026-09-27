//! Deterministic hashing for the sculpt kernel's integer-keyed maps.
//!
//! The standard library's random state would reorder iteration between runs
//! and make a remesh step non-reproducible, so every map keyed by a vertex,
//! group or face id uses this fixed, seed-free hash.

use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Fixed multiply-xor hasher. `u64` arithmetic only, so 32-bit and 64-bit
/// targets hash the same key to the same value.
#[derive(Default, Clone)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, remainder) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add_to_hash(u64::from_le_bytes(*chunk));
        }
        if !remainder.is_empty() {
            let mut word = [0u8; 8];
            word[..remainder.len()].copy_from_slice(remainder);
            self.add_to_hash(u64::from_le_bytes(word));
        }
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add_to_hash(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add_to_hash(value);
    }

    #[inline]
    fn write_i64(&mut self, value: i64) {
        self.add_to_hash(value as u64);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add_to_hash(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// Build hasher for the fixed scheme.
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
/// A map keyed by integers with a run-stable iteration order.
pub type FxHashMap<K, V> = std::collections::HashMap<K, V, FxBuildHasher>;
/// A set of integers with a run-stable iteration order.
pub type FxHashSet<K> = std::collections::HashSet<K, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_across_runs_and_key_widths() {
        let mut map = FxHashMap::<(u32, u32), u32>::default();
        map.insert((7, 9), 1);
        map.insert((9, 7), 2);
        assert_eq!(map.get(&(7, 9)), Some(&1));
        // A usize and a u64 carrying the same value must hash equally.
        let mut a = FxHasher::default();
        a.write_usize(0xDEAD_BEEF);
        let mut b = FxHasher::default();
        b.write_u64(0xDEAD_BEEF);
        assert_eq!(a.finish(), b.finish());
    }
}
