---
title: rrt-input, the host's pad
status: solid
crates: rrt-input
covers: rrt_input::Input, rrt_input::Input::read, rrt_input::Input::keyboard_only, rrt_input::Input::rumble, rrt_input::Input::motors, rrt_input::Input::describe, rrt_input::Input::gamepads, rrt_input::compose, rrt_input::GamepadState, rrt_input::Pad, rrt_input::Pad::pressed, rrt_input::Pad::released, rrt_input::Buttons, rrt_input::Buttons::libpad, rrt_input::Buttons::from_libpad, rrt_input::Buttons::by_name, rrt_input::Stick, rrt_input::axis_byte, rrt_input::square_stick, rrt_input::StickShape, rrt_input::Motors, rrt_input::KeyMap, rrt_input::Keyboard, rrt_input::Side, rrt_input::Script, rrt_input::Script::parse, rrt_input::Script::buttons_at, rrt_input::Script::end, rrt_input::Press
---

# rrt-input

The keyboard and one gamepad, read once a frame into a `Pad` shaped like a
PlayStation pad's report. The bit order, stick bytes and trigger values are
fixed in [the pad convention](../conventions/pad.md).

## Reading

`Input::new()` opens gilrs with its default filters off (its dead zone
rescales the stick; the game has its own); `Input::keyboard_only()` never
opens it, for tools and tests. `Input::read()`, once a tick:

1. drains gilrs' events. A pad takes over as the one read when it presses a
   button or moves a button or axis past 0.5; drift on another pad does not
   take over; the active pad disconnecting leaves none chosen, and then the
   first connected pad is read,
2. snapshots that pad as a `GamepadState` (its buttons mapped where a
   DualShock has them, the hat, both sticks, L2/R2's analog travel),
3. hands the keyboard and the snapshot to `compose`, which is everything
   else and a pure function:
   - the keyboard's buttons OR the gamepad's OR the hat's directions (past
     0.5, for pads with no SDL mapping),
   - the sticks shaped by `StickShape` (`Square` by default, `square_stick`)
     and converted by `axis_byte`, unless held stick keys override them,
   - L2/R2 as 0-255 travel, or 255 when only the button bit is held,
   - `Pad::analog` true while a gamepad is read.

`Input::describe()` gives the read pad's raw axis values for a pad log.

## Keyboard

`KeyMap::default()`, by physical key:

```
arrows      D-pad          Z X A S   cross circle square triangle
Q W         L1 R1          1 2       L2 R2
Enter       Start          Shift / Backspace   Select
C V         L3 R3          I J K L   right stick up left down right
```

`Keyboard` tracks held keys as a set, so two keys on one button release it
only when both are up; `release_all` lets go of everything when the window
loses focus.

## Scripts

`Script::parse("1300:down,1340:x,1600:start:30,0:l1+r1:2")`: each item is
`FRAME:BUTTONS[:FRAMES]`, buttons by their `Buttons::NAMED` name (or `x`,
`o`) joined with `+`, held `FRAMES` frames (1 when left out). Overlapping
presses add up. `buttons_at(frame)` is what the script holds on a frame,
`end()` the first frame after the last press. A bad item is a `ParseError`
naming the item and what is wrong. `rrt-app` applies one through
`Config::script`.

## Rumble

`Input::rumble(Motors { small, large })` as a DualShock takes them: the
small motor on or off, the large one's power 0-255. Only changes are sent
(`Input::motors` is the last set), and the motors run until set again (each
gilrs effect repeats until stopped). The effect is made on the read pad if it
supports force feedback, and remade when the read pad changes.

## Tests

`pressed_and_released_are_edges_against_the_previous_frame`,
`libpad_order_is_the_byte_swapped_word`, `names_round_trip`,
`axis_bytes_hit_both_ends_and_the_centre`, `square_stick_saturates_diagonals`,
`a_button_stays_held_while_any_of_its_keys_is`, `stick_keys_push_to_the_corner`,
`every_gilrs_button_maps_to_its_own_pad_bit`, `a_hat_presses_the_dpad`,
`the_keyboard_alone_is_a_digital_pad`, `a_gamepad_and_the_keyboard_merge`,
`square_sticks_reach_the_corner_and_round_ones_do_not`,
`stick_keys_override_the_gamepad_stick`,
`the_active_pad_changes_on_a_press_or_a_real_push`,
`rumble_remembers_the_motors_with_no_gamepad`,
`parses_frames_buttons_and_holds`, `holds_cover_their_frames_and_overlaps_add_up`,
`bad_items_say_what_is_wrong`.

## On hardware

What only a pad can show is the gilrs calls themselves:
`GamepadState::from_gilrs` and the force-feedback effects. A DualSense on
Linux is found and listed by `Input::gamepads` (seen in `rrt-demo`'s log).
`rrt-demo` is the check for the rest: every button lights its box, the sticks
move their dots, L2/R2 fill their bars, Circle runs the motors.

## Not here

- What a game does with the pad: edge detection with a repeat delay, its own
  dead zone, the stick pressing the D-pad, `scePadRead` quirks. Those are
  the game's (piney_apples' `ccPad::Read` port stays in `piney-input`).
- DualShock 2 pressure-sensitive face buttons: gilrs reports analog values
  for the triggers only, so no host pad can supply them.

## Gaps

Nothing known.
