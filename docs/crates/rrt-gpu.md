---
title: rrt-gpu, wgpu for retro games
status: solid
crates: rrt-gpu
covers: rrt_gpu::GrowBuffer, rrt_gpu::GrowBuffer::write, rrt_gpu::GrowBuffer::slice, rrt_gpu::GrowBuffer::capacity, rrt_gpu::Gpu, rrt_gpu::Gpu::headless, rrt_gpu::Gpu::for_surface, rrt_gpu::Gpu::wait, rrt_gpu::Target, rrt_gpu::TargetOptions, rrt_gpu::Target::color_attachment, rrt_gpu::Target::depth_attachment, rrt_gpu::Target::multisample, rrt_gpu::Target::resize, rrt_gpu::Target::read_back, rrt_gpu::read_texture, rrt_gpu::DepthBuffer, rrt_gpu::Presenter, rrt_gpu::Presenter::present_picture, rrt_gpu::Presenter::present_view, rrt_gpu::Aspect, rrt_gpu::Filter, rrt_gpu::projection, rrt_gpu::plain_format, rrt_gpu::letterbox
---

# rrt-gpu

The wgpu pieces every port rebuilt: opening a device, an offscreen target that
reads back, a depth buffer that follows the size, and a presenter for a
low-resolution picture. No console's GPU is emulated here: the PS1's VRAM
texturing and the PS2's GS stay in their games' crates and draw into a
`Target`.

## Gpu

`Gpu { instance, adapter, device, queue }`. `Gpu::headless()` asks for the
high-performance adapter with no surface; `Gpu::for_surface(instance,
&surface)` for one that can draw to a window (rrt-app calls it).
`Gpu::wait()` blocks until submitted work is done.

## Target

An offscreen colour texture, `RENDER_ATTACHMENT | TEXTURE_BINDING |
COPY_SRC` plus `TargetOptions::usage`. `Target::new(device, w, h)` is plain
`Rgba8Unorm` (`Target::FORMAT`); `Target::with_options` takes:

| field | default | what |
| --- | --- | --- |
| `label` | `"target"` | wgpu's label |
| `width`, `height` | - | 0 is taken as 1 |
| `format` | `Rgba8Unorm` | the colour format |
| `usage` | none | extra usages: `COPY_DST`, `STORAGE_BINDING` |
| `view_formats` | none | e.g. the sRGB twin |
| `samples` | 1 | above 1, a multisampled texture resolves into `texture` |
| `depth` | None | a depth texture of this format, same size and samples |

`color_attachment(load)` returns the attachment with the MSAA resolve wired;
`depth_attachment(clear)` the depth one; `multisample()` the state a pipeline
must match. `resize(device, w, h)` remakes it with the same options (contents
lost). `read_back(&gpu)` returns a `Picture`, alpha forced to 255.
`read_texture` reads any 4-byte texture with `COPY_SRC`. `DepthBuffer` is a
bare depth texture remade when the size it is asked for changes.

## GrowBuffer

A GPU buffer for data that changes every frame: the moving models' vertices,
a frame's uniforms. `GrowBuffer::new(label, usage)` makes nothing;
`write(device, queue, bytes)` writes through the queue from the start,
remaking the buffer only when the bytes outgrow it, at the next power of two
(at least `GrowBuffer::MIN`, 256 bytes), and never shrinks it. `slice()` is
the written part to draw from, `len()` its size. Pair it with
`rrt_kit::Staging` to pack the bytes without allocating:

```rust
let bytes = self.staging.pack(&vertices);              // rrt_kit::Staging
let buf = self.moving.write(&gpu.device, &gpu.queue, bytes);
```

## Presenter

Draws a picture into the window's frame: `present_picture` from a CPU
`Picture` (its texture kept and rewritten while the size holds), or
`present_view` from a texture a renderer drew. An optional overlay
`Picture` goes over it texel for pixel at the window's top left, blended by
its straight alpha (a console).

| setting | values |
| --- | --- |
| `aspect` | `Aspect::Fixed(w/h)` (default `Aspect::TV`, 4:3), `Source` (square pixels), `Stretch` |
| `filter` | `Nearest`; `SharpBilinear` (default: each texel a solid block, only the seam pixel blended); `Linear` |
| `border` | the bars' RGB, 0-1 |

Made for an sRGB format, the presenter converts the picture's
display-encoded values to linear so the surface encodes them back; for a
plain format it writes them as they are. See [colour](../conventions/colour.md).

## Helpers

`projection(fov_y, aspect, near, far)`: right-handed, depth 0 near to 1 far.
`letterbox(w, h, aspect)`: the picture's rectangle in a window.
`plain_format(formats)`: the first non-sRGB one.

## Tests

Unit: `letterbox_bars_the_long_side`, `projection_maps_near_to_0_and_far_to_1`.
On the machine's adapter (`tests/gpu.rs`; lavapipe in CI):
`a_picture_is_letterboxed_into_the_middle`,
`display_encoded_values_survive_an_srgb_target` (0, 1, 10, 64, 128, 200, 255
come back within 1), `sharp_bilinear_blends_only_the_seam` (two texels over
five pixels: red, red, an even mix, green, green; plain bilinear blends three),
`an_overlay_draws_over_the_top_left`, `a_multisampled_target_resolves`,
`a_resized_target_keeps_its_options`,
`a_grow_buffer_grows_by_powers_of_two_and_is_reused`. With no adapter at all they say so and
pass.

## Gaps

Nothing known.
