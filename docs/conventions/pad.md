---
title: The pad: bit order, sticks, triggers
status: solid
crates: rrt-input, rrt-app
covers: rrt_input::Buttons, rrt_input::Script, rrt_input::Stick, rrt_input::axis_byte, rrt_input::square_stick, rrt_input::Pad
---

# The pad

Every game on retro_rt reads the same `Pad`, shaped like a PlayStation pad's
report, and converts to its own console's layout in its own crate.

## Buttons

16 bits, active high (1 pressed), in the order the PlayStation pad shifts
them out over SIO:

```
bit  0 SELECT   4 UP      8 L2       12 TRIANGLE (north)
     1 L3       5 RIGHT   9 R2       13 CIRCLE   (east)
     2 R3       6 DOWN   10 L1       14 CROSS    (south)
     3 START    7 LEFT   11 R1       15 SQUARE   (west)
```

| consumer | conversion |
| --- | --- |
| a PS1 game (`hwtr`) | the word as it is: `Buttons::bits()` |
| the wire (active low) | `Buttons::active_low()` |
| a PS2 libpad game (`piney_apples`' `direct`) | bytes swapped: `Buttons::libpad()`, `Buttons::from_libpad()` (`libpad_order_is_the_byte_swapped_word`) |
| any other machine | by position: `SOUTH`, `EAST`, `WEST`, `NORTH` |

Names for scripted presses and logs (`Buttons::NAMED`, `by_name`):
`select l3 r3 start up right down left l2 r2 l1 r1 triangle circle cross
square`, plus `x` and `o`.

## Sticks

One byte an axis: 0 left or up, 0x80 centre, 0xff right or down. From a
gilrs axis (-1..1, y up positive) by `axis_byte`: 128 steps below centre, 127
above, rounded; y flipped. No dead zone anywhere in the runtime: the game
applies its own, as the original did.

A round-gated modern stick reaches about 0.7 an axis on a diagonal; a
DualShock's potentiometers saturate to the corner. `square_stick` (the
default `StickShape::Square`) carries the circle to the square so games tuned
on DualShocks see full diagonals (`square_stick_saturates_diagonals`).

## Triggers

`Pad::l2` and `Pad::r2`: 0 released to 255, from the gamepad's analog
trigger, or 255 when only the button bit is held. The L2/R2 bits are set as
gilrs reports the buttons pressed.

## Scripts

Scripted presses use the ports' `--press` form, `FRAME:BUTTONS[:FRAMES]`
comma separated, buttons by the names above joined with `+`
(`rrt_input::Script`). Frames count from 0, one a tick, as `Tick::frame`
does.

## Rumble

`Motors { small: bool, large: u8 }`, the DualShock's two motors. They run
until set again.
