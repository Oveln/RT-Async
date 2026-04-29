/// Priority value with **inverted** ordering: lower numeric value = higher priority.
///
/// Priority 0 is the highest; Priority 63 is the lowest in a 64-level system.
/// Use [`is_lower_than`] / [`is_higher_than`] instead of raw `<` / `>` for
/// readability when comparing scheduling priorities.
///
/// [`is_lower_than`]: Priority::is_lower_than
/// [`is_higher_than`]: Priority::is_higher_than
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Priority(usize);

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        other.0.cmp(&self.0)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Priority {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    pub fn to_usize(&self) -> usize {
        self.0
    }

    pub fn set(&mut self, prio: usize) {
        self.0 = prio
    }

    pub fn is_lower_than(&self, other: &Priority) -> bool {
        self < other
    }

    pub fn is_higher_than(&self, other: &Priority) -> bool {
        self > other
    }
}
