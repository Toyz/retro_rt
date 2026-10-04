//! Puts a game's low-resolution picture on the window: letterboxed at the
//! display's aspect, scaled sharp, converted for an sRGB surface, with an
//! optional overlay drawn at the window's own resolution (a console, a HUD of
//! the port's own).

use wgpu::util::DeviceExt;

use crate::{Gpu, Picture, letterbox};

/// The shape the picture is shown at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aspect {
    /// Width over height, whatever the picture's pixel count: 4:3 for a
    /// television, whatever the console's horizontal resolution.
    Fixed(f32),
    /// The picture's own width over height: square pixels.
    Source,
    /// The whole window.
    Stretch,
}

impl Aspect {
    /// A 4:3 television.
    pub const TV: Aspect = Aspect::Fixed(4.0 / 3.0);
}

impl Default for Aspect {
    fn default() -> Self {
        Aspect::TV
    }
}

/// How the picture's pixels are scaled to the window's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Nearest texel: sharp, but at a non-integer scale some pixels come out
    /// a window pixel wider than others.
    Nearest,
    /// Each picture pixel a solid block, only the one-window-pixel seam
    /// between blocks blended. Even at any scale; plain bilinear at scale 1.
    #[default]
    SharpBilinear,
    /// Bilinear: soft.
    Linear,
}

const SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// x: 1 for sharp bilinear, 0 for the sampler as it is.
@group(0) @binding(2) var<uniform> opts: vec4<f32>;

struct P {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One triangle covering the viewport.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> P {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i >> 1u) * 4.0 - 1.0;
    var o: P;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return o;
}

// The picture's values are what the television shows; an sRGB surface
// encodes what it is given, so give it the linear value that encodes back to
// them.
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Sharp bilinear: each texel a solid block at any window size, only the seam
// between two blocks blended.
fn sharp(uv: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let texel = uv * size;
    let step = max(abs(vec2<f32>(dpdx(texel.x), dpdy(texel.y))), vec2<f32>(1e-6));
    let scale = max(vec2<f32>(1.0) / step, vec2<f32>(1.0));
    let r = vec2<f32>(0.5) - vec2<f32>(0.5) / scale;
    let d = fract(texel) - vec2<f32>(0.5);
    let f = (d - clamp(d, -r, r)) * scale + vec2<f32>(0.5);
    return (floor(texel) + f) / size;
}

fn picture(uv: vec2<f32>) -> vec3<f32> {
    let s = sharp(uv);
    let at = select(uv, s, opts.x > 0.5);
    return textureSample(src, samp, at).rgb;
}

@fragment
fn fs_srgb(p: P) -> @location(0) vec4<f32> {
    return vec4<f32>(to_linear(picture(p.uv)), 1.0);
}

@fragment
fn fs_plain(p: P) -> @location(0) vec4<f32> {
    return vec4<f32>(picture(p.uv), 1.0);
}

// An overlay: texel for pixel, blended by its straight alpha.
@fragment
fn fs_overlay_srgb(p: P) -> @location(0) vec4<f32> {
    let c = textureSample(src, samp, p.uv);
    return vec4<f32>(to_linear(c.rgb), c.a);
}

@fragment
fn fs_overlay_plain(p: P) -> @location(0) vec4<f32> {
    return textureSample(src, samp, p.uv);
}
"#;

/// A picture uploaded to the GPU, kept while its size holds.
struct Upload {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Upload {
    /// `slot` holding `p`'s pixels: written in place, or remade at a new size.
    fn put<'a>(slot: &'a mut Option<Upload>, gpu: &Gpu, p: &Picture, label: &str) -> &'a wgpu::TextureView {
        let size = wgpu::Extent3d { width: p.width.max(1), height: p.height.max(1), depth_or_array_layers: 1 };
        match slot {
            Some(u) if (u.width, u.height) == (size.width, size.height) => {
                gpu.queue.write_texture(
                    u.texture.as_image_copy(),
                    &p.rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size.width * 4),
                        rows_per_image: Some(size.height),
                    },
                    size,
                );
            }
            _ => {
                let texture = gpu.device.create_texture_with_data(
                    &gpu.queue,
                    &wgpu::TextureDescriptor {
                        label: Some(label),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &p.rgba,
                );
                let view = texture.create_view(&Default::default());
                *slot = Some(Upload { texture, view, width: size.width, height: size.height });
            }
        }
        &slot.as_ref().unwrap().view
    }
}

