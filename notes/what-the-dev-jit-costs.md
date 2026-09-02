# What the Dev JIT costs

Measured against Luarust's LLVM JIT, on Luarust's own benchmark:
`sum = (sum + i) mod 1000000007`, a hundred million times. Each value needs the one
before it, so nothing can be vectorised or run out of order.

| 100M iterations | Quench Dev JIT (Cranelift, `opt_level = none`) | Luarust (LLVM) |
| --- | --- | --- |
| with the modulus | 3.04 ns/loop — **304 ms** | ~2.2 ns/loop — **~220 ms** |
| without it | 0.50 ns/loop — **50 ms** | **the loop is gone** |
| compiling | **1.6 ms**, measured in process | under 10 ms, including process start |

## Two numbers, and only one of them is a ratio

With the modulus, unoptimised Cranelift lands within **1.4×** of optimised LLVM. That
is much closer than "no optimisation at all" suggests, and the reason is that there is
nothing to optimise: a dependent chain through a 64-bit remainder is irreducible, the
division is slow enough to hide whatever was done around it, and both engines end up
waiting on the same instruction.

Without the modulus the comparison stops being a comparison. `sum = sum + i` has a
closed form, and LLVM finds it. Its emitted IR for a hundred million iterations is:

```llvm
define noundef i64 @luarust_main() {
entry:
  tail call void @luarust_print_value(i64 5000000050000000, i32 7)
  ret i64 0
}
```

It did not run the loop quickly. It did not run the loop. The Dev JIT ran all hundred
million iterations, at half a nanosecond each, which is what `opt_level = none` means
and is not a fault.

So **there is no single ratio between these two engines**, and quoting one would
mislead in both directions. The gap is however much structure an optimiser can find:
nothing when the work is irreducible, unbounded when the work can be deleted.

## What this says about the Dev JIT

It is doing its job. 1.6 ms to compile, and code within 1.4× of optimised LLVM on work
that cannot be optimised. Being fast to compile is the point, and being slow on
optimisable code is the price, and both showed up exactly where they should.

## What it says about the oracle

Both engines answered `5000000050000000`. One computed it a hundred million times and
one read it off a constant, and **the oracle passes**, correctly, because they agree.

That is not a hole to plug. It is [passes are a thing you have to ask
for](passes-are-a-thing-you-have-to-ask-for.md) demonstrated on real numbers rather
than argued: agreement testing is structurally blind to whether any optimisation
happened at all, and the guards for that are a different kind of test entirely.
