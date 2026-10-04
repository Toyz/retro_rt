---
name: worklog
description: Write an entry in the project worklog - one file per entry under worklog/, indexed by WORKLOG.md. Use whenever a unit of work finishes - a problem diagnosed, a subsystem built and verified, a decision made, a dead end ruled out, a measurement taken. Also use when the user says "worklog", "log this", "write it up", "/worklog".
---

# Worklog

The durable record of retro_rt. Code says what; the worklog says *how we
found out* and *why it is that way*. It is the primary artifact that survives
context compaction - write it for a reader who was not here, including yourself
in a later session.

## Where it lives

One entry per file:

```
worklog/0003-a-short-title.md    the entries, NNNN-slug.md
WORKLOG.md                            generated index - never edit by hand
```

One file per entry means the next number is a filename lookup rather than a read
of the whole log, an entry can be found by grepping front matter instead of
scrolling, and two entries written in the same session do not collide in a diff.

## Writing one

Write the whole entry in one command, with the prose on stdin:

```sh
cairns new "Short title in plain words" --area app,input --files "a.rs,b.rs" \
  --unknown "what remains open, or nothing" --body - <<'EOF'
The finding first, then the evidence.

## A sub-heading if it needs one
EOF
```

That numbers and dates the entry, writes the front matter and the `# N. Title`
heading, appends the `**Still unknown:**` line from `--unknown`, and refreshes
the index. There is no file to open and edit afterwards.

- `--body -` reads the prose from stdin. Quote the heredoc marker (`<<'EOF'`)
  so backticks and `$` in the prose arrive as written.
- The body is the prose only. Do not start it with the `# N. Title` heading -
  the command writes that, and drops a leading one if you do.
- Say what is still unknown **once**: either `--unknown "..."`, or a
  `**Still unknown:**` line at the end of the body - not both, and not neither.
  `--unknown nothing` closes the entry out. The command refuses the other two
  cases and says which it was, before anything is written.

Without `--body`, `cairns new` writes a stub with the front matter and heading,
to be filled in by editing the file.

An entry may be filed under **several areas at once** - comma separated, as
above, or by repeating `--area`. Where a piece of work genuinely sits in two,
say so; it is truer than picking whichever it was mostly.

`--area` is one or more of:

| area | what belongs there |
| --- | --- |
| `app` | the engine loop, Game, Config, the clock, #[rrt::main], headless runs |
| `input` | the host pad: gamepads, keyboard, rumble, the pad convention |
| `gpu` | wgpu plumbing: devices, targets, readback, the presenter, colour |
| `audio` | sound out: sources, the device, resampling |
| `disc` | disc images, CUE sheets, raw sectors, ISO 9660 |
| `net` | UDP, TCP, framing, the reliable UDP protocol |
| `docs` | the reference pages, the docs skill, rrt-docs |
| `port` | moving a game onto the runtime, or what a game needed from it |
| `design` | a decision made, with the alternatives rejected |
| `bug` | a fault diagnosed, with the root cause |
| `perf` | a measurement taken |
| `build` | workspace layout, dependencies, CI, how things are run |
| `test` | harnesses and fixtures |

Other commands:

```sh
cairns next     # the number the next entry would take
cairns index    # regenerate the index
cairns open     # every unresolved question in the log
cairns check    # numbering sound, front matter complete, index current
```

Run `check` before finishing. It catches a stale index, a file whose name no
longer matches its title, a missing date, and an area nobody has heard of.

## When to write an entry

When a unit of work concludes:

- a problem diagnosed, with the root cause - not just the symptom
- a subsystem built and verified, with what verified it
- a decision made, with the alternatives that were rejected and why
- a hypothesis tested and **disproven** - dead ends are the most valuable
  entries, because they stop the next session re-walking them
- a measurement taken - a benchmark, a size, a count - because the number is
  the thing that is hard to get again
