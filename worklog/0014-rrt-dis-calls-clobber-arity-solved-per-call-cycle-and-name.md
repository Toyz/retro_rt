---
number: 14
title: rrt-dis: calls clobber, arity solved per call cycle, and &name for an address
date: 2026-10-15
area: dis, bug, test
files: crates/rrt-dis/src/analysis.rs, crates/rrt-dis/src/pseudo.rs, docs/crates/rrt-dis.md
---

# 14. rrt-dis: calls clobber, arity solved per call cycle, and &name for an address

Three problems the rugrats port found in rrt-dis, each fixed with a test.

**A call clobbers.** `value_before` walked back across a `jal` as if
nothing happened. A store after a call through a base register built
before it read that old base, and Rugrats' `Program::slots` gained junk
slots at 0x53 and 0x55.

The walk now stops when it would cross a call's clobber point for a
register the calling convention says calls clobber. The clobber point is
the end of the delay slot: the delay slot runs before the callee, so a
value built there is still good, and `arg_at_call` asks for the callee's
entry and ignores its own call. Building data references now evaluates
the instruction on the values before it, rather than asking for the
value after it, which sits right at the clobber point for a delay-slot
`addiu`.

A last junk slot, `0x1b4e81fd`, came from a store whose base was stale;
stores now need a pointer-aligned slot, as loads already did. hwtr's
numbers are unchanged. On .hack, one jump that a stale register had
"resolved" is now honestly unresolved.

**Arity in a call cycle depended on order.** The cache seeded a function
being computed with "all arguments" and kept whatever came out, so
`fn_8002e67c` read differently depending on which function was decompiled
first. Arity is now solved per strongly connected component of the call
graph (Tarjan's algorithm, without recursion), callees first, as the
least fixpoint from no arguments. Two functions calling each other with
one argument now read as one argument each, in either order.

**`D_` named both an address and the memory at it.** In pseudo-code a data
symbol's memory is `name` and its address `&name`, as in C. A function's
address stays its name and a string's its literal.

**Still unknown:** nothing
