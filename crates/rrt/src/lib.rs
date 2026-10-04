//! retro_rt in one dependency.
//!
//! A game depends on this crate alone and reaches each part through its
//! module: [`app`] (the loop: implement `app::Game`, run with
//! `#[rrt::main]`), [`input`], [`gpu`], [`audio`], [`disc`], [`net`],
//! [`image`], [`kit`]. The wgpu, winit and glam it was built with are re-exported as
//! [`wgpu`], [`winit`] and [`glam`]: use them, not your own, so the versions
//! cannot drift. Each part is a feature; `default-features = false` and a
//! list picks fewer (a tool that only reads discs wants `disc` alone).
//!
//! Start at `docs/README.md` in the retro_rt repository; the
//! `docs/guides/new-game.md` page walks through a first game.

#[cfg(feature = "app")]
pub use rrt_app as app;
#[cfg(feature = "audio")]
pub use rrt_audio as audio;
#[cfg(feature = "disc")]
pub use rrt_disc as disc;
#[cfg(feature = "gpu")]
pub use rrt_gpu as gpu;
pub use rrt_image as image;
#[cfg(feature = "input")]
pub use rrt_input as input;
pub use rrt_kit as kit;
#[cfg(feature = "net")]
pub use rrt_net as net;

/// The game's entry point. See `rrt_macros::main`.
#[cfg(feature = "app")]
pub use rrt_macros::main;

/// tracing, which the loop's logging (`Config::log`, `RUST_LOG`) collects.
#[cfg(feature = "app")]
pub use rrt_app::tracing;
/// winit, at the version the loop was built with.
#[cfg(feature = "app")]
pub use rrt_app::winit;
/// glam, at the version the renderer was built with.
#[cfg(feature = "gpu")]
pub use rrt_gpu::glam;
/// wgpu, at the version the renderer was built with.
#[cfg(feature = "gpu")]
pub use rrt_gpu::wgpu;

/// The names nearly every game file wants.
#[cfg(feature = "app")]
pub mod prelude {
    pub use rrt_app::{Config, Draw, Game, Init, Tick};
    pub use rrt_gpu::{Gpu, Picture, Target};
    pub use rrt_input::{Buttons, Motors, Pad};
}