- a claim in an earlier entry overturned

Do not write an entry for trivial edits, formatting, or anything the diff
already explains on its own.

## Entry format

The file starts with front matter the tool wrote. Do not renumber it, and do not
edit the index to match - run `cairns index`.

```markdown
---
number: 12
title: A short title in plain words
date: 2026-09-20
area: app, input
files: src/thing.rs
supersedes: 6
---

# 12. A short title in plain words

<What was done and what was learned, in prose. Lead with the finding, not the
process. Include the concrete evidence: numbers, offsets, names, sample values,
measured timings. Show the table or the struct in a fenced block when there is
one.>

**Still unknown:** <what remains open, or "nothing" if closed out.>
```

Sub-headings inside an entry use `##` - the entry's own title is the `#`.

`**Still unknown:**` must begin a line, and it is what `cairns open` collects
across the whole log. It is the log's list of what the project does not yet
know, so it is worth writing honestly rather than leaving blank.

## Pointing at another entry

Write `[[12]]`. It becomes a link to entry 12 carrying that entry's title, and
`check` fails if entry 12 does not exist. `[[12|in other words]]` supplies your
own wording.

Use it freely in prose - "as [[12]] found", "this contradicts [[6]]". A log
whose entries do not point at each other is a pile of entries.

Inside code, fenced or inline, `[[...]]` is left exactly as written.

## Linking an entry to an earlier one

The log is append-only. Never rewrite or renumber an entry. Two front matter
fields carry everything that would otherwise tempt you to edit one, and both
take one or more entry numbers:

```sh
cairns new "What changed" --area app --supersedes 6
cairns new "What it turned out to be" --area app --resolves 6
```

**`supersedes`** - this entry overturns something an earlier one claimed. The
reader who lands on entry 6 is then told that 12 corrected it. This is the
single most valuable edge in the log, and it only exists if you record it.

**`resolves`** - this entry answers the question an earlier one left open. That
question then leaves the open-questions list, and entry 6 keeps it, struck
through, naming what closed it.

They are not interchangeable. Answering a question does not mean the entry that
asked it was wrong, and `check` will reject a `resolves` aimed at an entry that
left no question open.

## Rules

- Prose, not bullet soup. Bullets for genuine lists only - field tables,
  enumerated options.
- No emojis anywhere in the log.
- Absolute facts over impressions. If something is a guess, label it a guess and
  say what evidence would confirm it.
- Keep numbers, names and paths exact. A wrong constant in the log is worse than
  no log.
- Record the *negative* results: the thing that turned out not to be what it
  looked like, the approach that failed, the code that turned out to be dead.
- If the entry records a behaviour, there should be a test that holds it. Say
  which test, by name, so the claim and its proof are linked.

<!-- cairns:project -->

## This project

retro_rt is a runtime, not a reverse engineering project: an entry here is
about the runtime's own design, a fault in it, or what a game needed from it.

- **Locators** are workspace paths and item paths: `crates/rrt-net/src/reliable.rs`,
  `rrt_net::Connection::transmit`. A fact learned from a consumer names the
  project and its path (`hwtr crates/hwtr-input/src/lib.rs`,
  `piney_apples crates/piney-game/src/input.rs`). Those repos are read, never
  edited from here.
- **Every behaviour claimed has a test**, named in the entry by its function
  name (`heavy_random_loss_still_delivers_everything_in_order`). A bug entry
  names the test that failed before the fix and passes after.
- **An entry that changes a public item is accompanied by the docs** (the
  `docs` skill): the crate's page and its `covers`, and
  `cargo run -q -p rrt-docs -- check` passing. The entry says why; the page
  says what.
- **Measurements** (packet sizes, frame times, test durations) give the
  machine, the build profile and the command that produced them.
- **Moving a game onto the runtime** goes under `port`, with what was
  replaced, what stayed in the game and why, and the game's `--shot` images
  or tests before and after.