/// Draws a picture into the window's surface. Made once for the surface's
/// format; [`Presenter::aspect`] and [`Presenter::filter`] may change at any
/// time.
pub struct Presenter {
    /// The shape the picture is shown at.
    pub aspect: Aspect,
    /// How the picture is scaled.
    pub filter: Filter,
    /// What the bars around the picture are filled with, RGB 0-1.
    pub border: [f64; 3],
    pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    opts: wgpu::Buffer,
    picture: Option<Upload>,
    overlay: Option<Upload>,
}

impl Presenter {
    /// A presenter drawing into `format` (the surface's format, or the view
    /// format the frame is drawn through). An sRGB format gets the
    /// conversion from display-encoded values; a plain one gets them as
    /// they are.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Presenter {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[
                texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let srgb = format.is_srgb();
        let main = pipeline("present", if srgb { "fs_srgb" } else { "fs_plain" }, None);
        let overlay = pipeline(
            "overlay",
            if srgb { "fs_overlay_srgb" } else { "fs_overlay_plain" },
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("present linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("present nearest"), ..Default::default() });
        let opts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present options"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Presenter {
            aspect: Aspect::default(),
            filter: Filter::default(),
            border: [0.0; 3],
            pipeline: main,
            overlay_pipeline: overlay,
            layout,
            linear,
            nearest,
            opts,
            picture: None,
            overlay: None,
        }
    }

    /// Draws `picture` from the CPU into `target`, `width` x `height`, and
    /// `overlay` over it. The picture's texture is kept and rewritten each
    /// call while its size holds.
    pub fn present_picture(
        &mut self,
        gpu: &Gpu,
        picture: &Picture,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        overlay: Option<&Picture>,
    ) -> wgpu::CommandBuffer {
        let mut slot = self.picture.take();
        let view = Upload::put(&mut slot, gpu, picture, "picture");
        let out = self.present_view(gpu, view, (picture.width, picture.height), target, width, height, overlay);
        self.picture = slot;
        out
    }

    /// Draws `source` (a texture a renderer drew into, `source_size` texels)
    /// into `target`, `width` x `height`, and `overlay` over it.
    #[allow(clippy::too_many_arguments)]
    pub fn present_view(
        &mut self,
        gpu: &Gpu,
        source: &wgpu::TextureView,
        source_size: (u32, u32),
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        overlay: Option<&Picture>,
    ) -> wgpu::CommandBuffer {
        let device = &gpu.device;
        let sharp = if self.filter == Filter::SharpBilinear { 1.0f32 } else { 0.0 };
        let bytes: Vec<u8> = [sharp, 0.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        gpu.queue.write_buffer(&self.opts, 0, &bytes);
        let sampler = if self.filter == Filter::Nearest { &self.nearest } else { &self.linear };
        let group = self.group(device, source, sampler);
        let [x, y, w, h] = match self.aspect {
            Aspect::Fixed(a) => letterbox(width, height, a),
            Aspect::Source => letterbox(width, height, source_size.0.max(1) as f32 / source_size.1.max(1) as f32),
            Aspect::Stretch => [0.0, 0.0, width.max(1) as f32, height.max(1) as f32],
        };
        let overlay_view = overlay
            .filter(|o| o.width > 0 && o.height > 0)
            .map(|o| (Upload::put(&mut self.overlay, gpu, o, "overlay").clone(), o.width, o.height));
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let [r, g, b] = self.border;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
            if let Some((view, ow, oh)) = &overlay_view {
                let og = self.group(device, view, &self.nearest);
                let (ow, oh) = ((*ow).min(width.max(1)) as f32, (*oh).min(height.max(1)) as f32);
                pass.set_viewport(0.0, 0.0, ow, oh, 0.0, 1.0);
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_bind_group(0, &og, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder.finish()
    }

    fn group(&self, device: &wgpu::Device, view: &wgpu::TextureView, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.opts.as_entire_binding() },
            ],
        })
    }
}
