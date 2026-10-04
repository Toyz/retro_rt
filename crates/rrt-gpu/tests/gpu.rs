//! The GPU paths, run on whatever adapter the machine has (Mesa's lavapipe in
//! CI). With no adapter at all each test says so and passes: the logic they
//! hold is still covered by the unit tests, and a missing GPU is the
//! machine's state, not a fault.

use rrt_gpu::{Aspect, Filter, Gpu, Picture, Presenter, Target, TargetOptions, wgpu};

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("no GPU adapter, skipping: {e}");
            None
        }
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A 2 x 2 picture, red left column and green right, presented square into a
/// 4 x 2 target with nearest filtering: a black bar each side.
#[test]
fn a_picture_is_letterboxed_into_the_middle() {
    let Some(gpu) = gpu() else { return };
    let mut picture = Picture::filled(2, 2, RED);
    picture.fill_rect(1, 0, 1, 2, GREEN);
    let target = Target::new(&gpu.device, 4, 2);
    let mut presenter = Presenter::new(&gpu.device, Target::FORMAT);
    presenter.aspect = Aspect::Fixed(1.0);
    presenter.filter = Filter::Nearest;
    let c = presenter.present_picture(&gpu, &picture, &target.view, 4, 2, None);
    gpu.queue.submit([c]);
    let out = target.read_back(&gpu);
    for y in 0..2 {
        let row: Vec<[u8; 4]> = (0..4).map(|x| out.get(x, y).unwrap()).collect();
        assert_eq!(row, [BLACK, RED, GREEN, BLACK], "row {y}");
    }
}

/// Presented into an sRGB target, a display-encoded value is stored as that
/// same value: the presenter's conversion to linear and the target's
/// encoding cancel.
#[test]
fn display_encoded_values_survive_an_srgb_target() {
    let Some(gpu) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = Target::with_options(&gpu.device, TargetOptions { format, ..TargetOptions::new(4, 4) });
    let mut presenter = Presenter::new(&gpu.device, format);
    presenter.aspect = Aspect::Stretch;
    presenter.filter = Filter::Nearest;
    for v in [0u8, 1, 10, 64, 128, 200, 255] {
        let picture = Picture::filled(4, 4, [v, v, v, 255]);
        gpu.queue.submit([presenter.present_picture(&gpu, &picture, &target.view, 4, 4, None)]);
        let got = target.read_back(&gpu).get(2, 2).unwrap();
        assert!(got[..3].iter().all(|c| c.abs_diff(v) <= 1), "{v} came back as {got:?}");
    }
}

/// An overlay's texels go over the picture at the window's top left,
/// blended by their alpha.
#[test]
fn an_overlay_draws_over_the_top_left() {
    let Some(gpu) = gpu() else { return };
    let target = Target::new(&gpu.device, 4, 4);
    let mut presenter = Presenter::new(&gpu.device, Target::FORMAT);
    presenter.aspect = Aspect::Stretch;
    presenter.filter = Filter::Nearest;
    let picture = Picture::filled(4, 4, RED);
    let mut overlay = Picture::filled(2, 2, [0, 0, 0, 0]);
    overlay.set(0, 0, GREEN);
    gpu.queue.submit([presenter.present_picture(&gpu, &picture, &target.view, 4, 4, Some(&overlay))]);
    let out = target.read_back(&gpu);
    assert_eq!(out.get(0, 0), Some(GREEN));
    assert_eq!(out.get(1, 1), Some(RED), "a transparent overlay texel leaves the picture");
    assert_eq!(out.get(3, 3), Some(RED));
}

