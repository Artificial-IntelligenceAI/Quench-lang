# Quench

Project under development. Nothing here is stable, and stability is **not a guarantee**.

**Quench** is a language focused on three things: **explicit syntax**, **okay
performance**, and **very helpful error messages** (unlike fucking `C`, just joking 🤣).

Quench compiles **once**, to one artefact, and that artefact runs on whatever
machine it lands on — the machine decides how. There are four ways, each with a job
none of the others does, and **all four must agree**:

| Way | Backend | Its job |
| --- | --- | --- |
| **Interpreter** | none | **Being believed.** It generates no code, allocates no registers and lowers nothing, so when the engines disagree it is the one that is right. It is also the quickest way to run a small program, because it skips the part that costs: compiling is roughly 352× running. |
| **Dev JIT** | Cranelift | **The edit loop.** 1.6 ms to compile, and within 1.4× of optimised LLVM on work that cannot be optimised. Deliberately at `opt_level = none`, which is what keeps it fast and what keeps it honest. |
| **Hot JIT** | LLVM | **Running a travelling artefact fast.** This is the one that exists *because* of compile-once-run-anywhere: take portability away and ahead-of-time output would do its job, so the artefact is what justifies it. |
| **AOT native** | LLVM | **Shipping.** Optimises fully and takes as long as it likes — nobody is waiting at a keyboard, and its compile time is spent once while its run time is spent by everyone. Also where the *anywhere* is spent rather than lost: the artefact is target-independent, so this turns it into a binary **for** any machine, at the last possible moment. |

