---
title: Colour: display-encoded values, converted once
status: solid
crates: rrt-gpu, rrt-app
covers: rrt_gpu::Presenter, rrt_gpu::Target, rrt_gpu::plain_format, rrt_app::Init
---

# Colour

A retro game's pixel values are what the television showed: display-encoded,
not linear light. retro_rt keeps them that way everywhere and converts in one
place.

- A `Picture`, a `Target` (`Target::FORMAT` is `Rgba8Unorm`, not sRGB) and a
  game's renderer hold and blend the values as the console did. Blending in
  this space is what the hardware did, so it is right to do it here.
- The window's surface is configured sRGB when the adapter offers one, with
  its non-sRGB twin as an extra view format. `Init::format` is the surface's,
  `Init::plain_format` the twin; `Draw::target` and `Draw::plain` view the
  same frame in each.
- The `Presenter`, built for an sRGB format, converts each value to the
  linear value the surface will encode back to it; built for a plain format,
  it writes values as they are.
- A game drawing straight to the window draws through `Draw::plain` (or a
  pipeline made for `Init::plain_format`), so its values land unconverted.

Alpha in a target is the console's (the GS's alpha, a PS1 semi-transparency
flag), not an opacity: `Target::read_back` forces it to 255.

## Not decided

Gamma or colour adjustments a console applied on output (PS2 GS gamma, a
CRT's response) are not modelled; a game that wants them does it before
presenting.
