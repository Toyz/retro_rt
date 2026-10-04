//! A generational arena: values in slots, reached by handles that go stale
//! when their slot is freed and reused.

/// A slot's index and the generation it was filled in. A handle to a value
/// that has been removed finds nothing, even after its slot holds another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    /// The slot.
    pub index: u32,
    /// The slot's generation when the value went in.
    pub generation: u32,
}

#[derive(Clone, Debug)]
enum Slot<T> {
    Full {
        generation: u32,
        value: T,
    },
    /// Free: the generation the next value gets, and the next free slot.
    Free {
        generation: u32,
        next: Option<u32>,
    },
}

/// Values reached by [`Handle`]. Insert and remove are O(1) and reuse freed
/// slots; iteration is in slot order.
#[derive(Clone, Debug)]
pub struct Slab<T> {
    slots: Vec<Slot<T>>,
    free: Option<u32>,
    len: usize,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Slab::new()
    }
}

impl<T> Slab<T> {
    /// An empty slab.
    pub fn new() -> Slab<T> {
        Slab { slots: Vec::new(), free: None, len: 0 }
    }

    /// Values held.
    pub fn len(&self) -> usize {
        self.len
    }

    /// No values held.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Stores `value` in a freed slot, or a new one, and returns its handle.
    ///
    /// # Panics
    ///
    /// With more than `u32::MAX` slots.
    pub fn insert(&mut self, value: T) -> Handle {
        self.len += 1;
        if let Some(index) = self.free {
            let slot = &mut self.slots[index as usize];
            let Slot::Free { generation, next } = *slot else { unreachable!("the free list holds free slots") };
            self.free = next;
            *slot = Slot::Full { generation, value };
            return Handle { index, generation };
        }
        let index = u32::try_from(self.slots.len()).expect("more than u32::MAX slots");
        self.slots.push(Slot::Full { generation: 0, value });
        Handle { index, generation: 0 }
    }

    /// The value `h` names, if it is still there.
    pub fn get(&self, h: Handle) -> Option<&T> {
        match self.slots.get(h.index as usize)? {
            Slot::Full { generation, value } if *generation == h.generation => Some(value),
            _ => None,
        }
    }

    /// The value `h` names, mutably, if it is still there.
    pub fn get_mut(&mut self, h: Handle) -> Option<&mut T> {
        match self.slots.get_mut(h.index as usize)? {
            Slot::Full { generation, value } if *generation == h.generation => Some(value),
            _ => None,
        }
    }

    /// Whether `h` still names a value.
    pub fn contains(&self, h: Handle) -> bool {
        self.get(h).is_some()
    }

    /// Takes out the value `h` names; its slot is freed and every handle to
    /// it goes stale. None when it was already gone.
    pub fn remove(&mut self, h: Handle) -> Option<T> {
        let slot = self.slots.get_mut(h.index as usize)?;
        match slot {
            Slot::Full { generation, .. } if *generation == h.generation => {
                let free = Slot::Free { generation: generation.wrapping_add(1), next: self.free };
                let Slot::Full { value, .. } = std::mem::replace(slot, free) else { unreachable!() };
                self.free = Some(h.index);
                self.len -= 1;
                Some(value)
            }
            _ => None,
        }
    }

    /// Every value with its handle, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| match s {
            Slot::Full { generation, value } => Some((Handle { index: i as u32, generation: *generation }, value)),
            Slot::Free { .. } => None,
        })
    }

    /// Every value mutably with its handle, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| match s {
            Slot::Full { generation, value } => Some((Handle { index: i as u32, generation: *generation }, value)),
            Slot::Free { .. } => None,
        })
    }

    /// Removes every value for which `keep` is false.
    pub fn retain(&mut self, mut keep: impl FnMut(Handle, &mut T) -> bool) {
        let gone: Vec<Handle> = self.iter_mut().filter_map(|(h, v)| (!keep(h, v)).then_some(h)).collect();
        for h in gone {
            self.remove(h);
        }
    }

    /// Removes everything; every handle goes stale.
    pub fn clear(&mut self) {
        let handles: Vec<Handle> = self.iter().map(|(h, _)| h).collect();
        for h in handles {
            self.remove(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_handle_finds_nothing_after_its_slot_is_reused() {
        let mut s = Slab::new();
        let a = s.insert("a");
        assert_eq!(s.remove(a), Some("a"));
        let b = s.insert("b");
        assert_eq!(b.index, a.index, "the slot is reused");
        assert_ne!(b.generation, a.generation);
        assert_eq!(s.get(a), None);
        assert_eq!(s.remove(a), None);
        assert_eq!(s.get(b), Some(&"b"));
    }

    #[test]
    fn insert_get_iterate_and_retain() {
        let mut s = Slab::new();
        let h: Vec<Handle> = (0..5).map(|i| s.insert(i)).collect();
        *s.get_mut(h[2]).unwrap() = 20;
        assert_eq!(s.len(), 5);
        s.retain(|_, v| *v % 2 == 0);
        assert_eq!(s.iter().map(|(_, v)| *v).collect::<Vec<_>>(), [0, 20, 4]);
        assert!(!s.contains(h[1]) && s.contains(h[2]));
        s.clear();
        assert!(s.is_empty() && !s.contains(h[0]));
        let again = s.insert(9);
        assert!(again.index < 5, "cleared slots are reused");
    }
}
