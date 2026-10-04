---
title: rrt-macros, #[rrt::main] and #[derive(rrt::Args)]
status: solid
crates: rrt-macros
covers: rrt_macros::main, rrt_macros::derive_args
---

# rrt-macros

`#[rrt::main(...)]` on `fn main() -> R`, where `R: rrt::app::IntoGame`: the
game, `rrt::app::WithArgs(game, app_args)` (the standard flags applied by
`launch`, `--shot` included), or a `Result` of either:

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

## #[derive(rrt::Args)]

A struct with named fields becomes the program's command line: each field
an argument, its doc comment's first paragraph the help, the struct's the
about.

| field type | argument |
| --- | --- |
| `bool` | a flag, `--name` |
| `Option<T>` | an optional `--name VALUE` |
| `Vec<T>` | a repeatable `--name VALUE` |
| `T` | a required `--name VALUE`, or optional with `#[arg(default = "...")]` |

`T` is anything `rrt::cli::FromArg`. The long name is the field's, `_` to
`-`. `#[arg(...)]`: `long = "name"`, `short = 'x'`, `alias = "other"`
(repeatable), `default = "text"`, `value_name = "N"` (default from the type:
PATH, N, NUMBER, TEXT, WxH, VALUE), `positional` (in field order; `Option`
optional, `Vec` the rest), `flatten` (another `Args` struct's arguments in
place: `#[arg(flatten)] app: rrt::app::AppArgs` gives every game the
standard flags). `#[args(crate = "path")]` on the struct moves the `cli`
module from `::rrt::cli`.

```rust
/// Hot Wheels Turbo Racing, ported.
#[derive(rrt::Args)]
struct Cli {
    /// The disc's CUE sheet.
    #[arg(positional, default = "work/disc/game.cue")]
    cue: PathBuf,
    /// Start on this track.
    #[arg(short = 't', default = "DESERT1")]
    track: String,
    #[arg(flatten)]
    app: AppArgs,
}

#[rrt::main(title = "hwtr")]
fn main() -> Result<WithArgs<Hwtr>, String> {
    let cli = Cli::parse_env(env!("CARGO_PKG_VERSION"));
    Ok(WithArgs(Hwtr::load(&cli.cue, &cli.track)?, cli.app))
}
```

Tests (`crates/rrt/tests/args.rs`, where `::rrt` resolves):
`every_field_shape_parses`, `missing_and_wrong_arguments_are_errors`,
`the_specs_and_usage_come_from_the_struct`.

## Gaps

Nothing known. The expansion is exercised by building `rrt-demo`.
