---
title: rrt, the facade a game depends on
status: solid
crates: rrt
covers: rrt::prelude, rrt::main, rrt::wgpu, rrt::winit, rrt::glam, rrt::tracing
---

# rrt

The one dependency a game takes. Each runtime crate is a module of it, behind
a feature; the libraries whose types cross crate boundaries are re-exported at
the versions the runtime was built with.

```toml
[dependencies]
rrt = { path = "../../repos/retro_rt/crates/rrt" }
# a disc tool that wants nothing else:
rrt = { path = "...", default-features = false, features = ["disc"] }
```

## Modules and features

| module | crate | feature | default |
| --- | --- | --- | --- |
| `rrt::app` | [rrt-app](rrt-app.md) | `app` (pulls `gpu`, `input`, the macro) | yes |
| `rrt::input` | [rrt-input](rrt-input.md) | `input` | via `app` |
| `rrt::gpu` | [rrt-gpu](rrt-gpu.md) | `gpu` | via `app` |
| `rrt::audio` | [rrt-audio](rrt-audio.md) | `audio` | yes |
| `rrt::disc` | [rrt-disc](rrt-disc.md) | `disc` | yes |
| `rrt::net` | [rrt-net](rrt-net.md) | `net` | yes |
| `rrt::image` | [rrt-image](rrt-image.md) | always | yes |
| `rrt::kit` | [rrt-kit](rrt-kit.md) | always | yes |
| `#[rrt::main]` | [rrt-macros](rrt-macros.md) | `app` | yes |

## Re-exports

`rrt::wgpu`, `rrt::glam` (with `gpu`), `rrt::winit`, `rrt::tracing` (with
`app`). A game names these and never adds its own `wgpu`, `winit` or `glam`
line: two wgpu versions in one build are two incompatible sets of types. See
[dependencies](../conventions/dependencies.md).

`rrt::prelude` holds the names nearly every game file wants: `Config`, `Game`,
`Init`, `Tick`, `Draw`, `Gpu`, `Picture`, `Target`, `Buttons`, `Motors`,
`Pad`.

## Gaps

Nothing known.
