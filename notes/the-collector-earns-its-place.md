# The collector earns its place

Quench collects. Ownership, moves and a borrow checker were the earlier decision and
are gone.

## What ownership could not do

Ownership requires the shape of your data to be a tree, and five common shapes are
not one:

- **Cycles** — doubly-linked lists, parent pointers, back-edges, observers.
- **Self-reference** — a value holding a pointer into its own buffer, which moving
  invalidates. This is why Rust needs `Pin`, and why async Rust is the hardest part
  of Rust.
- **Aliasing that is safe but not provable.** A borrow checker is sound, not
  complete: it rejects correct programs. `&mut v[0]` and `&mut v[1]` are obviously
  disjoint and refused anyway.
- **Lifetimes that are not structural** — caches, interning, memoisation, widget
  trees. "Until something says otherwise" is not a scope.
- **Shared mutable state**, which is not forbidden but is not granted either.

The usual escape is an arena with integer indices, and it is worth naming honestly:
that replaces a pointer with an index, so a dangling index becomes a *logic* bug the
checker cannot see. The memory safety survives and the guarantee it was bought for
does not.

## What it costs, and what turned out not to cost anything

**The collector is the work.** Writing one is the project; the backends are the
smaller half.

**Stack maps come from two backends.** Cranelift's 2024 rework moved the complexity
into the frontend — the mid-end, backends and register allocator no longer track GC
references, so it is an API on the `FunctionBuilder` already in use here. LLVM has
`gc.statepoint`, stable and unchanged since at least LLVM 9, but its emitted stack
map is explicitly not in a form a collector can use: the runtime parses it at load
time and re-encodes.

Which points where the IR contract already pointed. **Both backends normalise into
one Quench stack-map format**, versioned alongside QIR, and the collector reads only
that. A collector that understood Cranelift's format *and* LLVM's would be two
parsers that must agree — the shape of bug that cannot be tested for.

**Shipping a collector costs nothing to programs that do not allocate.** This looked
like a cost and is not: nothing goes onto a machine unless the program uses it, so a
program that never allocates carries no collector, exactly as Luarust's `[gc] mode`
already works.

## The rule that makes it possible

**Finalisation is not observable.** No destructors whose *timing* a program can
notice.

This is not a preference, it is what keeps the oracle meaningful. Three engines
running the same program will collect at different moments — different stack maps,
different safepoints, different heuristics. If a program could observe when, that
difference would be a legitimate disagreement, and the oracle's verdict would stop
meaning "one of these is wrong".

Make collection unobservable and the engines need not agree about it at all. Only
each be correct. Unlike codegen, there is nothing here to keep in sync — which is
the hardest part of a GC across three backends, removed by a language rule rather
than by engineering.

## The unexpected win

A borrow checker would have needed an escape hatch — raw pointers, or something like
them — because a language with none cannot express a doubly-linked list. And an
escape hatch can produce undefined behaviour.

**UB is where a differential oracle stops working.** A program with UB entitles every
engine to answer differently, so a disagreement no longer means a bug. Csmith goes to
great lengths to emit only UB-free C for exactly this reason.

A collected language with no unsafe escape has no such hole. Every generated program
is one where disagreement is always a bug, and `quench-gen` needs no rule about which
parts of the language to avoid. The oracle is sound by construction rather than by
care.

## What this does not change

Constants outside and variables inside stays, and the argument for it never depended
on ownership: an initialiser that needs code to run would need it to run before
`START`. See [the top level does not run](the-top-level-does-not-run.md).