Two of the four run today. See [Status](#status).

The artefact is serialised QIR, which is also what the C++ backend reads — one
format doing both jobs. Because it travels, QIR may not know what machine it is
for: no pointer width, no calling convention, no target-specific anything. See
[notes/compile-once-run-anywhere.md](notes/compile-once-run-anywhere.md).

Agreement between them is not a hope, it is a test. See [The oracle](#the-oracle).

## Hello, World

```quench
START {
    var.immut.str ['greeting'] = [*Hello*];
    var.immut.i64 ['answer']   = [*42*];

    print.stdout['greeting' str:*, World!* \n];
    print.stdout[str:*The answer is * 'answer' str:*.* \n];
}
```

```bash
quench run examples/hello.qnl     # the Dev JIT
quench walk examples/hello.qnl    # the interpreter
```

Both print the same thing, which is not a coincidence — it is
[the oracle](#the-oracle) applied to the smallest possible program.

## Status

| Part | State |
| --- | --- |
| Diagnostics (`quench-diag`) | Working — the error format, spans, and grapheme/byte/cell-correct columns |
| **Lexer** (`quench-lex`) | **Working** — tokens, comments, and diagnostics with recovery |
| **Parser** (`quench-parse`) | **Working** — `START`, declarations, `print`, and recovery at the semicolon |
| **Lowering** (`quench-lower`) | **Working** — the tree turned into QIR: `START`, `print`, text and escapes |
| **CLI** (`quench-cli`) | **Working** — `quench run`, `walk`, `check` |
| **Settings** (`quench-conf`) | **Working** — `QNL-Config.toml`, hand-read, with real diagnostics |
| **Type checker** (`quench-check`) | **Working** — names resolved, types checked, `i64` and `str` all the way down |
| Collector, stack maps | **Stage 1** — arrays allocate and nothing frees them yet. Written here, in Rust, not borrowed |
| **Numbers** (`quench-num`) | **Working** — `Big` unbounded integers (binary gcd, Knuth division) and `Exact` rationals behind `e` |
| QIR (`quench-qir`) | Seed — `i64` and `bool`, SSA with block parameters, verified before any backend sees it |
| **Interpreter** (`quench-interp`) | **Working** — QIR run directly, the engine that does the least |
| **Dev JIT** (`quench-dev`) | **Working** — QIR lowered by Cranelift and run in process |
| Hot JIT / AOT (LLVM, C++) | Not started |
| **Generator + oracle** (`quench-gen`) | **Working** — 200,000 programs checked across two engines in 5.5s, all cores |

## Decisions made

- **A program starts at `START`.** Named for what it does, rather than by
  convention. Nothing marks it and nothing is special about it otherwise: the
  compiler builds every function, then looks for that name.
- **The top level does not run.** A file is a list of declarations — functions,
  types, constants — and they are order-free, because none of them execute.
  Execution begins in `START` and nowhere else. See
  [notes/the-top-level-does-not-run.md](notes/the-top-level-does-not-run.md).
- **Declarations chain**, as Luarust's do: `var.mut.b16 ['x'] = [*1000*];`.
  Names in quotes, values between marks, semicolons at the end. See
  [notes/the-declaration-chain.md](notes/the-declaration-chain.md).
- **A Quench file is `.qnl`.** Unclaimed, and distinctive enough to search for —
  `.q`, `.qs`, `.qm` and `.qml` are all taken, and `.qn` collides with a common
  abbreviation.
- **Two marks**: `'a name'` and `*a written value*`. Whether a written value is
  text or a number is the *type's* question, not the mark's — `*1000*` is a number
  under `b16` and four characters under `str`. A written value is literal, and
  escapes stand outside it: `\n` is an item in the list, not a character hidden in
  the text. Items juxtapose to build a value, commas separate values. Where no
  chain supplies a type, the value carries it — `print.stdout[str:*Hello* 'name' \n];` —
  and a bare written value there is not valid. See
  [notes/what-the-marks-are-for.md](notes/what-the-marks-are-for.md).
- **Precedence stops where mathematics stopped.** `x` binds tighter than `+`, and
  comparison looser than both, because that was settled before computers existed.
  Everything programming invented — `mod` infix, `and` against `or`, bitwise — has no
  agreed order and takes brackets. C put `&` too loose and Python put it too tight, and
  both produced famous traps: the lesson is not that C chose wrong but that there was
  nothing to choose. See
  [notes/precedence-stops-where-maths-stopped.md](notes/precedence-stops-where-maths-stopped.md).
- **A `print` says where it goes**: `print.stdout[…]`, `print.stderr[…]`, and there
  is no default. Go's built-in `println` writes to standard error and nothing about
  writing it says so — a surprise a language can simply not have. Naming the
  destination only earns its place because there is more than one, so there are two.
- **Deciding.** `if 'n' > *10* { … } else-if … { … } else { … }`. The condition
  wears no brackets, because `[ ]` holds a *list* everywhere else and a condition is
  never one; it runs until the `{`, which is unambiguous. **`else-if` is one word**,
  so chaining and nesting are different syntax rather than the same syntax read two
  ways — the dangling-else problem does not arise. Nothing is truthy: a condition is
  a `bool` and there is no second way to be one.
- **Functions.** `fn.export.i64 ['add'] [immut.i64 'a', immut.i64 'b'] { give ['a' + 'b']; }`.
  The chain reads like a declaration's because it is one: who can see it, then what it
  gives back. **`nothing` is a word**, not a missing arrow — a reader should never have
  to read a body to find out whether there is an answer in it. A function that answers
  with something must answer on **every way out**, and an `if` counts only when it has
  an `else`: there is nothing honest to invent for the path that falls off the bottom.
  Parameters are declarations with `var` taken off, and `[]` is written even when empty.
  Calls cost no new syntax — `add[*1*, *2*]` is a call and `'xs'[*2*]` is an index,
  because names wear quotes. See
  [notes/what-a-function-has-to-say.md](notes/what-a-function-has-to-say.md).
- **Constants outside, functions at the top, variables inside.**
  `const.export.i64 ['LIMIT'] = [*100*];`. A constant has no storage — its value is
  written in wherever it is named — so `set` on one is refused and so is indexing one.
  No `mut`/`immut` link either: `const` is already that answer, which is the whole
  reason it is a different word from `var`. This is where `file` / `program` / `export`
  finally mean something, and they are required.
- **Looping.** `loop.temp.range.i64 ['i'] = [*1*, *5*] { … }` counts, both ends
  included; `loop.while 'delta' > *0* { … }` asks again before every pass. The rule
  between them is flat — **`range` always has a counter, `while` never has one** — and
  a counted `while` was dropped to keep it that way. A counter says how long it lives
  (`temp` or `perm`, required) and never whether it changes, because neither answer
  would be true: the loop moves it every pass, and nothing you write may. `set ['i']`
  is refused by name. `perm` keeps the counter afterwards holding the last value it
  took, which is the one thing in Quench that outlives its block and is the reason
  `break` is worth having. See
  [notes/a-counter-belongs-to-its-loop.md](notes/a-counter-belongs-to-its-loop.md).
- **`set` changes things**, and `mut` finally means something: `set ['total'] =
  ['total' + *55*];`, `set ['xs'[*2*]] = [*99*];`. Changing something not declared
  `mut` is refused, with the line that would have worked.
- **Arrays.** `var.immut.arr.i64 (2 3) ['m']` is one allocation of six, laid out row by
  row; `arr.arr.i64 (2 3)` is three allocations — two of three, and two handles over
  them. **Every `arr` link is one allocation**, sizes are spent one per link outside in,
  and the innermost takes what is left. Both are written flat and they print
  differently (`[1 2 3 4 5 6]` against `[[1 2 3] [4 5 6]]`), because only one of them
  can be taken apart: an index may stop where an allocation ends, and hands back the
  array that lives there. **A size may say `grow`** instead of a
  number — `arr.i64 (grow)`, `arr.arr.i64 (grow grow)` for rows of different lengths —
  which is not a second type but one more thing a size is allowed to say. `add ['xs'] =
  [*4*];` puts one on the end. Only the *first* size of an allocation may grow, because
  every other one is a stride. `count` folds to a number on a fixed array and costs one
  call on a growing one, which is the whole of what `grow` costs a reader. Holds any
  built type — `arr.bool`, `arr.str`, `arr.e` — and crosses into and out of functions,
  where the **call site** says `share` or `copy` so a reader knows what happens to their
  array without opening the function. The
  shape is written **without marks** — it is part of the type, and the `64` in
  `i64` wears none either. Counted from one, because an inclusive loop with an
  unsigned counter walks `[1, count]` exactly while `[0, count - 1]` wraps on an
  empty one. `print.stdout['xs']` shows everything it holds, flat however many
  dimensions it has, because flat is how the elements were written.
- **An array is the only thing two names can reach**, so it is the only thing that has
  to say which was meant: `['b'] = [share 'a']` makes a second name for one array,
  `[copy 'a']` makes a second array, and a bare `['a']` is refused. Both answers hide
  something and they hide opposite things — sharing hides a `set` here changing a thing
  there, copying hides a megabyte allocated on a line that looks free. Which is why
  `==` on two arrays can then mean the obvious thing, **their contents**: once *the
  same array* has a way to be said, the other question is free. See
  [notes/an-array-is-the-only-thing-two-names-can-reach.md](notes/an-array-is-the-only-thing-two-names-can-reach.md).
- **`e` never rounds.** `var.immut.e ['third'] = [*1* / *3*];` is a third, and times
  three is exactly one. `e:*0.1* + e:*0.2* == e:*0.3*` is **true** — a decimal point is
  exact here, which is the whole reason to write one. Arbitrarily large, so a 32-digit
  number squares to a 64-digit one with nothing lost. An `i64` and an `e` do not mix,
  because nothing converts on its own; `mod` is refused, because an exact division
  leaves nothing over. Every engine calls the *same* arithmetic, so the one addition
  that could have made them disagree cannot. See
  [notes/e-is-big-and-exact.md](notes/e-is-big-and-exact.md).
- **Arithmetic works.** `+ - x / mod` and the comparisons, with the precedence
  mathematics settled applied and everything else refused. `1 + 2 x 3` is 7;
  `10 mod 3 + 1` is an error offering both readings.
- **`and`, `or` and `not`** join and turn round `bool`s, and have **no agreed order**
  against each other or against a comparison, so brackets say what was meant:
  `[('n' > *0*) and ('n' < *9*)]`. Whether the right side is asked once the left has
  settled it is `[defaults] logic`, and it defaults to `stops-early` — not for speed.
  Quench stops rather than having undefined behaviour, so under `asks-both`
  `[('n' != *0*) and ((*100* / 'n') > *5*)]` does not waste a division, it **stops the
  program**. That setting was in the free pile until functions arrived and gave the
  right side something it could do.
- **`^` answers by squaring**, in the runtime rather than as an instruction — a power
  needs a loop, and two engines each writing their own would be two chances to write it
  differently. `[*2* + *3* ^ *2*]` is 11. An `e` takes a negative exponent and gives a
  ratio; an `i64` stops, because the answer to that is a fraction and this is not one.
- **The operators**: `+` `-`, `x`/`×` for multiply (never `*`, which is the
  written-value mark), `/`/`÷`, `mod`, `^`/`xx` for an exponent, and `<` `>` `</=`
  `>/=` `==` `!=`. Two spellings were not available rather than not chosen: `**`
  lexes as an *empty written value*, and `=` would have meant a declaration outside
  the brackets and a comparison inside them.
- **A declaration says whether it can change**: `var.immut.i64` or `var.mut.i64`,
  and silence is neither. The same rule as visibility, applied where it had not
  been — it was the one place left where not writing something still meant
  something.
- **Three visibilities**, on top-level declarations only: `file`, `program` and
  `export`. **Required** — there is no default, so a missing one is an error on the
  declaration rather than on some innocent use of it later. Words rather than
  initials, since the volume that would justify abbreviating is already gone.
  Variables never carry one, because nothing outside a function can name them
  anyway. See
  [notes/three-lines-a-name-can-cross.md](notes/three-lines-a-name-can-cross.md).
- **Constants outside, variables inside.** A constant is a value the compiler can
  work out; anything needing code to run to produce it would need that code to run
  before `START`, which is the model above, smuggled back in. So every variable
  lives inside a function.
- **Memory is collected.** A garbage collector, not ownership and not refcounting.
  Ownership makes the shape of your data a tree, and cycles, self-reference, caches
  and interning are not trees; the usual escape — an arena of integer indices —
  keeps the memory safety and loses the guarantee it was bought for. **Finalisation
  is not observable**, which is what lets three engines collect at different moments
  without that being a disagreement. And a collected language with no unsafe escape
  has no undefined behaviour, so the oracle is sound by construction rather than by
  care. Nothing ships to a program that never allocates. See
  [notes/the-collector-earns-its-place.md](notes/the-collector-earns-its-place.md).
- **Two host languages.** Rust for the frontend and the Cranelift Dev JIT; C++ for
  the LLVM Hot JIT and AOT native backend. They meet at a versioned, serialised IR
  rather than a shared header. See [notes/architecture.md](notes/architecture.md).
- **The error format comes from [Luarust](https://github.com/Artificial-IntelligenceAI/Luarust)**,
  unchanged in shape. Same author, same copyright holder, and it was already
  right. See [Credit](#credit).

## Decisions not made yet

- The **type system**.
- **How a running program fails** — how an error is signalled and propagated at run
  time, as opposed to how the compiler reports one. `START` returns an `i64` exit
  status for now, and that is waiting on this.
- **How a top-level function is declared.** The chain covers variables; there is
  nothing in Luarust to inherit for routines.
- Whether **`mut`** keeps that spelling, given visibility chose words over initials.
- **Whether there are modules inside a file.** The three visibilities assume a file
  is the unit of privacy; modules would add a rung.

## Types, iteration 1

| | |
| --- | --- |
| `b16` `b32` `b64` | IEEE 754 binary. `b64` is the widest for now — no `b128`, no `b256` |
| `d32` `d64` | IEEE 754 decimal |
| `u8` `u16` `u32` `u64` | unsigned integers, two's complement |
| `i8` `i16` `i32` `i64` | signed integers, two's complement |
| `e` | exact, unbounded **rationals**, for numbers too large to hold any other way. Never rounds — including on division. See [notes/e-is-big-and-exact.md](notes/e-is-big-and-exact.md) |
| `bool` | |
| `str` | |

There is no IEEE 754 for integers. The nearest standard is ISO/IEC 10967-1
(*Language Independent Arithmetic*), which covers bounded, unbounded and modulo
integers and is written to sit alongside IEEE 754 — but almost nothing cites it, and
the honest description is two's complement, which C only mandated outright in C23.

The standard would not settle the interesting question anyway. **How arithmetic
behaves — what overflow does, how division rounds — is a `QNL-Config.toml` setting**, and
those land in the semantic pile, so each one multiplies what the oracle has to prove.
See [Settings](#settings).

Only two of these allocate: `str`, and `e` because it is unbounded. Capping binary
floats at `b64` is what keeps the rest of them out of the heap.

## Errors

An error names the rule that was broken, points at the line, and ends with the fix,
because the fix is what should still be on screen when the reader stops reading.

```text
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qnl, line: 2, column: 10 (src/main.qnl:2:10)

`'name'` is declared twice.

  1 | var.immut.str ['name'] = [*Tankun*];
    |          ~~~~~~ declared here first, as `str`
  2 | var.immut.b16 ['name'] = [*1000*];
    |          ^^^^^^ and declared again here, as `b16`

Error code: E0201
Rule(s) broken: a name is declared once, and keeps the type it was declared with
Tip(s): a declaration always makes a new name. It never replaces one.
Suggested fix(s): rename one of them

1 error.
```

That is real Quench, and the rendering is asserted byte for byte in
`quench-diag`'s tests.

The greeting is printed once however many errors follow, and the count once at the
end, so a program with twelve mistakes apologises once rather than twelve times.

A position is reported three ways at once, because a position is three different
numbers and only one of them is the one a person means: the column a reader is given
counts **graphemes**, the column in `file:line:column` counts **bytes** so it can be
pasted into an editor or a `grep`, and the caret is placed by **terminal cells**, so
an emoji that draws two cells wide gets two carets.

## Settings

A project's settings live beside its source in a [`QNL-Config.toml`](QNL-Config.toml),
read by `quench-conf` — by hand, not by a library, because this file decides how every
source file in the project is built and so a mistake in it deserves the rule, the line
and the fix rather than `invalid value at line 4`.

Quench is meant to be very customisable — but settings come in two kinds, and only one
of them is cheap:

- those that change **what gets delivered** — embedded source, target CPU, which
  engine runs it — cost nothing to test, because the answer is the same either way;
- those that change **what a program answers** — how division rounds, what overflow
  does — multiply the oracle, because three engines have to agree under *each*
  setting, not once overall.

There is a third case in between, which the note describes: `[build] optimise` cannot
change what a program answers — every level must agree, and that is precisely what is
checked — but it does change what the *compiler* does, so sweeping it is free coverage
rather than a cost.

So the first kind can grow freely and the second is argued one knob at a time.

The first semantic one is here: `[defaults] division`, truncated or floored. It is
threaded all the way through — the generator picks a configuration per seed, both
engines carry it as **separate QIR instructions** rather than a mode they interpret,
and a disagreement names the settings it happened under. The oracle now proves two
languages instead of one — and three ways of running each, since the oracle sweeps
optimisation levels too. 200,000 programs, 600,000 comparisons, 8.3 seconds. See
[notes/every-knob-is-a-multiplier.md](notes/every-knob-is-a-multiplier.md).

## The oracle

A language with three execution methods has three places for the same bug to hide.
So the methods are not trusted, they are tested against each other:

- a **program generator** writes Quench programs that are guaranteed to compile —
  built from the types outward, so the interesting case (a program that *runs*, and
  can therefore be answered differently by two engines) is the only case generated;
- every program is run by **every method, at every optimisation level**, and the
  answers must match — **including the way a program stops**, since stopping in the
  same place for the same reason is as much an agreement as printing the same
  number. About one generated program in sixteen stops, and which stop it was is
  compared as strictly as any number;
- the generator is built to **saturate the machine it runs on** rather than testing
  one program at a time. Batching first — compiling a program costs about 352× what
  running it costs, so many programs go in one module and one compilation covers all
  of them — and then batches are *claimed* from a shared counter rather than dealt
  out, because this machine has fast cores and slow ones and a fixed share leaves the
  fast ones waiting.

Where it stands: **200,000 programs across two engines in 5.5 seconds**, 36,000 a
second, on ten cores. One worker manages 8,000, so the cores are worth 4.6×.

Any disagreement is a bug in at least one engine, and the seed that produced it is
kept so it can be replayed.

The oracle answers one question, though, and it is worth being clear about which.
Code that is never optimised is still **correct**: every engine agrees, every
generated program agrees, and the suite stays green while the output is a third
slower than it should be. Luarust shipped exactly that. So the optimised paths carry
their own guards, of the same shape — two things that must match, asserted to still
match — rather than benchmarks. See
[notes/passes-are-a-thing-you-have-to-ask-for.md](notes/passes-are-a-thing-you-have-to-ask-for.md).

## Building

```bash
cargo test
```

The LLVM half needs **LLVM 23** and is not wired up yet. When it is, `build.rs`
will find `llvm-config` from an environment variable and **assert its version**,
rather than trusting a path: Homebrew's `/opt/homebrew/opt/llvm` moves under you
on the next `brew upgrade`, which is exactly how the 22 that used to be written
here stopped being true.

## Licence

Copyright © 2026 Tankun Sriket.

Licensed under either of

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE), or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT), or
  <http://opensource.org/licenses/MIT>)

at your option. In SPDX terms: `MIT OR Apache-2.0`.

You do not have to comply with both. Pick whichever one suits you and comply
with that one.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 licence, shall
be dual licensed as above, without any additional terms or conditions.

### Provenance

`quench-diag` is derived from `luarust-diag` in Luarust, which is MIT and
copyright the same author. Relicensing it under the dual licence here is the
copyright holder's to do.

No third-party code is vendored into this repository — every file here was
written for it. There is one third-party *dependency*, Cranelift, which
`quench-dev` builds on and which brings a tree of sixty crates behind it. All of
them are permissive: **Apache-2.0 WITH LLVM-exception** for Cranelift and its
own dependencies, **MIT OR Apache-2.0** for most of the rest, and MIT, Zlib or
BSD-2-Clause for a handful. Nothing in the tree is copyleft and nothing in it
puts a condition on Quench's own licence. A binary that ships Cranelift inside
it carries their notices too; the source in this repository does not, because it
contains none of their code.

## Credit

Quench is one person's language. What it stands on is that person's own earlier
work, and two compiler backends written by other people.

### Luarust

Quench stands on [**Luarust**](https://github.com/Artificial-IntelligenceAI/Luarust),
the author's earlier language, now abandoned.

The licence permits reuse without acknowledgement. This is here anyway, because
the reuse is not incidental:

- **The error format is Luarust's**, and is carried over unchanged in shape —
  the greeting, the rule, the tip, the fix last, the primary and secondary
  labels, and the insistence on reporting a position three ways at once because
  a reader, a `grep` and a caret each need a different number. `quench-diag` is
  `luarust-diag` with the names changed.
- **The oracle is Luarust's idea too.** Generating programs from the types
  outward so that every one of them compiles, then insisting that every way of
  running a program agrees — including on how it *stops* — is the standard
  Quench inherited, along with the 200,000-program bar it has to clear.

Luarust is not maintained. Quench is where the work continued.

### Cranelift

The Dev JIT is [**Cranelift**](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift),
from the Bytecode Alliance. It is the only third-party dependency Quench has, and
it was picked for the thing it is best at: compiling fast enough that you do not
notice it happened. **1.6 ms** to get a small program from QIR to machine code,
and code within 1.4× of optimised LLVM on work that cannot be optimised — both of
those are Cranelift's numbers, not Quench's. See
[notes/what-the-dev-jit-costs.md](notes/what-the-dev-jit-costs.md).

Its IR is also why QIR looks the way it does. Block parameters instead of phi
nodes is Cranelift's answer to SSA, and Quench took it — which is why `if` and
loops needed no new IR between them, and why the two engines can be held to the
same construction rather than to two descriptions of it.

Licensed Apache-2.0 WITH LLVM-exception.

### LLVM

The Hot JIT and AOT native paths will be [**LLVM**](https://llvm.org), and the
reason is measured rather than assumed: Luarust's LLVM-compiled AOT output ran at
**1.001× C** on the benchmark that started this project. That is not "close to
C". That is C, with a different front end in front of it.

None of that half exists yet — no C++ is written and LLVM is not a dependency
today — so this credit is for a decision rather than for code. It is here because
the decision was load-bearing: three engines that must agree is a design that only
makes sense when the slowest one is worth waiting for, and LLVM is why it is. See
[notes/passes-are-a-thing-you-have-to-ask-for.md](notes/passes-are-a-thing-you-have-to-ask-for.md)
for what Quench has to be careful about when it gets there.

Licensed Apache-2.0 WITH LLVM-exception — the licence Cranelift's is named after.
