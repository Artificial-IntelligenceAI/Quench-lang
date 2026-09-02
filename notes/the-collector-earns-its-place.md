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

## Written here, not borrowed

MMTk was the alternative and was looked at properly: a Rust toolkit, a dozen plans
from MarkSweep to generational Immix, bindings proven against OpenJDK, V8, Ruby and
Julia. It asks a runtime for five traits — `ObjectModel`, `Scanning`, `Collection`,
`ActivePlan`, `ReferenceGlue` — and says of itself that it "is not yet ready for
production use".

Quench writes its own. The other Rust collectors on offer were never candidates
regardless, and not because they are weaker: `gc-arena`, `rust-gc` and the rest
collect *Rust values*, needing Rust's type system to see an object through a trait
and a derive. Quench's heap is walked by generated machine code, which cannot
participate in any of that.

**The staging MMTk recommends is worth keeping even without MMTk**, because it
defers the hard part rather than merely postponing it:

1. **Allocate, never collect.** A bump pointer and no collector at all. Needs an
   object header and nothing else — no roots, no stack maps, no backend work.
2. **Mark and sweep.** Now roots matter, and stack maps with them. But **nothing
   moves**, so there is no relocation: no `gc.relocate` in LLVM, no forwarding
   pointers, no updating a root after a collection. The single hardest thing in this
   note does not appear at this stage.
3. **Move things.** Compaction, or generations. Now relocation is real and statepoints
   have to be right.

Which means a working collector arrives before LLVM statepoints are touched at all.

What blocks step 1 is the **object header** — a mark bit, and enough to find an
object's type so its references can be traced.

With iteration 1's types that is very nearly nothing. Binary floats stop at `b64` and
integers at 64 bits, so every number fits in a register and never reaches the heap.
**Two things allocate: `str`, and `e`.**

And `e` is where the interesting question is. It is exact unbounded rationals, so it
is a numerator and a denominator, each of them arbitrarily large — and *how that is
laid out decides whether the collector has edges to follow at all*:

- **One allocation**, with both magnitudes stored inline, is a leaf. The collector
  marks it and never looks inside. Tracing has no edges anywhere in the language.
- **Three allocations** — a rational pointing at two big integers — and `e` becomes
  the first thing in Quench that contains references. Tracing has edges, `ObjectModel`
  has real work, and this arrives long before compound types do.

The first is simpler for the collector and worse for arithmetic, since every result
of a different size means copying rather than sharing. The second is the usual
implementation and costs the collector its holiday. Not decided.

## Testing something the oracle cannot see

Twice now a class of bug has turned out invisible to three engines agreeing. A
missing pass pipeline produces correct code — see
[passes are a thing you have to ask for](passes-are-a-thing-you-have-to-ask-for.md).
A collector bug is worse: because collection is unobservable *by design*, a program
that frees a live object does not disagree with anything. It reads a value that was
correct until a moment ago, and every engine may do it identically.

So the collector carries its own guards, and they are not the oracle's:

- **Collect at every safepoint**, under a setting. Most GC bugs are a race between
  an object dying and a root being missed; collecting constantly turns "rare and
  unreproducible" into "deterministic and on the first test".
- **Poison what is freed**, so a use-after-free reads an obviously wrong value
  rather than a plausible stale one.
- **Walk the heap after each collection** and check it is well formed — every live
  object's references point at live objects.
- **Miri over the unsafe core.** A collector is unsafe code by nature; Rust's
  contribution is that the unsafe parts are *delimited* and Miri can interpret them
  looking for exactly the aliasing and pointer-arithmetic mistakes this code makes.

The pattern is the same one as everywhere else here: do not assert that an answer is
good, assert that something which must hold still holds.

## What this does not change

Constants outside and variables inside stays, and the argument for it never depended
on ownership: an initialiser that needs code to run would need it to run before
`START`. See [the top level does not run](the-top-level-does-not-run.md).