/// A multisampled target with depth: a cleared pass resolves into the
/// texture that is read back.
#[test]
fn a_multisampled_target_resolves() {
    let Some(gpu) = gpu() else { return };
    let options =
        TargetOptions { samples: 4, depth: Some(wgpu::TextureFormat::Depth32Float), ..TargetOptions::new(8, 8) };
    let target = Target::with_options(&gpu.device, options);
    assert_eq!(target.multisample().count, 4);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let blue = wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(target.color_attachment(wgpu::LoadOp::Clear(blue)))],
            depth_stencil_attachment: target.depth_attachment(1.0),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    gpu.queue.submit([encoder.finish()]);
    assert_eq!(target.read_back(&gpu).get(4, 4), Some([0, 0, 255, 255]));
}

#[test]
fn a_resized_target_keeps_its_options() {
    let Some(gpu) = gpu() else { return };
    let options = TargetOptions { samples: 4, usage: wgpu::TextureUsages::COPY_DST, ..TargetOptions::new(8, 8) };
    let mut target = Target::with_options(&gpu.device, options);
    target.resize(&gpu.device, 16, 0);
    assert_eq!((target.width, target.height, target.options.samples), (16, 1, 4));
    assert!(target.texture.usage().contains(wgpu::TextureUsages::COPY_DST));
}

/// Sharp bilinear at 2.5x: two texels over five pixels. Each texel is a solid
/// block and only the seam pixel, which straddles both, is blended - half
/// and half. Plain bilinear would blend three of the five.
#[test]
fn sharp_bilinear_blends_only_the_seam() {
    let Some(gpu) = gpu() else { return };
    let mut picture = Picture::filled(2, 1, RED);
    picture.set(1, 0, GREEN);
    let target = Target::new(&gpu.device, 5, 1);
    let mut presenter = Presenter::new(&gpu.device, Target::FORMAT);
    presenter.aspect = Aspect::Stretch;
    presenter.filter = Filter::SharpBilinear;
    gpu.queue.submit([presenter.present_picture(&gpu, &picture, &target.view, 5, 1, None)]);
    let out = target.read_back(&gpu);
    let px: Vec<[u8; 4]> = (0..5).map(|x| out.get(x, 0).unwrap()).collect();
    assert_eq!(&px[..2], [RED, RED]);
    assert_eq!(&px[3..], [GREEN, GREEN]);
    let seam = px[2];
    assert!(seam[0].abs_diff(128) <= 2 && seam[1].abs_diff(128) <= 2 && seam[2] == 0, "seam {seam:?}");

    presenter.filter = Filter::Linear;
    gpu.queue.submit([presenter.present_picture(&gpu, &picture, &target.view, 5, 1, None)]);
    let soft = target.read_back(&gpu);
    let blended = (0..5).filter(|&x| ![RED, GREEN].contains(&soft.get(x, 0).unwrap())).count();
    assert!(blended >= 3, "plain bilinear blends {blended} of 5");
}

/// Written larger it grows to the next power of two; written smaller it keeps
/// the same buffer; what was written reads back.
#[test]
fn a_grow_buffer_grows_by_powers_of_two_and_is_reused() {
    use rrt_gpu::GrowBuffer;
    let Some(gpu) = gpu() else { return };
    let mut b = GrowBuffer::new("test", wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC);
    assert_eq!((b.capacity(), b.len()), (0, 0));
    assert!(b.slice().is_none());
    b.write(&gpu.device, &gpu.queue, &[1u8; 100]);
    assert_eq!((b.capacity(), b.len()), (GrowBuffer::MIN, 100));
    b.write(&gpu.device, &gpu.queue, &[2u8; 1000]);
    assert_eq!(b.capacity(), 1024);
    let first = b.buffer().unwrap().clone();
    let data: Vec<u8> = (0..64).collect();
    b.write(&gpu.device, &gpu.queue, &data);
    assert_eq!(b.buffer().unwrap(), &first, "a smaller frame reuses the buffer");
    assert_eq!(b.len(), 64);

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(b.buffer().unwrap(), 0, &readback, 0, 64);
    gpu.queue.submit([enc.finish()]);
    readback.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
    gpu.wait().unwrap();
    assert_eq!(&*readback.get_mapped_range(..).unwrap(), &data[..]);
}
