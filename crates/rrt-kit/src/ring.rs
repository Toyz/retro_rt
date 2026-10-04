//! A fixed-capacity history that overwrites its oldest.

/// The newest `capacity` values pushed, oldest first. Pushing past capacity
/// drops the oldest. Never reallocates after it is made.
#[derive(Clone, Debug)]
pub struct Ring<T> {
    buf: Vec<T>,
    capacity: usize,
    /// Where the next push goes once full; the oldest is here.
    head: usize,
}

impl<T> Ring<T> {
    /// An empty ring holding up to `capacity` values.
    ///
    /// # Panics
    ///
    /// When `capacity` is 0.
    pub fn new(capacity: usize) -> Ring<T> {
        assert!(capacity > 0, "a ring holds at least one value");
        Ring { buf: Vec::with_capacity(capacity), capacity, head: 0 }
    }

    /// Values held, at most [`Ring::capacity`].
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Nothing pushed yet.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The most values it holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Adds `value` as the newest; returns the oldest it pushed out, if full.
    pub fn push(&mut self, value: T) -> Option<T> {
        if self.buf.len() < self.capacity {
            self.buf.push(value);
            return None;
        }
        let old = std::mem::replace(&mut self.buf[self.head], value);
        self.head = (self.head + 1) % self.capacity;
        Some(old)
    }

    /// The `i`th value, 0 the oldest.
    pub fn get(&self, i: usize) -> Option<&T> {
        (i < self.buf.len()).then(|| &self.buf[(self.head + i) % self.buf.len()])
    }

    /// The newest value.
    pub fn newest(&self) -> Option<&T> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }

    /// The oldest value.
    pub fn oldest(&self) -> Option<&T> {
        self.get(0)
    }

    /// Every value, oldest first. Allocates nothing.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        let (newer, older) = self.buf.split_at(self.head.min(self.buf.len()));
        older.iter().chain(newer.iter())
    }

    /// Empties it, keeping the allocation.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_capacity_the_oldest_goes() {
        let mut r = Ring::new(3);
        assert_eq!(r.push(1), None);
        r.push(2);
        r.push(3);
        assert_eq!(r.push(4), Some(1));
        assert_eq!(r.push(5), Some(2));
        assert_eq!(r.iter().copied().collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!((r.oldest(), r.newest(), r.get(1), r.get(3)), (Some(&3), Some(&5), Some(&4), None));
        assert_eq!(r.iter().rev().copied().collect::<Vec<_>>(), [5, 4, 3]);
    }

    #[test]
    fn below_capacity_it_is_in_push_order() {
        let mut r = Ring::new(4);
        r.push('a');
        r.push('b');
        assert_eq!(r.iter().collect::<String>(), "ab");
        assert_eq!(r.len(), 2);
        r.clear();
        assert!(r.is_empty() && r.newest().is_none());
    }
}
