---
title: Dependencies: one version of everything, pinned here
status: solid
crates: all
covers: rrt::wgpu, rrt::winit, rrt::glam, rrt::tracing
---

# Dependencies

The versions of wgpu, winit, glam, gilrs, cpal, pollster, flate2 and
tracing are set once, in `[workspace.dependencies]` of retro_rt's
`Cargo.toml`. Games get them through the facade.

- A game depends on `rrt` and names `rrt::wgpu`, `rrt::winit`, `rrt::glam`,
  `rrt::tracing`. It does not add its own `wgpu`, `winit` or `glam` line: a
  second version is a second, incompatible set of types (`wgpu::Device`
  from one cannot be passed to the other).
- Upgrading wgpu is one edit here, then fixing the runtime, then each game
  builds against it.
- A crate in retro_rt takes its dependencies with `.workspace = true` and
  carries `[lints] workspace = true` (`missing_docs` warns; `rrt-docs check`
  enforces the lints line).
- The runtime holds nothing console-specific. A piece of code belongs here
  when two games would otherwise carry a copy of it and it does not encode
  one console's hardware or one game's behaviour.
