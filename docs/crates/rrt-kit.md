---
title: rrt-kit, the small things every port rewrites
status: solid
crates: rrt-kit
covers: rrt_kit::Generator, rrt_kit::Generator::next, rrt_kit::Generator::bits, rrt_kit::Generator::state, rrt_kit::Generator::set_state, rrt_kit::RngExt, rrt_kit::RngExt::modulo, rrt_kit::RngExt::scaled, rrt_kit::RngExt::top_bits, rrt_kit::RngExt::below, rrt_kit::RngExt::range, rrt_kit::RngExt::chance, rrt_kit::RngExt::pick, rrt_kit::RngExt::shuffle, rrt_kit::RngExt::unit_f32, rrt_kit::RngExt::skip, rrt_kit::rng::Lcg, rrt_kit::rng::Lcg::jump, rrt_kit::rng::Lcg64, rrt_kit::rng::Xorshift32, rrt_kit::rng::Lfsr, rrt_kit::rng::Lfsr::next_bit, rrt_kit::rng::TableRng, rrt_kit::rng::Pcg32, rrt_kit::rng::Counted, rrt_kit::rng::Recorded, rrt_kit::rng::Forced, rrt_kit::rng::Forced::force, rrt_kit::Staging, rrt_kit::Staging::pack, rrt_kit::Staging::begin, rrt_kit::Staging::bytes, rrt_kit::Pack, rrt_kit::Pool, rrt_kit::Pool::take, rrt_kit::Pool::give, rrt_kit::Pool::with_max_spare, rrt_kit::Pool::with_max_capacity, rrt_kit::Pool::trim, rrt_kit::SyncPool, rrt_kit::SyncPool::take, rrt_kit::SyncPool::take_vec, rrt_kit::SyncPool::give, rrt_kit::SyncPool::set_max_spare, rrt_kit::SyncPool::set_max_capacity, rrt_kit::SyncPool::trim, rrt_kit::Pooled, rrt_kit::Pooled::into_inner, rrt_kit::Slab, rrt_kit::Slab::insert, rrt_kit::Slab::get, rrt_kit::Slab::get_mut, rrt_kit::Slab::remove, rrt_kit::Slab::retain, rrt_kit::Handle, rrt_kit::Ring, rrt_kit::Ring::push, rrt_kit::Ring::iter, rrt_kit::Fixed, rrt_kit::Q12, rrt_kit::Fixed::from_raw, rrt_kit::Fixed::from_f32, rrt_kit::Fixed::to_f32, rrt_kit::Fixed::floor, rrt_kit::Reader, rrt_kit::Eof, rrt_kit::Pixel, rrt_kit::color::expand, rrt_kit::color::reduce, rrt_kit::color::to_rgba8, rrt_kit::Rgb555, rrt_kit::Rgb555::rgb8, rrt_kit::Rgb555::rgb8_shifted, rrt_kit::Rgb555::rgba8, rrt_kit::Argb1555, rrt_kit::Rgb565, rrt_kit::Bgr565, rrt_kit::Argb4444, rrt_kit::Abgr4444, rrt_kit::Rgba5551, rrt_kit::Rgb5a3, rrt_kit::Ia16, rrt_kit::Ia8, rrt_kit::Md333, rrt_kit::Sms222, rrt_kit::Rgb332, rrt_kit::PsmCt32, rrt_kit::color::ycbcr_to_rgb, rrt_kit::Range, rrt_kit::color::indexed4, rrt_kit::color::indexed8, rrt_kit::Nibbles, rrt_kit::color::ps2_clut_index, rrt_kit::bcd::decode, rrt_kit::bcd::encode, rrt_kit::bcd::msf_to_lba, rrt_kit::bcd::lba_to_msf
---

# rrt-kit

Small types every port ended up writing for itself. No dependencies, no I/O.

## Recycling

A frame that allocates nothing once warm. Both keep their memory between
frames and only grow.

`Staging` is a byte buffer packed again each frame: `pack(&values)` clears it
and writes each value's `Pack` layout end to end, reallocating only when the
values outgrow every earlier frame (hwtr-render's `VertexStaging`, made
generic). `begin()` hands over the cleared `Vec<u8>` to write by hand.
`Pack` is a fixed little-endian layout: implemented for the integer and float
primitives and arrays of them; a vertex implements it field by field in its
pipeline's order. Pair with `rrt_gpu::GrowBuffer` for the upload.

