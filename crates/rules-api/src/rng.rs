//! A small seeded random number generator that games keep in their state.
//!
//! Rules may not use system randomness, so a game replays exactly from its
//! seed and action log. This is SplitMix64: tiny, fast, deterministic on every
//! platform (including wasm32), and good enough for dice and shuffles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`. `n` must be above zero.
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0, "Rng::below(0)");
        // Lemire's multiply-shift; the tiny bias is irrelevant for games.
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    /// A number in `lo..=hi`.
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.below(hi - lo + 1)
    }

    /// One roll of a die with `sides` faces, 1-based.
    pub fn die(&mut self, sides: u32) -> u32 {
        self.range(1, sides)
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_numbers() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn dice_stay_in_range_and_hit_every_face() {
        let mut rng = Rng::new(7);
        let mut seen = [false; 6];
        for _ in 0..600 {
            let d = rng.die(6);
            assert!((1..=6).contains(&d));
            seen[d as usize - 1] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = Rng::new(3);
        let mut v: Vec<u32> = (0..52).collect();
        rng.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(sorted, (0..52).collect::<Vec<_>>());
        assert_ne!(v, sorted);
    }
}
