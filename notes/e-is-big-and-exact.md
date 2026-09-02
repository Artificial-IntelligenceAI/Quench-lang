# `e` is big and exact

`e` exists to hold absurdly large numbers and represent them exactly. Both halves,
not one — exactness is why it never rounds, and unboundedness is what it is *for*.

## It is one allocation

A rational is a numerator and a denominator, each arbitrarily large. They live in one
block, contiguous, not as a rational pointing at two separately allocated integers.

The reason is not that copying is cheap. It is that:

- `a + b` computes a new numerator *and* a new denominator, then reduces, so both
  parts are new and there is nothing left to share;
- assignment shares the whole object anyway, since values are immutable and
  collected, so the pointer form buys nothing there either;
- big-number arithmetic is **memory-bandwidth-bound**. A large multiply is limited by
  how fast digits stream, not by the multiplies. One contiguous block with both
  magnitudes adjacent is what you want; chasing two pointers into separate
  allocations is what you do not.

The bigger the numbers, the more that argues for one block. It also leaves `e` a
**leaf** as far as the collector is concerned — no references inside, nothing to
trace, anywhere in the language. See
[the collector earns its place](the-collector-earns-its-place.md).

## What ports, and where it stops

Luarust already wrote this: `luarust-num` is 1,634 lines, with `Big` — sign and
little-endian 64-bit limbs, no leading zero limb, zero always positive — and `Exact`,
a numerator over a denominator, always in lowest terms, sign on the numerator,
denominator strictly positive. One representation of every value, so equality is
comparison rather than arithmetic. That is careful work and it should be taken.

What should not be taken uncritically is the arithmetic underneath it, because it was
written for numbers that stay small:

| | Luarust's | what "absurdly big" wants |
| --- | --- | --- |
| multiply | schoolbook, O(n²) | Karatsuba above a crossover, then Toom-Cook |
| divide | shift-and-subtract, one bit at a time — its own comment admits this is slower than schoolbook | Knuth algorithm D |
| gcd | Euclid, on that division | binary GCD, which needs no division at all |

**Fix them in the opposite order to that table.** `Exact` normalises eagerly, so a
gcd runs after every single operation — which means the slowest routine in the crate
is on the hot path of everything. Binary GCD removes division from that path
entirely and is the largest win available. Division is second, and multiply, despite
being the famous one, is third.

## Eager or lazy, and why it is not obvious

Luarust reduces to lowest terms after every operation, and says why: there is one
representation of each value, so two rationals are equal exactly when they are the
same number. That is a real property and it makes equality trivial.

It also means gcd runs constantly. The alternative is to reduce only when someone
looks — printing, comparing, or when a denominator has grown past some bound. That
trades canonical form away for far less gcd, and equality becomes cross-multiplication
rather than a memcmp.

For numbers that stay small, eager is obviously right. For the numbers `e` is
actually for, it is not obvious at all. Undecided.

## What it is not

`e` is not the numeric type a program reaches for. `b64`, `i64` and friends are, and
they stay in registers and never allocate. `e` is the one you ask for when the answer
has to be right regardless of size, and it costs an allocation and a gcd to be right.
