# Two host languages, and the seam between them

Quench's compiler is written in two languages on purpose.

- **Rust** — the frontend and the Dev JIT: lexer, parser, name resolution, type
  checking, borrow checking, diagnostics, QIR, Cranelift code generation, the
  program generator, and the oracle driver.
- **C++** — the LLVM half: the Hot JIT and the AOT native backend.

This is not a compromise between two options. Each half is written in the language
that owns its dependency. Cranelift *is* a Rust crate; using it from C++ would mean
inventing a binding layer for a library that has none. LLVM *is* a C++ library;
using it from Rust means either an incomplete binding crate or writing the C++
anyway and calling it. So: write each one where it lives.

## What that costs, and what it buys

It costs a seam. Two languages cannot share a type, so everything crossing between
them has to be written down.

It buys the thing that seam is for. The Dev JIT and the LLVM backends are not two
views of one data structure that happen to be compiled twice — they are two
consumers of one *specified* format. A backend cannot quietly depend on a frontend
detail that was never meant to be part of the contract, because it cannot see one.
For a project whose correctness standard is "three engines agree", a hard boundary
between the engines is worth more than the convenience it costs.

## The seam is QIR

**QIR** is Quench's typed SSA mid-level IR, and it is the only thing that crosses.

- It is **serialised**, little-endian, with a **version number in the header**. A
  backend that meets a version it does not know refuses the module rather than
  guessing.
- It is **fully typed and fully explicit**: no implicit conversion, no implicit
  drop, no implicit anything. Ownership has already been resolved by the time QIR
  exists — moves, borrows and drops are instructions, not properties to be inferred.
  A backend never runs an analysis to find out what the frontend meant.
- It carries **spans**, so a backend that has to report something can report it in
  the same format as everything else.
- Every change to QIR is a change in two languages. That is the price, and it is
  paid deliberately, in one place, with a version bump.

## How the halves are linked

The **AOT** path could be a separate process — frontend writes QIR, C++ reads it and
emits an object file. The **Hot JIT** cannot: it has to hand a *running* program over
to freshly compiled code, with live values crossing the join, which means it has to
be in the same address space.

So the C++ side builds as a **static library with a C ABI**, and the Rust driver
links it. Not C++ types across the boundary — a C ABI, with QIR bytes in and a
function pointer or an object file out. `cc`-crate-driven from `build.rs`, with
`llvm-config` locating LLVM (cmake and ninja are deliberately not required).

## Why the Dev JIT is the reference

Three engines that disagree do not tell you which one is wrong. Something has to be
the reference, and it should be the one built for clarity rather than speed:

- Cranelift compiles roughly an order of magnitude faster than LLVM, which is what
  makes the Dev JIT usable while editing;
- and it applies far fewer optimisations, which is exactly what makes it the better
  reference — fewer transformations means fewer places for a miscompile to originate.

So the Dev JIT defines the answer, and the LLVM engines are measured against it.
Where the LLVM engines are also *faster*, that is the point of them; where they
*differ*, that is a bug, and the oracle's job is to find it before a user does.

## Open

- The type system is not decided.
- The surface syntax is not decided. Placeholders in the tree are labelled as such.
- Whether AOT reuses the in-process library or runs as a separate tool is open; the
  C ABI works either way, so it does not need deciding yet.
