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

And `verify` runs on load. It exists already, written as an internal check on the
frontend, and its messages say so — they are addressed to whoever is writing the
compiler, because nothing else could produce a malformed module. **That stops being
true here.** A module that arrives from elsewhere and does not check out is a corrupt
or hostile file, not a compiler bug, and telling its reader that Quench has an
internal error would be a lie. The same check needs a second voice.

## What it does not change

The three engines still have to agree, and now they have to agree **across
machines**: the same artefact, run by two different engines on two different
processors, answers identically. Which is what the oracle was always for; the
artefact just makes the claim bigger.
