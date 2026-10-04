//! Offscreen targets and depth buffers.

use rrt_image::Picture;

use crate::Gpu;

/// How a [`Target`] is made. `TargetOptions::new(w, h)` gives a plain RGBA8
/// target; set the rest as needed.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetOptions {
    /// The label wgpu reports the textures by.
    pub label: &'static str,
    /// Pixels across.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// The colour format. [`Target::read_back`] wants a 4-byte one.
    pub format: wgpu::TextureFormat,
    /// Usages beyond the ones every target has (`RENDER_ATTACHMENT |
    /// TEXTURE_BINDING | COPY_SRC`): `COPY_DST` to upload into it,
    /// `STORAGE_BINDING` for a compute pass.
    pub usage: wgpu::TextureUsages,
    /// Other formats the texture may be viewed as, such as the sRGB twin of
    /// [`TargetOptions::format`].
    pub view_formats: Vec<wgpu::TextureFormat>,
    /// Samples a pixel. 1 is no multisampling; above 1 the target also holds
    /// a multisampled texture that passes draw into and that resolves into
    /// [`Target::texture`] ([`Target::color_attachment`] wires it).
    pub samples: u32,
    /// A depth attachment of this format, the target's size and sample
    /// count, or none.
    pub depth: Option<wgpu::TextureFormat>,
}

impl TargetOptions {
    /// A `width` x `height` target in [`Target::FORMAT`], single-sampled,
    /// with no depth.
    pub fn new(width: u32, height: u32) -> TargetOptions {
        TargetOptions {
            label: "target",
            width,
            height,
            format: Target::FORMAT,
            usage: wgpu::TextureUsages::empty(),
            view_formats: Vec::new(),
            samples: 1,
            depth: None,
        }
    }
}

/// An offscreen colour target: rendered into, sampled by the presenter, and
/// read back as a [`Picture`]. Optionally multisampled, optionally with its
/// own depth attachment ([`TargetOptions`]).
pub struct Target {
    /// The single-sampled texture: what is sampled and read back, and what a
    /// multisampled target resolves into.
    pub texture: wgpu::Texture,
    /// A view of [`Target::texture`].
    pub view: wgpu::TextureView,
    /// Pixels across.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// What the target was made with; [`Target::resize`] keeps it.
    pub options: TargetOptions,
    msaa: Option<wgpu::TextureView>,
    depth: Option<wgpu::TextureView>,
}

impl Target {
    /// The plain RGBA8 format a game's frame buffer is kept in: the values
    /// are display-encoded, so not sRGB.
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// A `width` x `height` target in [`Target::FORMAT`].
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Target {
        Target::with_options(device, TargetOptions::new(width, height))
    }

    /// A target made as `options` say. A size of 0 is taken as 1.
    pub fn with_options(device: &wgpu::Device, mut options: TargetOptions) -> Target {
        options.width = options.width.max(1);
        options.height = options.height.max(1);
        options.samples = options.samples.max(1);
        let size = wgpu::Extent3d { width: options.width, height: options.height, depth_or_array_layers: 1 };
        let make = |label: &str, samples: u32, format, usage, view_formats: &[wgpu::TextureFormat]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let texture = make(
            options.label,
            1,
            options.format,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | options.usage,
            &options.view_formats,
        );
        let view = texture.create_view(&Default::default());
        let msaa = (options.samples > 1).then(|| {
            make(
                "target msaa",
                options.samples,
                options.format,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                &options.view_formats,
            )
            .create_view(&Default::default())
        });
        let depth = options.depth.map(|f| {
            make("target depth", options.samples, f, wgpu::TextureUsages::RENDER_ATTACHMENT, &[])
                .create_view(&Default::default())
        });
        Target { texture, view, width: options.width, height: options.height, options, msaa, depth }
    }

    /// Remakes the target at a new size with the same options; nothing when
    /// the size is unchanged. The contents are lost.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if (width.max(1), height.max(1)) != (self.width, self.height) {
            *self = Target::with_options(device, TargetOptions { width, height, ..self.options.clone() });
        }
    }

    /// The colour attachment a pass draws into this target with: the
    /// multisampled texture resolving into [`Target::texture`], or the
    /// texture itself. `load` is what the pass starts from.
    pub fn color_attachment(&self, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPassColorAttachment<'_> {
        let (view, resolve_target) = match &self.msaa {
            Some(m) => (m, Some(&self.view)),
            None => (&self.view, None),
        };
        wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        }
    }

    /// The depth attachment, cleared to `clear` (1.0 for a `Less` test) and
    /// kept; None when the target was made without depth.
    pub fn depth_attachment(&self, clear: f32) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
        self.depth.as_ref().map(|view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        })
    }

    /// The pipeline state a pipeline drawing into this target needs to match:
    /// its sample count.
    pub fn multisample(&self) -> wgpu::MultisampleState {
        wgpu::MultisampleState { count: self.options.samples, ..Default::default() }
    }

    /// The target's pixels, waiting for the GPU. Alpha is forced to 255:
    /// a retro frame buffer's alpha is the hardware's, not an opacity.
    pub fn read_back(&self, gpu: &Gpu) -> Picture {
        let mut rgba = read_texture(gpu, &self.texture, self.width, self.height);
        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        Picture { width: self.width, height: self.height, rgba }
    }
}

/// The first `width` x `height` texels of a 4-byte-per-texel `texture`, rows
/// top first with no padding. Waits for the GPU.
///
/// # Panics
///
/// When the texture lacks `COPY_SRC`, or the GPU is lost while waiting.
pub fn read_texture(gpu: &Gpu, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
    let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.expect("map readback"));
    gpu.wait().expect("poll");
    let data = readback.get_mapped_range(..).expect("mapped range");
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&data[y * row as usize..y * row as usize + width as usize * 4]);
    }
    out
}

/// A depth texture remade whenever the size it is asked for changes.
pub struct DepthBuffer {
    /// The depth format.
    pub format: wgpu::TextureFormat,
    current: Option<(wgpu::TextureView, u32, u32)>,
}

impl Default for DepthBuffer {
    fn default() -> Self {
        DepthBuffer::new(wgpu::TextureFormat::Depth32Float)
    }
}

impl DepthBuffer {
    /// A depth buffer in `format`, made on first use.
    pub fn new(format: wgpu::TextureFormat) -> DepthBuffer {
        DepthBuffer { format, current: None }
    }

    /// The view for a `width` x `height` pass, remade if the size changed.
    pub fn view(&mut self, device: &wgpu::Device, width: u32, height: u32) -> &wgpu::TextureView {
        let (width, height) = (width.max(1), height.max(1));
        if self.current.as_ref().is_none_or(|c| (c.1, c.2) != (width, height)) {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.current = Some((t.create_view(&Default::default()), width, height));
        }
        &self.current.as_ref().unwrap().0
    }
}
