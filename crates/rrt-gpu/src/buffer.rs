//! A GPU buffer rewritten every frame and reused while it is big enough.

/// A buffer for data that changes every frame (the moving models' vertices,
/// a frame's uniforms): [`GrowBuffer::write`] puts the bytes in through the
/// queue, remaking the buffer only when they outgrow it, at the next power of
/// two so a slowly growing frame remakes it rarely. It never shrinks.
pub struct GrowBuffer {
    label: &'static str,
    usage: wgpu::BufferUsages,
    buffer: Option<wgpu::Buffer>,
    len: u64,
}

impl GrowBuffer {
    /// The smallest buffer made, in bytes.
    pub const MIN: u64 = 256;

    /// An empty buffer for `usage` (`VERTEX`, `INDEX`, `UNIFORM`,
    /// `STORAGE`); `COPY_DST` is added. Nothing is made until the first
    /// write.
    pub fn new(label: &'static str, usage: wgpu::BufferUsages) -> GrowBuffer {
        GrowBuffer { label, usage: usage | wgpu::BufferUsages::COPY_DST, buffer: None, len: 0 }
    }

    /// Writes `bytes` from the start, remaking the buffer first if they do not
    /// fit. The write lands with the queue's next submit.
    ///
    /// # Panics
    ///
    /// When `bytes.len()` is not a multiple of 4, as `Queue::write_buffer`
    /// requires.
    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) -> &wgpu::Buffer {
        let need = bytes.len() as u64;
        if self.buffer.as_ref().is_none_or(|b| b.size() < need) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: need.max(Self::MIN).next_power_of_two(),
                usage: self.usage,
                mapped_at_creation: false,
            }));
        }
        let buffer = self.buffer.as_ref().unwrap();
        if !bytes.is_empty() {
            queue.write_buffer(buffer, 0, bytes);
        }
        self.len = need;
        buffer
    }

    /// The buffer, once written.
    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }

    /// Bytes the last write put in: what to bind or draw from.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// The last write was empty, or there has been none.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The written bytes as a slice of the buffer, to bind or set as a
    /// vertex buffer. None before the first write or after an empty one.
    pub fn slice(&self) -> Option<wgpu::BufferSlice<'_>> {
        self.buffer.as_ref().filter(|_| self.len > 0).map(|b| b.slice(..self.len))
    }

    /// Bytes the buffer holds; 0 before the first write.
    pub fn capacity(&self) -> u64 {
        self.buffer.as_ref().map_or(0, |b| b.size())
    }
}
