---
title: rrt-macros, the #[rrt::main] entry point
status: solid
crates: rrt-macros
covers: rrt_macros::main
---

# rrt-macros

`#[rrt::main(...)]` on `fn main() -> R`, where `R: rrt::app::IntoGame` (the
game, or a `Result` of it):

```rust
#[rrt::main(title = "hwtr", hz = rrt::app::rate::NTSC, size = (960, 720))]
fn main() -> Result<Hwtr, String> {
    Hwtr::load(&args()?)          // logging has already started
}
```

expands to

```rust
fn main() {
    let config = ::rrt::app::Config::default().title("hwtr").hz(rrt::app::rate::NTSC).size((960, 720));
    ::rrt::app::launch(config, || -> Result<Hwtr, String> { Hwtr::load(&args()?) });
}
```

Each `name = value` is a call to the `Config` builder method of that name, so
the settings are exactly [`Config`'s](rrt-app.md#config) and a misspelt one
is a compile error naming the missing method. The function must take no
arguments and not be async. The expansion names `::rrt`, so the game depends
on the [facade](rrt.md), not on `rrt-app` directly.

A body may exit early (a `--shot` run that writes its PNG and calls
`std::process::exit(0)`, as `rrt-demo` does).

## Gaps

Nothing known. The expansion is exercised by building `rrt-demo`.
