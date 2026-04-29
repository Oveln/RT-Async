//! Two-level priority bitmap for O(1) RTOS priority scheduling.
//!
//! Priorities are divided into groups of 64 (one `u64` per group).
//! A group bitmap (`u64`) tracks which groups are non-empty; per-group `u64`
//! bitmaps track individual priorities.  Both levels use `trailing_zeros()`
//! / `leading_zeros()` — single hardware instruction, no lookup table.
//!
//! Lower number = higher priority (0 is highest).
//!
//! # Configuration
//!
//! `PriorityBitmap<N>` has `N` groups of 64 priorities each, supporting up to `N * 64`
//! total priorities.  `N` must be in `1..=64` (i.e. 64–4096 priorities).

pub struct PriorityBitmap<const NUM_GROUPS: usize> {
    group_map: u64,
    group_table: [u64; NUM_GROUPS],
}

impl<const NUM_GROUPS: usize> PriorityBitmap<NUM_GROUPS> {
    const GROUP_BITS: usize = 64;

    pub const CAPACITY: usize = NUM_GROUPS * 64;

    pub const fn new() -> Self {
        assert!(NUM_GROUPS > 0, "NUM_GROUPS must be > 0");
        assert!(NUM_GROUPS <= 64, "NUM_GROUPS must be <= 64");
        Self {
            group_map: 0,
            group_table: [0u64; NUM_GROUPS],
        }
    }

    #[inline]
    pub fn set(&mut self, prio: usize) {
        debug_assert!(prio < Self::CAPACITY);
        let (g, b) = (prio / Self::GROUP_BITS, prio % Self::GROUP_BITS);
        self.group_table[g] |= 1 << b;
        self.group_map |= 1 << g;
    }

    #[inline]
    pub fn clear(&mut self, prio: usize) {
        debug_assert!(prio < Self::CAPACITY);
        let (g, b) = (prio / Self::GROUP_BITS, prio % Self::GROUP_BITS);
        self.group_table[g] &= !(1 << b);
        if self.group_table[g] == 0 {
            self.group_map &= !(1 << g);
        }
    }

    #[inline]
    pub fn highest(&self) -> Option<usize> {
        let gm = self.group_map;
        if gm == 0 {
            return None;
        }
        let g = gm.trailing_zeros() as usize;
        let b = self.group_table[g].trailing_zeros() as usize;
        Some(g * Self::GROUP_BITS + b)
    }

    #[inline]
    pub fn is_set(&self, prio: usize) -> bool {
        debug_assert!(prio < Self::CAPACITY);
        (self.group_table[prio / Self::GROUP_BITS] >> (prio % Self::GROUP_BITS)) & 1 != 0
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.group_map == 0
    }

    #[inline]
    pub fn pop_highest(&mut self) -> Option<usize> {
        let prio = self.highest()?;
        self.clear(prio);
        Some(prio)
    }

    #[inline]
    pub fn clear_all(&mut self) {
        self.group_map = 0;
        self.group_table = [0u64; NUM_GROUPS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let bm: PriorityBitmap<1> = PriorityBitmap::new();
        assert!(bm.is_empty());
        assert_eq!(bm.highest(), None);
    }

    #[test]
    fn set_clear_highest() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(3);
        bm.set(17);
        bm.set(42);
        assert_eq!(bm.highest(), Some(3));

        bm.clear(3);
        assert_eq!(bm.highest(), Some(17));
        assert!(!bm.is_set(3));
        assert!(bm.is_set(17));
    }

