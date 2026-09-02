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

## What it does not change

The three engines still have to agree, and now they have to agree **across
machines**: the same artefact, run by two different engines on two different
processors, answers identically. Which is what the oracle was always for; the
artefact just makes the claim bigger.
