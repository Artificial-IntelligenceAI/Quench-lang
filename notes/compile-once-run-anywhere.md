# Compile once, run anywhere

Quench compiles to one artefact, and that artefact runs on whatever machine it lands
on. Java's bargain, and Luarust's before this. The machine it arrives at decides
*how* it runs — the Dev JIT or the Hot JIT — and **AOT native is the one that trades
the anywhere away**, for a binary that needs nothing installed.

The artefact is **serialised QIR**. Not a second format invented for the purpose: the
same thing the C++ backend reads, doing both jobs.

## What that costs the IR, and it is not nothing

A portable artefact means **QIR cannot know what machine it is for**:

- no pointer width, no word size, no host-sized integer;
- no ABI decisions — calling conventions, struct layout, register classes belong to
  whichever backend eventually runs it;
- no target-specific types, and no assumption about endianness in the *program*,
  whatever the file's own encoding is.

Every one of those is easy to let in by accident and very hard to take back out once
a file exists that somebody else compiled. This is a constraint on the IR as it is
being designed, not a property of how it is written to disk.

The file itself is **little-endian everywhere**, so the encoding is one thing rather
than a property of the machine that wrote it.

## What it costs the schedule

An earlier draft of `quench-qir`'s documentation said serialisation could wait,
because there was only one consumer and a format wants a second reader before it is
designed. That was wrong, and wrong in an expensive direction: **portability is the
second reader**, and it has been there since the beginning. The requirement is real
now, not imagined, so the format can be designed properly now.

*(And then it waited anyway, for longer than it should have, while types and arrays
and a collector went in — until it was noticed that the C++ backend cannot begin,
because its input is a file that did not exist. It exists now.)*

## Reading a file somebody else made

The moment an artefact travels, it stops being something this compiler produced and
starts being **input**. Two different things follow.

A chunk carries **a sum of its own bytes**, checked before a single field of it is
believed. That is for accidents — a bad copy, a transfer that stopped early, a disk
going wrong — and it says so, rather than range-checking a damaged file field by
field into some plausible program nobody wrote. It is explicitly **not** a defence:
anyone editing a chunk on purpose recomputes it in a line.

And `verify` runs on load. It was written as an internal check on the frontend, and
its messages said so — addressed to whoever is writing the compiler, because nothing
else could produce a malformed module. **That stops being true the moment a module
travels.** So the findings are data now, and an `Audience` decides who is being told:

- `Ourselves` — this compiler built it, nothing else could have, so it is a bug in
  Quench. The reader is told their program is probably fine and asked to report it.
  `E9001`, and E9xxx is the range for *not your program's fault*.
- `AFileWeWereGiven` — it arrived. A copy that stopped early and a module built by
  another version of Quench both look exactly like this, so the fixes offered are
  build it again from source, or check it was transferred whole. `E0801`.

Both come out as ordinary diagnostics, in the format every other Quench error uses.
A reader should not have to learn a second one because the trouble is in a file
rather than in a line — which meant the renderer had to survive a diagnostic with no
labels at all, since a module is not a line of source and there is nothing to point
a caret at. It does, and there is a test that says so.

## Where the portability actually lives

Ahead-of-time output is not portable, and that is the trade. One machine, one binary,
needing nothing installed.

But it is worth being exact about what is given up and when, because the first draft of
the README got this slightly wrong. **The portability is in the artefact, not in the
compiler and not in the binary.** QIR knows nothing about any machine, so a compiled
artefact can be carried anywhere and turned into native code *for* anywhere, at any
later moment. Ahead-of-time output is not where the *anywhere* is lost. It is where it
is **spent**, at the last possible point, on purpose.

Which means cross-compiling is a normal thing rather than a special one: the same
artefact, a different target. LLVM as built here emits for twenty architectures, so the
code generation half costs nothing.

The other half is what the language cannot supply for you, and Luarust found both:

- **A linker carrying a libc for the target.** `zig cc` does this for most targets and
  is looked for first; a `<triple>-gcc` after it.
- **The Quench runtime, built for that target.** This one is larger here than it was
  for Luarust, because Quench collects: the collector is a Rust `staticlib` and it has
  to exist compiled for every machine you want to reach.

And a consequence of shipping nothing a program does not use, which cuts the right way:
**a program that never allocates carries no collector**, so cross-compiling it needs no
runtime archive at all. The simple programs stay simple to send somewhere else, and
only the allocating ones need the per-target build.

## What it does not change

The three engines still have to agree, and now they have to agree **across
machines**: the same artefact, run by two different engines on two different
processors, answers identically. Which is what the oracle was always for; the
artefact just makes the claim bigger.

---

## The file

```
"QNL\0"  version                       4 + 4
then, for each section:
  kind  length  sum-of-body  body        4 + 8 + 8 + length
```

Four sections — the text a program was written with, the constant tables, the
functions, and which function is the entry. Every number little-endian, whatever
machine wrote it and whatever machine reads it, so the encoding is one thing rather
than a property of who was holding the pen.

`quench build hello.qnl` writes `hello.qnlo`, and `quench run` takes either.
`examples/functions.qnl` is 1,672 bytes as an artefact.

### The sum is for accidents, and says so

FNV-1a, five lines, no dependency. A chunk's body is checked before a single field of
it is believed — a copy that stopped early, a transfer that went wrong, a disk going
bad. It is **not** a defence: anybody editing a chunk on purpose recomputes it in a
line, and the code says so rather than implying otherwise.

### The codes are written out, not derived

Every enum in the file has its numbers listed one by one rather than taken from the
order the variants happen to be declared in. A file outlives the source that wrote it,
so moving a variant must not silently change what an old file means. There is a test
that round-trips every type, every operation, every comparison and every runtime call,
which is what says none was missed.

### Reading is total

No reader panics, none indexes without looking, and a counted run is checked against
what is actually left before a single byte is reserved for it — so a file claiming four
billion of something is refused rather than asking for the memory to hold them. An
enum code nobody has heard of is an error, not a guess.

And `verify` runs on load with the audience switched, so a module that arrived gets
`E0801` and the two fixes that are actually available — build it again, or check it was
copied whole — rather than `E9001`, which asks somebody to report a bug in Quench.

### The oracle reads it too

Every module the generator makes is written to bytes and read back before anything
runs it, and the programs are run from what came back. Two thousand round trips per
sweep, 200,000 programs, and a format that lost something would show up as a wrong
answer rather than as a broken file. It costs nothing measurable.

That is the second reader the note wanted, arriving before the C++ one — and it is why
the format can be trusted before a line of C++ exists to test it against.
