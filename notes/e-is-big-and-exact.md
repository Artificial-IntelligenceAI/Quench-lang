# `e` is big and exact

`e` exists to hold absurdly large numbers and represent them exactly. Both halves,
not one — exactness is why it never rounds, and unboundedness is what it is *for*.

## It is wired up, and here is what that took

`e` is a type you can write now:

```quench
var.immut.e ['third'] = [*1* / *3*];        # a third, and it stays a third
var.immut.e ['back']  = ['third' x *3*];    # exactly one
var.immut.bool ['ok'] = [e:*0.1* + e:*0.2* == e:*0.3*];   # true
```

That last line is the whole reason the type exists. A decimal point is **exact**
here: `0.1` is one tenth, not the `b64` nearest to it.

An `e` is a handle, like an array is. The arithmetic is seven host calls —
`exact-read`, four operations, one comparison, one print — and every engine calls the
**same Rust** for all seven. That is the part worth noticing: an `e` is the one
addition to the language that could plausibly have made two engines disagree, and it
cannot, because there is only one implementation of it to disagree with. Six
comparisons became one call, since comparing two exact numbers is the sign of their
difference and every comparison is that sign against zero.

Three things fell out of rules already in the language:

- **Nothing converts on its own**, so an `i64` and an `e` do not mix. They are both
  numbers and they are not the same number.
- **`mod` is refused.** A remainder is what a division left over, and an exact
  division leaves nothing. `mod` belongs to the number types that round, which is
  every one of them but this.
- **A chain says what its numbers are.** `var.immut.e [...] = [*0.1*]` reads one
  tenth because the chain said `e`; the same marks under `i64` are an error. Where
  the chain cannot say — a comparison under a `bool` chain — the value says it
  itself, `e:*0.1*`, which is what a `print` list has always let a value do.

Division by zero stops the program and is reported rather than aborting, like every
other trap.

## It is one allocation — eventually

**Not yet.** What runs today is a `Vec<Exact>` per engine, each `Exact` two `Big`s,
each `Big` a `Vec<u64>` of its own. Three allocations where the design below wants
one, and a handle is an index into that vector. Allocated and never freed, which is
the first stage of the collector and the same stage arrays are at.

The design the rest of this section describes is still the target. It is written
here as a plan, and the plan has not been carried out.

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
written for numbers that stay small. **Two of these three are done**:

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

That order was followed. `quench-num` has binary gcd and Knuth's algorithm D, and
its multiplication is still schoolbook — which is the one the table calls third and
the one a reader would have guessed was first.

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
