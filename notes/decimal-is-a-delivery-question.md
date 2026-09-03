# Decimal is a delivery question

`d32` and `d64` are IEEE 754 decimal. Neither is built. This note is the decision
about how they will be *provided*, written down before there is code for it, because
the answer turns out to be cheap and the reason it is cheap is easy to lose.

## Where it goes: `[build]` and `[run]`

```toml
[build]
decimal = "software"        # or "hardware" — for ahead-of-time output

[run]
decimal = "software"        # or "hardware", or the third one below — for the Hot JIT
```

The **Hot JIT** gets a third answer the AOT compiler cannot have: look at the machine
it is already running on and take the hardware when it is there. That is the whole
difference between them — the Hot JIT knows what it landed on, and ahead-of-time
output is aimed at a machine that may not be the one aiming.

The **Dev JIT** and the **interpreter** never consult either, and never will. The
interpreter generates no code, and the Dev JIT being the engine that did the least is
what makes it the one to believe when the three disagree. Both are software always,
for the same reason the Dev JIT stays at `optimise = "none"`.

The third value wants a name. `adaptive` says the mechanism rather than the behaviour,
which is the one thing
[name things for what they do](../README.md#decisions-made) argues against —
`whatever-is-there` or `ask-the-machine` say what happens. Not settled.

## Why this is in the free pile

**Delivery, not semantic.** It changes what gets built and how fast it runs, and never
what a program answers — so the oracle does not double, and this pile can grow freely.
See [every knob is a multiplier](every-knob-is-a-multiplier.md).

Which is worth being exact about, because there is a real reason it could have gone
the other way.

IEEE 754 specifies decimal arithmetic **completely**, the same way it specifies binary:
`+ − × ÷` and the comparisons are correctly rounded, so a conforming software
implementation and conforming hardware give the same *values*. There is nothing there
to disagree about.

But the standard specifies **two encodings** for those values:

- **BID**, binary integer decimal — the coefficient as a plain binary integer. What
  Intel's software library uses.
- **DPD**, densely packed decimal — the coefficient in ten-bit groups of three digits.
  What IBM's hardware uses.

Same numbers, different bits. So a program that could *see* the bits would get two
different answers from two conforming implementations, and this would be a semantic
setting after all — three engines to agree under each of two encodings.

**Quench has no bit-casting.** Nothing converts on its own, and there is no `as`, no
reinterpret, no way to ask a `b64` for its `i64`:

```text
this is a `b64`, and it is being given to an `i64`.
```

So an encoding is not something a program can observe, and the choice stays free.

**And that is a load-bearing dependency rather than a happy accident.** If Quench ever
gains a way to look at a value's bits, this setting moves piles — exactly as
`[defaults] logic` did when functions arrived and gave an expression something it could
*do*. A setting can move under you because the language grew a way to tell the
difference, and this is the second one where that is foreseeable rather than a surprise.

## What "hardware" actually means

Hardware decimal floating point exists on **IBM POWER6 and later**, and on **IBM
z/Architecture**. It is not on x86-64 and not on ARM64.

So on every machine this project is developed and tested on, `hardware` is unreachable
and the third value always picks software. That is not a reason to leave the setting
out — cross-compiled ahead-of-time output is exactly the case where somebody targets a
machine they are not sitting at — but it does mean the hardware path will ship
untested by the oracle until it runs somewhere with the instructions.

Which is the honest version of "no oracle risk": there is none *in principle*, because
the values are specified. There is the usual amount in practice, because an untested
path is an untested path.

## Not built

`d32` and `d64` themselves are not built, and the settings above are not in
`QNL-Config.toml`. A setting that parses and does nothing would be worse than one that
is refused, since the file's whole job is to decide how a program is built. They arrive
with the types.
