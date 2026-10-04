---
title: rrt-image, pictures out of a game
status: solid
crates: rrt-image
covers: rrt_image::font::draw, rrt_image::font::measure, rrt_image::font::glyph, rrt_image::font::GLYPHS, rrt_image::Picture, rrt_image::Picture::filled, rrt_image::Picture::get, rrt_image::Picture::set, rrt_image::Picture::fill_rect, rrt_image::Picture::to_png, rrt_image::png::encode
---

# rrt-image

`Picture { width, height, rgba }`: RGBA8, rows top first, no padding. The
type every crate passes pictures in: the presenter shows one, a target reads
back into one, a game's texture decoder produces one.

`png::encode(width, height, rgba)` writes 8-bit RGBA, one IDAT, filter 0 on
every row, zlib at the default level. `Picture::to_png` calls it. It panics
when `rgba` is shorter than `width * height * 4`. Tests:
`crc_of_iend_is_the_fixed_value`, `a_one_pixel_png_has_signature_header_and_end`,
`rects_clip_to_the_picture`.

## Text

`font` draws text on a `Picture` for consoles, HUDs and debug overlays:
`draw(picture, x, y, text, rgba, scale)` (only glyph pixels are set; `\n`
starts a new line; returns the x after the text), `measure(text, scale)`,
`glyph(c)`. The glyphs are X11's "Misc Fixed" 6x13 (the classic xterm font),
printable ASCII, converted from `6x13.pcf`, whose copyright property reads
"Public domain font. Share and enjoy."; anything else draws as `?`. Tests:
`the_capital_a_is_the_fonts`, `drawing_sets_only_glyph_pixels_scaled_and_clipped`.

## Not here

Decoding. Game data arrives in the game's formats (TIM, GS textures), which
its own crate decodes to a `Picture`.

## Gaps

Nothing known.