`Pool<T>` lends empty `Vec<T>`s and takes them back: `take()` gives a spare
one with the capacity it had (or a new one with `capacity`), `give(v)` clears
it and keeps it. Two limits, each a public field with a builder:
`max_spare` (64 by default; a Vec given back past it is dropped) and
`max_capacity` (none by default; a Vec given back bigger is shrunk to it, so
one huge frame does not pin its memory). `with_max_spare` and
`with_max_capacity` apply to spares already held; `trim(n)` drops spares down
to `n` now.

`SyncPool<T>` is the same across threads. Clone it to hand it to another
thread; every clone draws on the same spares and sees the same limits
(`set_max_spare`, `set_max_capacity`, or `with_*` as it is made). One mutex
guards the spare list and is held only to push or pop a `Vec`, never while a
caller uses one; clearing and shrinking happen outside it. So a taker on an
audio thread waits at most for another thread's push or pop. `take()`
returns a `Pooled<T>` guard that derefs to the `Vec` and gives it back when
dropped, wherever that is - including while its thread unwinds from a panic;
`into_inner()` keeps the `Vec` instead. `take_vec()` and `give()` are the
by-hand pair. A poisoned lock is used as it is: the spare list is only
pushed and popped, so a panic cannot leave it half-changed.

## Containers

`Slab<T>` is a generational arena: `insert` returns a `Handle { index,
generation }`; `get`, `get_mut`, `contains` and `remove` find nothing once
the value is removed, even after its slot is reused, because the slot's
generation moved on. Insert and remove are O(1) and reuse freed slots;
`iter`, `iter_mut`, `retain` and `clear` work in slot order. For objects,
entities, sounds - anything referred to by id after it may have gone.

`Ring<T>` holds the newest `capacity` values, oldest first: `push` returns the
value it pushed out once full; `get(i)` (0 the oldest), `oldest`, `newest`,
and `iter` (double-ended, allocation-free). For frame-time graphs, input
history, rewind buffers.

## Randomness a port can reproduce

A game's randomness is two things a port must match bit for bit: the
**generator** (state and step) and the **reduction** (how a raw value became
a roll - `rand() % 6` and `(rand() * 6) >> 15` give different rolls from the
same value).

`Generator` is the trait a generator implements: `next()` steps and returns
a value in its low `bits()` bits; `state()` / `set_state()` save and restore
everything that decides the sequence, for save states, replays and lockstep
checks. A game with its own scheme implements these four and gets the rest;
it also works as a trait object (`Box<dyn Generator<State = u8>>`).

| generator | what | checked against |
| --- | --- | --- |
| `Lcg` | 32-bit LCG, any constants: `state * mul + add`, output `(state >> shift)` masked to `out_bits` | - |
| `Lcg::ANSI_C` | the C standard's example `rand` (1103515245, 12345, bits 16-30), shipped by many compilers and console libraries | seed 1: 16838, 5758, 10113, 17515, 31051 |
| `Lcg::MSVC` | Microsoft C's `rand` (214013, 2531011, bits 16-30) | seed 1: 41, 18467, 6334, 26500, 19169 |
| `Lcg64`, `Lcg64::NEWLIB` | 64-bit-state LCG; newlib's `rand` (GCC console toolchains) | newlib's formula |
| `Xorshift32` | Marsaglia's, shifts 13, 17, 5 by default; never seeded 0 | seed 1: 270369 |
| `Lfsr` | Galois LFSR, any width 1-32 and taps; `next_bit` for bit-at-a-time games | 16-bit 0xACE1 / taps 0xB400: 0xE270, period 65535 |
| `TableRng` | values read in turn from the game's own table, index as state; step before reading (DOOM's order) or after | - |
| `Pcg32` | O'Neill's PCG32, for randomness the port adds of its own | `pcg32_srandom(42, 54)` demo output |

`Lcg::seeded(s)` is `srand(s)`; `Lcg::jump(n)` skips `n` steps in O(log n)
by composing the step, to resync with an original that drew values the port
does not.

`RngExt`, on every generator:

- the era's reductions, named for the expression: `modulo(n)` is
  `next() % n`; `scaled(n)` is `(next() * n) >> bits`; `top_bits(k)` is
  `next() >> (bits - k)`. Port the game's own expression with these.
- unbiased helpers for randomness the port adds: `below(n)` (rejection, so
  it may draw more than one value - never where the original drew one),
  `range(lo, hi)`, `chance(num, den)`, `pick`, `shuffle` (Fisher-Yates with
  `below`), `unit_f32`; and `skip(n)`.

