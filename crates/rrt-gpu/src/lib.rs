//! wgpu for retro games.
//!
//! [`Gpu`] is the instance, adapter, device and queue together, opened with no
//! window ([`Gpu::headless`], for `--shot` and tests) or for a window's
//! surface ([`Gpu::for_surface`], which `rrt-app` calls). [`Target`] is an
//! offscreen colour target whose pixels come back as an
//! [`rrt_image::Picture`]; [`DepthBuffer`] follows a target's size.
//! [`GrowBuffer`] is a GPU buffer rewritten each frame that grows to
//! whatever the frame needs and is otherwise reused.
//! [`Presenter`] puts a low-resolution picture - a [`Picture`] from the CPU or
//! a texture a renderer drew into - on the window, letterboxed at a fixed
//! aspect and scaled sharp.
//!
//! Colour: a game's pixels are display-encoded values (what the television
//! showed), held in plain `Rgba8Unorm`. Only the presenter converts, and only
//! for an sRGB surface. See `docs/conventions/colour.md`.
//!
//! The wgpu and glam this crate is built on are re-exported ([`wgpu`],
//! [`glam`]) so a game names the same versions.

pub mod buffer;
pub mod present;
pub mod target;

pub use buffer::GrowBuffer;
pub use glam;
pub use present::{Aspect, Filter, Presenter};
pub use rrt_image::Picture;
pub use target::{DepthBuffer, Target, TargetOptions, read_texture};
pub use wgpu;

use std::fmt;

/// Something the GPU would not do: no adapter, no device, a surface the
/// adapter cannot draw to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(e: impl fmt::Display) -> Error {
    Error(e.to_string())
}

/// The instance, adapter, device and queue a game draws with.
pub struct Gpu {
    /// The wgpu instance; a window's surface is made from it.
    pub instance: wgpu::Instance,
    /// The adapter the device was opened on.
    pub adapter: wgpu::Adapter,
    /// The device.
    pub device: wgpu::Device,
    /// The device's queue.
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// The high-performance adapter with no window: for `--shot`, tools and
    /// tests. Fails on a machine with no adapter at all.
    pub fn headless() -> Result<Gpu, Error> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        Gpu::open(instance, None)
    }

    /// An adapter that can draw to `surface`, made from `instance`.
    pub fn for_surface(instance: wgpu::Instance, surface: &wgpu::Surface<'_>) -> Result<Gpu, Error> {
        Gpu::open(instance, Some(surface))
    }

    fn open(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Gpu, Error> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: surface,
            apply_limit_buckets: false,
        }))
        .map_err(err)?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor { label: Some("rrt"), ..Default::default() }),
        )
        .map_err(err)?;
        Ok(Gpu { instance, adapter, device, queue })
    }

    /// Waits until the queue's submitted work is done.
    pub fn wait(&self) -> Result<(), Error> {
        self.device.poll(wgpu::PollType::wait_indefinitely()).map(|_| ()).map_err(err)
    }
}

/// The projection retro_rt renderers draw with: right-handed, `fov_y` radians
/// tall, depth mapped to wgpu's clip range of 0 (near) to 1 (far). glam files
/// this convention under its `directx` name; Vulkan and Metal share it.
pub fn projection(fov_y: f32, aspect: f32, near: f32, far: f32) -> glam::Mat4 {
    glam::camera::rh::proj::directx::perspective(fov_y, aspect, near, far)
}

/// A surface format that stores a shader's values as they are (not sRGB):
/// for drawing display-encoded colours straight to a window. The first of
/// `formats` when every one is sRGB.
///
/// # Panics
///
/// When `formats` is empty.
pub fn plain_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(formats[0])
}

/// The rectangle a picture of shape `aspect` (width over height) fills in a
/// `width` x `height` window, centred, bars on the long sides: x, y, w, h in
/// pixels.
pub fn letterbox(width: u32, height: u32, aspect: f32) -> [f32; 4] {
    let (w, h) = (width.max(1) as f32, height.max(1) as f32);
    let (vw, vh) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    [(w - vw) / 2.0, (h - vh) / 2.0, vw, vh]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_bars_the_long_side() {
        assert_eq!(letterbox(1600, 900, 4.0 / 3.0), [200.0, 0.0, 1200.0, 900.0]);
        assert_eq!(letterbox(800, 900, 4.0 / 3.0), [0.0, 150.0, 800.0, 600.0]);
        assert_eq!(letterbox(640, 480, 4.0 / 3.0), [0.0, 0.0, 640.0, 480.0]);
    }

    #[test]
    fn projection_maps_near_to_0_and_far_to_1() {
        let p = projection(1.0, 1.0, 1.0, 100.0);
        let near = p * glam::Vec4::new(0.0, 0.0, -1.0, 1.0);
        let far = p * glam::Vec4::new(0.0, 0.0, -100.0, 1.0);
        assert!((near.z / near.w).abs() < 1e-6);
        assert!((far.z / far.w - 1.0).abs() < 1e-6);
    }
}
