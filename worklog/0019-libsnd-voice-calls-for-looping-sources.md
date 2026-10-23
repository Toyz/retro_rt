---
number: 19
title: libsnd: voice calls for looping sources
date: 2026-10-22
area: audio
files: crates/rrt-kit/src/libsnd/synth.rs, docs/crates/rrt-kit.md
started: 2026-10-22T20:45:42-06:00
took: 25m
---

# 19. libsnd: voice calls for looping sources

Rugrats keeps its looping sound sources on voices of its own. The level's set-up starts them (0x8006fe04). Each frame (0x8006f88c):
- a source in hearing keys on voice 15 or 16 (`SsUtKeyOnV`);
- its volume then follows the listener's distance (`SsUtSetVVol`);
- it is keyed off out of hearing.

The software libsnd keyed effects only on the effect voices in turn, so `Spu` gains four calls:
- `key_on_voice(voice, program, tone, note, (left, right))`: an effect on that voice; `key_on` now goes through it;
- `set_voice_volume(voice, (left, right))`: the voice's volumes against its tone's, as it plays. Each voice keeps the volume its tone, program and bank gave it, and that base is what this scales;
- `key_off(voice)`: its envelope releases;
- `voice_on(voice)`: whether it still sounds.

The test `a_voice_keyed_on_turned_and_off` checks it sounds, turns down, survives the effects' own key-ons, and falls silent after the key-off.

**Still unknown:**
- the exact law of SsUtSetVVol against the tone's volume (here it scales the base as key_on's volumes do)