Wrappers, each a `Generator` passing state through: `Counted` (calls since
made - compare with the original's at the same frame to find a missing or
extra draw), `Recorded` (the last outputs in a `Ring`), `Forced` (queued
values come out first without stepping the inner generator, so the sequence
after is the one the game would have continued with - for forcing a crit in
a test or a debug menu).

## Data as the consoles hold it

`Fixed<FRAC>` is an `i32` with `FRAC` fraction bits; `Q12 = Fixed<12>` is the
PS1 GTE's 4.12 matrices and 20.12 positions (4096 is 1.0). `from_raw`/`raw`
are the console's word; `from_int`, `from_f32`, `to_f32`, `floor` (toward
minus infinity). `+`, `-` and negation wrap like 32-bit registers; `*`
widens to 64 bits and shifts right arithmetically, truncating toward minus
infinity as the GTE and its compilers' `>> 12` do.

`Reader` is a cursor over `&[u8]` for file formats: `u8`, `i8`, and
`u16`/`i16`/`u32`/`i32`/`u64`/`f32` each `_le` and `_be`; `bytes(n)`,
`array::<N>()`, `skip`, `seek`, `pos`, `remaining`; `text(n)` for a fixed
field (to the first NUL, trailing spaces trimmed). A read past the end is an
`Eof { at, want, len }` and moves nothing.

### Colour

Every format is a newtype over its raw word and implements `Pixel`:
`to_rgba8()` (straight alpha, channels expanded to 0-255 by bit replication,
`expand`) and `from_rgba8()` (channels truncated, `reduce`), so every
representable colour round-trips. `color::to_rgba8(&pixels)` converts a
slice to image bytes.

| type | layout, high bit to low | systems |
| --- | --- | --- |
| `Rgb555` | `T BBBBB GGGGG RRRRR` | PS1 VRAM, PS2 PSMCT16, SNES, GBA, GBC, Saturn RGB, PSP 5551 |
| `Argb1555` | `A RRRRR GGGGG BBBBB` | Dreamcast, PC (A1R5G5B5) |
| `Rgb565` | `RRRRR GGGGGG BBBBB` | Dreamcast, Xbox, GameCube, PC |
| `Bgr565` | `BBBBB GGGGGG RRRRR` | PSP 5650 |
| `Argb4444` | `AAAA RRRR GGGG BBBB` | Dreamcast, PC |
| `Abgr4444` | `AAAA BBBB GGGG RRRR` | PSP 4444 |
| `Rgba5551` | `RRRRR GGGGG BBBBB A` | N64 RGBA16 |
| `Rgb5a3` | `1 RRRRR GGGGG BBBBB` or `0 AAA RRRR GGGG BBBB` | GameCube, Wii |
| `Ia16` | `IIIIIIII AAAAAAAA` | N64 IA16 |
| `Ia8` | `IIII AAAA` | N64 IA8 |
| `Md333` | `0000 BBB0 GGG0 RRR0` | Mega Drive / Genesis CRAM |
| `Sms222` | `00 BB GG RR` | Master System |
| `Rgb332` | `RRR GGG BB` | MSX2 screen 8, 8-bit PC modes |
| `PsmCt32` | bytes R G B A, A 0x80 = 1.0 | PS2 PSMCT32 |

Format rules worth knowing:

- `Rgb555` follows the PS1 texture rule as a `Pixel`: the word 0x0000 is
  transparent, every other word (black with STP included) opaque; opaque
  black packs as 0x8000 so it stays drawn. RGBA8 has no place for STP, so it
  survives the round trip only on black - keep raw words where STP matters.
  `rgb8_shifted` (`c << 3`, 31 to 248) is what the PS1 GPU does with a texel
  before shading; `rgb8` is the full range.
- `Argb1555` and `Rgba5551`: alpha is one bit; from RGBA8, 128 and up is
  opaque.
- `Rgb5a3`: alpha 255 packs as the opaque RGB555 form, anything less as
  ARGB3444.
- `Ia16`, `Ia8`: grey; from RGBA8 the intensity is the mean of R, G, B.
- `Md333`: expanded linearly. The Mega Drive's DAC is not linear, so this is
  the colour as data, not as a television showed it.
- `PsmCt32`: alpha doubles to RGBA8 and saturates (0x80 is 255, over 1.0
  stays 255); from RGBA8 it halves, rounding up.

