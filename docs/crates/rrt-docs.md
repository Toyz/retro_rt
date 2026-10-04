---
title: rrt-docs, the docs checker
status: solid
crates: rrt-docs
covers: rrt_docs::check, rrt_docs::render_index, rrt_docs::declares
---

# rrt-docs

```sh
cargo run -q -p rrt-docs -- index    # regenerate docs/README.md
cargo run -q -p rrt-docs -- check    # all checks; exit 1 on any problem
```

`index` writes `docs/README.md` from every page's front matter and every
crate's `Cargo.toml` description. `check` fails on:

- a folder under `docs/` that is not a section (`SECTIONS` in
  `crates/rrt-docs/src/main.rs`),
- a crate under `crates/` with no `docs/crates/<crate>.md`, no Cargo
  `description`, no `[lints] workspace = true`, or no `//!` header,
- a page missing `title`, `status`, `crates` or `covers`, an unknown status,
  a `crates` entry naming no crate,
- a `covers` item (`rrt_input::Input::read`) whose last segment the crate's
  source does not declare (as a fn, type, const, module, field, variant or
  re-export),
- a dead relative `.md` link,
- a stale index.

The `covers` check is by name, not by full path: it catches an item renamed
or removed, not one moved between modules.

## Gaps

Nothing known.
