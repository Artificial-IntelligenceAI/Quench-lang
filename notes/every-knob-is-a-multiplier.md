# Every knob is a multiplier

Quench is meant to be very customisable, and settings for a project live beside its
source in a `QNL-Config.toml`. Luarust's shape is inherited: `[defaults]` for what gets
accepted, `[build]` for what gets delivered, sections for how it runs — and a
`defaults.` line at the top of a file still wins for that file, because whatever a
file says about itself is the last word on it.

The thing worth writing down is not the file format. It is that **settings do not
all cost the same**, and the expensive ones are not the ones that look expensive.

## Two piles

| | changes what a program answers? | what it costs |
| --- | --- | --- |
| `division`, `overflow`, float printing | **yes** | multiplies the oracle |
| `optimise` | no — every level must answer the same | free, and worth sweeping anyway |
| `embed-source`, `target-cpu`, gc mode, which engine runs it | no — same answer, different artefact | covered once |

The middle row is a refinement this note did not have at first, and it came from
watching an optimiser delete a loop. `optimise` does not change what a program *means*
— every level must produce the same number, and that requirement is the whole point of
it. But it does change **what the compiler does**, so a bug that only appears at one
level exists and is only found if something compiled at that level.

So the pile splits by a different question than the one it looked like:

- **Does it multiply the specification?** `division` does: each value is a different
  language, and each has to be proven separately. That is what costs confidence.
- **Does it multiply the code path?** `optimise` does: same specification, different
  route through the backend. That costs only time, and buys real coverage.
- **Neither?** `embed-source` produces identical machine code either way. Test it once.

The second pile can grow as large as it likes. A program built with its source
embedded and one built without give the same answer; only the file differs. Testing
one is testing both.

The first pile is different in kind. `division = "floored"` and
`division = "truncated"` disagree about `-7 / 2`. That is not one language with a
flag. It is two languages, and Quench's correctness bar — three execution methods
that agree — has to be met by **each of them separately**.

## The arithmetic of it

Three engines is the baseline. Add division with two settings and there are six
things to keep in agreement. Add overflow with two more and there are twelve. Every
semantic knob multiplies, and a bug that only appears under `wrap` is found only if
something generated a `wrap` configuration to look under.

So a handful of semantic settings, each chosen because it earns its place, stays a
proof. A dozen of them is 3 × 2¹² and the oracle quietly stops being a proof and
becomes a lottery — while still passing, which is the dangerous part.

None of that argues for fewer settings. It argues for knowing which pile a setting
lands in *before* adding it, because the cost is paid in confidence rather than in
compile time, and confidence is not something a benchmark will tell you that you
have lost.

## What follows

**The generator generates configurations, not just programs.** A seed picks a program
*and* a semantic configuration; all three engines run under it; a disagreement report
names both. Anything less leaves whole settings untested while the suite stays green.

**A compiled artefact records the semantic settings it was built under.** Otherwise
two files that disagree about division link together into a program where `/` means
different things in different functions, and nothing anywhere is wrong enough to
report.

**The reader is written here, not taken from a library.** Luarust's reason was
weight: a project that will not put a collector on a device that does not need one
should not put a deserialiser in a toolchain that does not need one either. Quench
has that reason and a better one. This file decides how every source file in the
project is built, so a mistake in it deserves the same error a mistake in a source
file gets — the rule that was broken, the line, and the fix. A TOML library says
`invalid value at line 4`, and that is the wrong voice for the most consequential
file in the project.

## Not decided

Which knobs exist. The delivery pile can be settled late and cheaply. Each addition
to the semantic pile is a decision to test twice as much for ever, so those get
argued one at a time.