`ycbcr_to_rgb(y, cb, cr, range)` converts decoded video by BT.601:
`Range::Full` (0-255, JPEG) or `Range::Limited` (Y 16-235, MPEG, so PS2 PSS
and IPU output). The PS1 MDEC and PS2 IPU leave this step to the program.

`indexed4(data, palette, order)` and `indexed8(data, palette)` look a
paletted image up in a palette of any `Pixel`; an index past the palette is
transparent black. `Nibbles::LowFirst` (PS1, PS2, PSP, GBA, Saturn) or
`HighFirst` (N64, GameCube, Mega Drive tiles) says which pixel of a byte is
first. `ps2_clut_index(i)` swaps index bits 3 and 4: the PS2's CSM1 order
for a 256-colour CLUT, its own inverse.

`bcd`: `decode` (0x59 to 59, None for a digit over 9), `encode`, and CD
addresses: `msf_to_lba` (the 150-sector lead-in taken off; None inside it) and
`lba_to_msf`.

## Tests

`values_pack_little_endian_in_field_order`, `the_buffer_is_reused_and_never_shrinks`,
`a_vec_given_back_is_lent_again_with_its_capacity`, `past_max_spare_a_given_vec_is_dropped`,
`a_vec_past_max_capacity_is_shrunk_when_given_back`, `limits_can_be_lowered_later`,
`a_sync_pool_and_its_guards_cross_threads`, `a_dropped_guard_goes_back_and_is_lent_again`,
`eight_threads_share_one_pool_within_its_limits` (8 threads, 1000 rounds each),
`limits_set_through_one_clone_hold_for_all`,
`a_guard_dropped_while_its_thread_panics_still_goes_back`,
`a_stale_handle_finds_nothing_after_its_slot_is_reused`, `insert_get_iterate_and_retain`,
`past_capacity_the_oldest_goes`, `below_capacity_it_is_in_push_order`,
`q12_holds_4096_as_one`, `multiply_truncates_toward_minus_infinity`,
`add_and_sub_wrap_like_the_registers`, `integers_read_in_either_order_and_advance`,
`a_read_past_the_end_fails_and_moves_nothing`, `text_fields_stop_at_nul_and_lose_trailing_spaces`,
`expansion_replicates_bits_to_the_full_range`,
`every_word_of_every_16_bit_format_round_trips` (all 65536 words of each),
`every_byte_of_every_8_bit_format_round_trips`, `red_lands_where_each_format_keeps_it`,
`alpha_rules`, `rgb555_channels_top_bit_and_both_expansions`, `ps2_alpha_is_0x80_for_one`, `ycbcr_greys_stay_grey_and_ranges_differ`,
`indexed_images_look_up_their_palette`, `the_ps2_clut_swaps_index_bits_3_and_4`, `bcd_round_trips_and_refuses_non_digits`,
`sector_16_is_read_at_00_02_16`, `ansi_c_rand_matches_its_published_sequence`,
`msvc_rand_matches_its_published_sequence`, `pcg32_matches_the_reference_demo`,
`xorshift32_steps_as_marsaglia_wrote_it`, `a_maximal_16_bit_lfsr_has_period_65535`,
`newlib_rand_is_its_64_bit_lcg_formula`, `a_table_reads_in_turn_and_wraps`,
`an_lcg_jumps_to_where_stepping_lands`, `saved_state_replays_the_same_sequence`,
`the_era_reductions_are_the_expressions_they_name`,
`below_and_range_cover_their_span_and_stay_inside`, `chance_pick_shuffle_and_unit`,
`counted_counts_and_recorded_remembers`,
`forced_values_come_first_and_leave_the_sequence_alone`,
`a_custom_generator_gets_every_helper`.

## Not here

A lock-free pool: `SyncPool`'s lock is held for one push or pop, which is
expected to be far below an audio buffer's period (inferred, not measured;
measure it before reaching for lock-free); any one game's generator (its constants and its reductions are the
port's, built from this trait); cryptographic randomness; fixed palettes
with no canonical values (the NES's
differs by console revision and capture); compressed texture formats (S3TC,
GameCube CMPR, PVRTC) and console texture swizzles other than the PS2 CLUT's,
which are decoders a game's crate owns; division and transcendental functions on `Fixed`
(each console's routines differ, so they belong to the game that ports them);
anything one console's hardware alone uses.

## Gaps

Nothing known.
