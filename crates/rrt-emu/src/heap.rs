//! A first-fit heap for HLE `malloc` and `free`.

/// A first-fit allocator over `[base, base + size)` with coalescing, standing
/// in for a console library's. Blocks are aligned to `align` bytes. A game
/// that relies on freed blocks being reused (most do: hwtr's lives in 2 MB
/// only because they are) needs one that reuses them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heap {
    /// (address, size, in use), in address order, covering the whole heap.
    pub blocks: Vec<(u32, u32, bool)>,
    /// Every block's alignment, a power of two.
    pub align: u32,
}

impl Heap {
    /// A heap over `size` bytes from `base`, blocks aligned to `align` (8 on
    /// the PS1, 16 on the PS2).
    ///
    /// # Panics
    ///
    /// When `align` is not a power of two.
    pub fn new(base: u32, size: u32, align: u32) -> Heap {
        assert!(align.is_power_of_two(), "alignment {align}");
        let start = base.next_multiple_of(align);
        let len = size.saturating_sub(start - base) & !(align - 1);
        Heap { blocks: vec![(start, len, false)], align }
    }

    /// A block of at least `n` bytes; 0 when nothing fits.
    pub fn alloc(&mut self, n: u32) -> u32 {
        let n = n.max(1).next_multiple_of(self.align);
        let Some(i) = self.blocks.iter().position(|b| !b.2 && b.1 >= n) else { return 0 };
        let (at, size, _) = self.blocks[i];
        self.blocks[i] = (at, n, true);
        if size > n {
            self.blocks.insert(i + 1, (at + n, size - n, false));
        }
        at
    }

    /// Frees the block at `at`, joining it with free neighbours. Anything
    /// that is not a block in use is ignored, as most consoles' `free` does.
    pub fn free(&mut self, at: u32) {
        let Some(i) = self.blocks.iter().position(|b| b.0 == at && b.2) else { return };
        self.blocks[i].2 = false;
        if i + 1 < self.blocks.len() && !self.blocks[i + 1].2 {
            self.blocks[i].1 += self.blocks[i + 1].1;
            self.blocks.remove(i + 1);
        }
        if i > 0 && !self.blocks[i - 1].2 {
            self.blocks[i - 1].1 += self.blocks[i].1;
            self.blocks.remove(i);
        }
    }

    /// Bytes in use.
    pub fn used(&self) -> u32 {
        self.blocks.iter().filter(|b| b.2).map(|b| b.1).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_blocks_are_reused_and_joined() {
        let mut h = Heap::new(0x1004, 0x100, 8);
        let a = h.alloc(10);
        assert_eq!(a, 0x1008, "aligned up");
        let b = h.alloc(16);
        let c = h.alloc(8);
        assert_eq!((b, c), (0x1018, 0x1028));
        h.free(b);
        assert_eq!(h.alloc(16), b, "first fit reuses the hole");
        h.free(a);
        h.free(b);
        assert_eq!(h.blocks[0], (0x1008, 0x20, false), "neighbours joined");
        assert_eq!(h.used(), 8);
        assert_eq!(h.alloc(0x1000), 0, "nothing fits");
        h.free(0xdead);
    }
}
