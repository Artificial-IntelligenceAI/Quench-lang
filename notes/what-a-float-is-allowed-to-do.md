# What a float is allowed to do

"Floats are non-deterministic" is the received wisdom and it is mostly wrong, which
matters here more than in most projects: three engines have to agree bit for bit.

**IEEE 754 fully specifies `+ − × ÷ √` and the comparisons** under round-to-nearest-
even. Every conforming implementation gives the same bits. There is nothing there to
disagree about. What diverges is a short, nameable list — and almost every item on it
is something a compiler does **only when asked**.

| | Where it comes from | What Quench does |
| --- | --- | --- |
| fusing `a×b + c` into one rounding | LLVM, given `contract` or fast-math. Cranelift does not | never sets a fast-math flag |
| 80-bit intermediates | 32-bit x86 using the x87 stack | x86-64 uses SSE2, ARM64 has no such thing |
| denormals flushed to nought | `MXCSR` FTZ/DAZ, set by fast-math | never sets it |
| which not-a-number you get | the bit pattern is not fully specified | a program cannot see it: no bit-casting, and printing gives a word |

Every one of those is the same rule the project already holds elsewhere —
[passes are a thing you have to ask for](passes-are-a-thing-you-have-to-ask-for.md) —
applied to arithmetic. **Fast-math is not a setting and will not become one.** It is
the one knob that would break the thing the whole project is for.

## The two that are actually hard, and are not here

**Transcendentals.** `sin`, `exp`, `pow` and `log` are *not* required by IEEE 754 to
be correctly rounded, and every library differs in the last bit. So `b64` does not
have them, and `^` on one is refused with the reason. When they arrive they go where
the exact arithmetic went: one implementation in `quench-num`, called by every engine,
which is the arrangement that makes disagreement impossible rather than unlikely.

**Printing.** Shortest-round-trip against `printf %.17g` against a language's own
`Display` all differ. `quench_num::show_f64` is the only one, for the same reason
`show_array` is.

It always writes a point — `1.0` rather than `1` — so what is shown says which type
it came from. `infinity`, `-infinity` and `not-a-number` are words, which is also
what stops a program from seeing a not-a-number's bits.

## `[defaults] no-number`

```toml
[defaults]
no-number = "carries-on"   # infinity and not-a-number, and the program continues
no-number = "stops"        # stop, in the same place in every engine
```

**Semantic**, so it multiplies the oracle like `division`, `overflow` and `logic`.
`*1.0* div *0.0*` is `infinity` under one and a stop under the other.

`carries-on` is the default, and the argument is different from the one for
`overflow = wrap`. It is not that it is free (though it is): **`b64` *is* IEEE 754
binary64**, and `infinity` and `not-a-number` are values of that type rather than
accidents of it. Asking to stop is asking for something narrower than the type you
named, which is a thing to opt into rather than out of.

## What the oracle gets

Something better than usual. Floats are compared **by their bits**, and a generated
program full of float arithmetic either produces the same 64 bits in every engine or
it does not. That catches contraction the instant anybody turns it on, which is a
sharper check than most of what the oracle does.

## Why `e` still exists

```quench
[e:*0.1* + e:*0.2* == e:*0.3*]     # true
[b64:*0.1* + b64:*0.2* == b64:*0.3*]  # false, and the sum is 0.30000000000000004
```

Both are right. `e` never rounds and pays an allocation and a gcd for it; `b64`
rounds the way the standard says and stays in a register. The second line is not a
bug in `b64` — it is what a binary float is, and getting it wrong identically
everywhere is the whole of what a standard buys.