    #[test]
    fn boundary() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(0);
        bm.set(63);
        assert_eq!(bm.highest(), Some(0));
    }

    #[test]
    fn pop_highest_drain() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(10);
        bm.set(20);
        bm.set(30);
        assert_eq!(bm.pop_highest(), Some(10));
        assert_eq!(bm.pop_highest(), Some(20));
        assert_eq!(bm.pop_highest(), Some(30));
        assert_eq!(bm.pop_highest(), None);
        assert!(bm.is_empty());
    }

    #[test]
    fn idempotent() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(5);
        bm.set(5);
        assert!(bm.is_set(5));
        bm.clear(5);
        bm.clear(5);
        assert!(bm.is_empty());
    }

    #[test]
    fn clear_all() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(1);
        bm.set(50);
        bm.clear_all();
        assert!(bm.is_empty());
    }

    #[test]
    fn wide_256() {
        let mut bm: PriorityBitmap<4> = PriorityBitmap::new();
        bm.set(0);
        bm.set(127);
        bm.set(255);
        assert_eq!(bm.highest(), Some(0));
        bm.clear(0);
        assert_eq!(bm.highest(), Some(127));
    }

    #[test]
    fn wide_4096() {
        let mut bm: PriorityBitmap<64> = PriorityBitmap::new();
        bm.set(0);
        bm.set(2048);
        bm.set(4095);
        assert_eq!(bm.highest(), Some(0));
    }

    #[test]
    fn single_group() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in (0..64).step_by(7) {
            bm.set(i);
        }
        assert_eq!(bm.highest(), Some(0));
        for i in (0..64).step_by(7) {
            assert!(bm.is_set(i));
        }
    }

    #[test]
    fn is_set_uninitialized() {
        let bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in 0..64 {
            assert!(!bm.is_set(i));
        }
    }

    #[test]
    fn set_all_64() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in 0..64 {
            bm.set(i);
        }
        assert!(!bm.is_empty());
        for i in 0..64 {
            assert!(bm.is_set(i));
        }
        assert_eq!(bm.highest(), Some(0));
        bm.clear(0);
        assert_eq!(bm.highest(), Some(1));
    }

    #[test]
    fn clear_all_64() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in 0..64 {
            bm.set(i);
        }
        for i in 0..64 {
            bm.clear(i);
        }
        assert!(bm.is_empty());
        assert_eq!(bm.highest(), None);
    }

    #[test]
    fn pop_highest_ascending_order() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in 0..64 {
            bm.set(i);
        }
        for i in 0..64 {
            assert_eq!(bm.pop_highest(), Some(i));
        }
        assert_eq!(bm.pop_highest(), None);
        assert!(bm.is_empty());
    }

    #[test]
    fn highest_descending_set() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in (0..64).rev() {
            bm.set(i);
        }
        assert_eq!(bm.highest(), Some(0));
    }

    #[test]
    fn set_and_clear_alternating() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        for i in 0..32 {
            bm.set(i * 2);
        }
        for i in 0..32 {
            assert!(bm.is_set(i * 2));
            assert!(!bm.is_set(i * 2 + 1));
        }
        for i in 0..32 {
            bm.clear(i * 2);
        }
        assert!(bm.is_empty());
    }

    #[test]
    fn clear_middle_of_three() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(10);
        bm.set(20);
        bm.set(30);
        bm.clear(20);
        assert!(!bm.is_set(20));
        assert!(bm.is_set(10));
        assert!(bm.is_set(30));
        assert_eq!(bm.highest(), Some(10));
    }

    #[test]
    fn re_set_after_clear() {
        let mut bm: PriorityBitmap<1> = PriorityBitmap::new();
        bm.set(5);
        bm.clear(5);
        assert!(bm.is_empty());
        bm.set(5);
        assert!(!bm.is_empty());
        assert!(bm.is_set(5));
        assert_eq!(bm.highest(), Some(5));
    }

    #[test]
    fn multi_group_crossing() {
        let mut bm: PriorityBitmap<4> = PriorityBitmap::new();
        bm.set(63);
        bm.set(64);
        assert_eq!(bm.highest(), Some(63));
        bm.clear(63);
        assert_eq!(bm.highest(), Some(64));
    }

    #[test]
    fn wide_drain_all_groups() {
        let mut bm: PriorityBitmap<4> = PriorityBitmap::new();
        bm.set(0);
        bm.set(65);
        bm.set(130);
        bm.set(255);
        assert_eq!(bm.pop_highest(), Some(0));
        assert_eq!(bm.pop_highest(), Some(65));
        assert_eq!(bm.pop_highest(), Some(130));
        assert_eq!(bm.pop_highest(), Some(255));
        assert_eq!(bm.pop_highest(), None);
        assert!(bm.is_empty());
    }

    #[test]
    fn clear_all_then_set() {
        let mut bm: PriorityBitmap<2> = PriorityBitmap::new();
        for i in 0..128 {
            bm.set(i);
        }
        bm.clear_all();
        assert!(bm.is_empty());
        assert_eq!(bm.highest(), None);
        bm.set(42);
        assert_eq!(bm.highest(), Some(42));
    }

    #[test]
    fn each_group_boundary() {
        let mut bm: PriorityBitmap<4> = PriorityBitmap::new();
        bm.set(0);
        bm.set(63);
        bm.set(64);
        bm.set(127);
        bm.set(128);
        bm.set(191);
        bm.set(192);
        bm.set(255);
        assert_eq!(bm.highest(), Some(0));
        for prio in [0, 63, 64, 127, 128, 191, 192, 255] {
            assert!(bm.is_set(prio));
        }
        bm.clear(0);
        assert_eq!(bm.highest(), Some(63));
        bm.clear(63);
        assert_eq!(bm.highest(), Some(64));
    }

    #[test]
    fn capacity_const() {
        assert_eq!(PriorityBitmap::<1>::CAPACITY, 64);
        assert_eq!(PriorityBitmap::<4>::CAPACITY, 256);
        assert_eq!(PriorityBitmap::<64>::CAPACITY, 4096);
    }

    #[test]
    fn is_empty_after_clear_all_with_multiple_groups() {
        let mut bm: PriorityBitmap<4> = PriorityBitmap::new();
        bm.set(1);
        bm.set(100);
        bm.set(200);
        assert!(!bm.is_empty());
        bm.clear_all();
        assert!(bm.is_empty());
        assert!(!bm.is_set(1));
        assert!(!bm.is_set(100));
        assert!(!bm.is_set(200));
    }
}
