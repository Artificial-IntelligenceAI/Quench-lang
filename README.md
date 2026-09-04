# Quench

Project under development. Nothing here is stable, and stability is **not a guarantee**.

**Quench** is a language focused on three things: **explicit syntax**, **okay
performance**, and **very helpful error messages** (unlike fucking `C`, just joking 🤣).

Quench compiles **once**, to one artefact, and that artefact runs on whatever
machine it lands on — the machine decides how. There are four ways, each with a job
none of the others does, and **all four must agree**:

| Way | Backend | Its job |
| --- | --- | --- |
| **Interpreter** | none | **Being believed.** It generates no code, allocates no registers and lowers nothing, so when the engines disagree it is the one that is right. It is also the quickest way to run a small program, because it skips the part that costs: compiling is roughly 352x running. |
| **Dev JIT** | Cranelift | **The edit loop.** 1.6 ms to compile, and within 1.4x of optimised LLVM on work that cannot be optimised. Deliberately at `opt_level = none`, which is what keeps it fast and what keeps it honest. |
| **Hot JIT** | LLVM | **Running a travelling artefact fast.** This is the one that exists *because* of compile-once-run-anywhere: take portability away and ahead-of-time output would do its job, so the artefact is what justifies it. |
| **AOT native** | LLVM | **Shipping.** Optimises fully and takes as long as it likes — nobody is waiting at a keyboard, and its compile time is spent once while its run time is spent by everyone. Also where the *anywhere* is spent rather than lost: the artefact is target-independent, so this turns it into a binary **for** any machine, at the last possible moment. |

Two of the four run today. See [Status](#status).

The artefact is serialised QIR, which is also what the C++ backend reads — one
format doing both jobs. Because it travels, QIR may not know what machine it is
for: no pointer width, no calling convention, no target-specific anything. See
[notes/compile-once-run-anywhere.md](notes/compile-once-run-anywhere.md).

Agreement between them is not a hope, it is a test. See [The oracle](#the-oracle).

**This file is a test too.** Every `quench` block in it is compiled, every error it
shows is produced and compared character for character, and every inline snippet that
is a whole statement is checked — so a claim here that stopped being true fails
`cargo test` rather than sitting until somebody notices. What is left is prose, which
nothing can check and which is therefore the only part to read sceptically.

## Hello, World

```quench
START {
    print.stdout[str:*Hello, World!* \n];
}
```

```text
Hello, World!
```

One line, and four of Quench's arguments are already in it. `START` is where a program
begins, because `main` says nothing. `print` says **where it goes** — there is no default
stream. `str:` says the value is text, because `*1000*` is a number under one type and
four characters under another. And the text wears `*` marks, which is what lets a name
hold a space, an apostrophe or an emoji everywhere else in the language.

```bash
quench run examples/hello.qnl     # the Dev JIT
quench walk examples/hello.qnl    # the interpreter
quench build examples/hello.qnl   # write the artefact — `run` takes that too
```

Both print the same thing, which is not a coincidence — it is
[the oracle](#the-oracle) applied to the smallest possible program.

## Status

| Part | State |
| --- | --- |
| Diagnostics (`quench-diag`) | Working — the error format, spans, and grapheme/byte/cell-correct columns |
| **Lexer** (`quench-lex`) | **Working** — tokens, comments, and diagnostics with recovery |
| **Parser** (`quench-parse`) | **Working** — the whole language, and recovery at the semicolon |
| **Lowering** (`quench-lower`) | **Working** — the checked tree turned into QIR, settings written in as instructions |
| **CLI** (`quench-cli`) | **Working** — `quench run`, `walk`, `check`, `build` |
| **The artefact** (`quench-qir`) | **Working** — QIR written down and read back, checked the way an arrival is |
| **Settings** (`quench-conf`) | **Working** — `QNL-Config.toml`, hand-read, with real diagnostics. Six semantic knobs, so the oracle proves sixty-four languages |
| **Type checker** (`quench-check`) | **Working** — names resolved, types checked; every number type, `str`, `bool` and `arr` all the way down, and `stitch`, `is` and `as` for the conversions |
| **Collector** (`quench-heap`) | **Stage 2** — mark and sweep in both engines, nothing moving. Written here, in Rust, not borrowed |
| **Numbers** (`quench-num`) | **Working** — `Big` unbounded integers (binary gcd, Knuth division), `Exact` rationals behind `e`, `Decimal` behind `d32` and `d64`, and the half of IEEE 754's maths the standard actually requires |
| **QIR** (`quench-qir`) | **Working** — nine types, SSA with block parameters, verified before any backend sees it, and written down |
| **Interpreter** (`quench-interp`) | **Working** — QIR run directly, the engine that does the least |
| **Dev JIT** (`quench-dev`) | **Working** — QIR lowered by Cranelift and run in process |
| Hot JIT / AOT (LLVM, C++) | Not started |
| **Generator + oracle** (`quench-gen`) | **Working** — 200,000 programs, three ways each, every module round-tripped through the artefact |

## Decisions made

- **A program starts at `START`.** Named for what it does, rather than by
  convention. Nothing marks it and nothing is special about it otherwise: the
  compiler builds every function, then looks for that name.
- **The top level does not run.** A file is a list of declarations — functions,
  types, constants — and they are order-free, because none of them execute.
  Execution begins in `START` and nowhere else. See
  [notes/the-top-level-does-not-run.md](notes/the-top-level-does-not-run.md).
- **Declarations chain**, as Luarust's do: `var.mut.i64 ['x'] = [*1000*];`.
  Names in quotes, values between marks, semicolons at the end. See
  [notes/the-declaration-chain.md](notes/the-declaration-chain.md).
- **A Quench file is `.qnl`.** Unclaimed, and distinctive enough to search for —
  `.q`, `.qs`, `.qm` and `.qml` are all taken, and `.qn` collides with a common
  abbreviation.
- **`#` comments a line, `#3` comments three.** The count includes the line it is
  written on, so `#3` covers this one and the two under it. Digits stuck to a word are
  refused rather than guessed at — `#3rd attempt` is neither a count nor a comment, and
  reading it as either would eat two lines nobody offered or ignore a number somebody
  wrote on purpose. **A space says comment**: `# 3rd attempt` is about a third attempt.
  A count of nought is refused, and so is one reaching past the end of the file.
- **Two marks**: `'a name'` and `*a written value*`. Whether a written value is
  text or a number is the *type's* question, not the mark's — `*1000*` is a number
  under `i64` and four characters under `str`. A written value is literal, and
  escapes stand outside it: `\n` is an item in the list, not a character hidden in
  the text. Items juxtapose to build a value, commas separate values. Where no
  chain supplies a type, the value carries it — `print.stdout[str:*Hello* 'name' \n];` —
  and a bare written value there is not valid. See
  [notes/what-the-marks-are-for.md](notes/what-the-marks-are-for.md).
- **A name holds whatever a line holds, and wears marks everywhere.** The marks do the
  delimiting, so there is no identifier grammar to break: `'ผลลัพธ์'`, `'🔥'`,
  `'a name with spaces'` and `'it\'s'` are all names, and all of them are fine names for
  a *function* too, because a call wears the marks like every other use of a name.
- **A call says `call`.** `call 'double'[*2*]`, never `'double'[*2*]` — because a name
  before a bracket is otherwise an index, and which one a line meant would depend on a
  declaration somewhere else. Quench asks for the word for the same reason it asks for
  `immut`, `share` and `nothing`: **a meaning carried by an absence is one a reader has
  to go and look up.** The marks then say only who made the thing being called —
  `call count['xs']` came with Quench, `call 'count'['xs']` did not — so a function and
  a variable may share a name, and nothing the language provides is reserved.
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
  `call 'add'[*1*, *2*]` is a call and `'xs'[*2*]` is an index, told apart on the line
  rather than by a declaration elsewhere. Arguments take commas, because juxtaposition
  already builds one value out of pieces and cannot also separate two; an index writes
  its dimensions side by side instead, matching the shape it indexes into. See
  [notes/what-a-function-has-to-say.md](notes/what-a-function-has-to-say.md).
- **A function may leave the type to its caller, and the hole is a bare word.**
  `any` takes all sixteen types; `number` takes the number ones. Both are Quench's
  words, like `arr` and `nothing`, because **a hole is not a name** — it names nothing,
  it is a place a type goes, so it never wanted marks.

  ```quench
  fn.file.number ['bigger of'] [immut.number 'a', immut.number 'b'] {
      if 'a' > 'b' { give ['a']; } else { give ['b']; }
  }

  START {
      print.stdout[call 'bigger of'[i64:*3*, i64:*9*] str:* * call 'bigger of'[b64:*2.5*, b64:*1.5*] \n];
  }
  ```

  ```text
  9 2.5
  ```

  There is **one hole per function** and every mention of it is the same one, which is
  what makes `[immut.any 'a', immut.any 'b']` two of the same thing. Nothing at the call
  says a type: the argument says it, the way an argument always has. What each hole
  allows is exactly what *all* the types filling it allow — so `any` gets `==` and
  little else, `number` gets `+` and `<`, and neither gets `mod` or `^`, because those
  are refused on some numbers.

  **A length is a hole too.** A size is part of the type, so an `arr.i64 (3)` was not an
  `arr.i64 (grow)` and a function taking an array had to name its length — which would
  have made `largest` a function per length. `(any)` says the number was never told to
  whoever wrote this, so one `largest` takes an `arr.i64 (3)`, an `arr.i64 (5)` and an
  `arr.i64 (grow)` alike. It is deliberately not `grow`: a growing array may be **added
  to**, and an array that merely arrived may not be assumed to be one.

  It **compiles away**. A generic function is a pattern, and the checker writes out one
  real function per type it was used at, so QIR and both engines never learn the word.
  That is forced rather than chosen: a slot is an `i64` whatever is in it, and one copy
  serving every type would have to tag every value so the collector knew what to follow.
  See [notes/a-hole-is-not-a-name.md](notes/a-hole-is-not-a-name.md).
- **A module is a block inside a file, and it nests.** What it does is decide which
  names reach which code; below the checker a function is just a function with a longer
  name, so nothing that runs knows modules exist.

  ```quench
  module.file ['maths'] {
      fn.module.b64 ['reduce'] [immut.b64 'x'] { give ['x' + *1.0*]; }

      module.file ['trig'] {
          fn.parent.b64 ['quarter turn'] [immut.b64 'x'] { give ['x' / *2.0*]; }
      }

      fn.export.b64 ['sin'] [immut.b64 'x'] {
          give [call 'reduce'[call 'trig'.'quarter turn'['x']]];
      }
  }

  START {
      print.stdout[call 'maths'.'sin'[*8.0*] \n];
  }
  ```

  ```text
  5.0
  ```

  The ladder is five, narrowest first: **`module`** (this module and everything nested
  inside it), **`parent`** (the module around this one, and everything under that),
  `file`, `program`, `export`. `module` reaching downward is what the whole thing is
  for — a helper in `maths` called from `maths.trig` — and the consequence, that a
  parent cannot see *into* a child, is what `parent` answers. `parent` goes one rung up
  and no further: Rust's `pub(in path)` is declined, because wanting to reach three
  levels up is evidence the nesting is too deep.

  **A module says who may see it too**, like everything else at the top of a file:
  `module.file ['maths']`, and `module.module ['trig']` for one that is an
  implementation detail of the module around it. A name never reaches further than the
  module around it does, so `'maths'.'trig'.'x'` asks three questions rather than one.

  **A program can be several files**, and `[program.files]` in `QNL-Config.toml` says
  which — the manifest is the authority on what the program *is*, and `import` in a file
  says which of them *it* uses, so a reader sees where a name came from without leaving
  the line. A file is a module, so two files may each hold a `'size'`.

  ```toml
  [program.files]
  main = "main.qnl"
  maths = "arithmetic.qnl"
  ```

  The name a file is imported by is **chosen there, not taken from the filename**:
  `arithmetic.qnl` is the module `maths`. A filename is not an interface, so renaming the
  file breaks no caller, a file may sit in a directory without the directory leaking into
  the name, and two files may share a stem.

  Then `main.qnl` opens with `import ['maths'];` and reaches into it with
  `call 'maths'.'sin'[*8.0*]`. There is no block for that here because a file which
  imports is not a program on its own, and every `quench` block in this file is compiled
  on its own — the whole thing is [examples/program](examples/program), which the test
  suite runs both ways and compares.

  That is what finally makes **`file` mean something**: until there could be a second
  file there was nowhere for it to be false. `program` and `export` still cannot be told
  apart, and now for a reason one step further out — telling *them* apart wants a second
  **program** using this one as a library.

  A path is **marks, dots, marks** — `call 'maths'.'trig'.'quarter turn'[…]`, and
  `['text'.'MARK']` for a constant in another module — because
  the rule is the one `call` already had: a bare word is Quench's, a marked name is
  yours, and every link of one path says the same thing about who made it. A name is
  looked for here and then outward, paths included, so `maths` says `'trig'.'…'`
  without saying `maths` again. See
  [notes/five-lines-a-name-can-cross.md](notes/five-lines-a-name-can-cross.md).
- **Constants outside, functions at the top, variables inside.**
  `const.export.i64 ['LIMIT'] = [*100*];`. A constant has no storage — its value is
  written in wherever it is named — so `set` on one is refused. A constant *array* does
  have somewhere it lives: a table in the module beside the text, which every engine
  lays out before the entry runs, so its handle is known while compiling and naming it
  costs nothing. There is one of it, so `share` names that one and `copy` gives you one
  you may change.
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
- **Pieces side by side join.** `[*Hello, * 'name' *!*]` builds a `str`, which is what
  juxtaposition has meant since the marks were settled — what is new is that a piece
  may not be known until the program runs. Nothing converts on its own, so a number
  among them is an error: `print` shows any type because showing is not joining, and
  writes one piece after another rather than making one.
- **`b64`, `b32` and `b16` are IEEE 754 and nothing else.** `+ − x /` and the comparisons, which the
  standard fully specifies — so every engine gives the same bits. What would break that
  is what a compiler does only when asked: fusing a multiply into an add, keeping extra
  precision, flushing denormals. **Fast-math is not a setting and will not become one.**
  `^` works on a `b64` and is worked out here rather than asked of a library; `mod` is
  refused, because a float division answers with the nearest float and leaves nothing
  behind to ask about — `call remainder['a', 'b']` is the question IEEE defines for
  floats, and its answer is exact. `[defaults] no-number` says whether `infinity` and `not-a-number` are answers
  or stops. A `b16` is **carried in a `b32`** — no machine Quench targets has a half —
  and gives binary16's own answers anyway, because one wider operation rounded once to
  binary16 *is* the correctly-rounded binary16 answer when the carrier has `2p + 2`
  bits, and `f32` has exactly the 24 that binary16's 11 asks for. See
  [notes/what-a-float-is-allowed-to-do.md](notes/what-a-float-is-allowed-to-do.md).
- **Every whole-number type rides in an `i64`**, held normalised — sign-extended when
  signed, zero-extended when not — so comparing and printing need know nothing about
  width. What makes a `u8` a `u8` is being **put back inside it after every operation**,
  and under `overflow = "trap"` that is where one which reached 256 stops rather than
  becoming nought. A narrow type finds its own overflow that way; only `u64` needs the
  operation itself to notice, and it is also the only one whose *comparison*, *division*
  and *printing* have to read the bits as unsigned.
- **`d32` and `d64` round in the base they were written in.** Not more accurate than a
  `b64` — differently wrong, in the direction a person reading the number expects:

  ```quench
  START {
      var.immut.d64 ['a'] = [*0.1*];
      var.immut.d64 ['b'] = [*0.2*];
      var.immut.b64 ['x'] = [*0.1*];
      var.immut.b64 ['y'] = [*0.2*];
      var.immut.d64 ['exact'] = ['a' + 'b'];
      var.immut.b64 ['near']  = ['x' + 'y'];
      print.stdout[str:*d64  * 'exact' str:*  b64  * 'near' \n];
  }
  ```

  ```text
  d64  0.3  b64  0.30000000000000004
  ```

  A `d64` keeps sixteen significant digits and a `d32` seven, and both keep the cohort
  they were given: `*2.50* + *1.00*` is `3.50` and not `3.5`, because a trailing zero in
  decimal is a statement about precision. Dividing by nought is `infinity` rather than a
  stop, which is the difference between a float and an `e`. `^` and `mod` are refused
  for the same reason they are on a `b64`. Both engines call the *same* arithmetic, so
  they cannot round differently.
- **The maths IEEE 754 requires, and no more.** `sqrt`, `abs`, `floor`, `ceil`,
  `round`, `trunc`, `copysign`, `min`, `max`, `fma`, `remainder`. The standard *requires* these to be
  correctly rounded, so every engine gives identical bits and there is nothing for the
  oracle to catch. `round` breaks ties to the **even** one — `*2.5*` is `2` and `*3.5*`
  is `4` — because that is `roundToIntegralTiesToEven` and not what most languages'
  `round` does. `min` and `max` are 754-2019's `minimumNumber` and `maximumNumber`: a
  not-a-number loses to a real number rather than poisoning it — **and which of those
  it does is `[defaults] min-max`**, because both are somebody's idea of right and the
  standard specifies both. `remainder` is the odd
  one: its answer is **exact**, never rounded, which is why it can be checked against
  arithmetic that does not round at all rather than trusted.
- **`exp`, `ln` and `pow` are worked out rather than asked for.** IEEE only
  *recommends* correct rounding for these, which in practice means every C library is a
  little bit wrong in its own way — so three engines calling three libraries would be
  three answers. Quench computes them itself instead, in a float as wide as the answer
  turns out to need, and rounds exactly once. **Twenty thousand arguments against this
  machine's own `libm`: `ln` agreed everywhere, `exp` and `pow` differed on about one in
  seven hundred — and at three hundred bits the true value was on our side every time.**
  **It is slow**: four to thirteen microseconds a call against one to five nanoseconds
  for the machine's own, so a few thousand times. That is down from seventy microseconds
  — most of the first draft was recomputing `ln 2` and π on every call, and then asking
  the allocator for a three-limb number several hundred times per call. Closing the rest
  would mean a fast path accurate enough to *prove* its own rounding, which is a real
  piece of work rather than a tuning pass. The reference engine is the one that does the
  least and being right was the point, so the bill is paid. `b64` only: rounding a `b64` answer down to a `b32` rounds twice.
  `sin`, `cos`, `tan`, `atan` and `atan2` are here too, and the argument reduction is
  the part a C library cannot afford: knowing which quarter-turn `1e300` falls in means
  knowing π to a thousand bits. Here π is a `Big` and it is asked for as many bits as the
  argument has exponent, so a sine of ten to the three hundredth is as exact as a sine of
  a half. Against this machine's `libm` over five thousand arguments each — and adjudicated
  at four hundred bits, which settles it past any doubt:

  ```text
  asin   differed on    98 of 2000   nearer: ours    98, platform     0
  acos   differed on   262 of 2000   nearer: ours   262, platform     0
  atanh  differed on   665 of 2000   nearer: ours   665, platform     0
  cbrt   differed on   153 of 2000   nearer: ours   153, platform     0
  sin    differed on    78 of 2000   nearer: ours    78, platform     0
  tan    differed on   872 of 2000   nearer: ours   872, platform     0
  ```

  Seventeen of them altogether: `exp`, `ln`, `sin`, `cos`, `tan`, `atan`, `atan2`,
  `asin`, `acos`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`, `cbrt`, `hypot`.
  **A power is not among them, because `^` already is one** — it works on `i64`, on `e`
  and on `b64`, and a second name for it would be a second spelling of an operator.
- **`call count['s']` counts characters, and what a character *is* is a setting.**
  `count['café']` is 4 either way. `count['🧑‍🧑‍🧒‍🧒']` is **1** under
  `characters = "clusters"` and **7** under `"letters"`, because that emoji is seven
  scalars welded together with zero-width joiners and one thing on the page. `clusters`
  is the default and is the whole of UAX #29, pinned to Unicode 17.0.0 and checked
  against Unicode's own 766-case conformance suite. What it costs is that the answer is
  tied to a Unicode version; `letters` is one scalar, needs no table, and never changes.
- **`call stitch[…]` is how a number becomes text**, and it is the only conversion in
  the language. Juxtaposing text with a number is refused — nothing converts on its own
  — and this is how a program says *do it anyway*; the word being written is what makes
  it a request rather than a guess. It takes the list a `print` takes, pieces side by
  side, of any types:

  ```quench
  START {
      var.immut.i64 ['n'] = [*42*];
      var.immut.d64 ['due'] = [*7.00*];
      var.immut.str ['line'] = [call stitch[*item * 'n' *, * 'due' *!*]];
      print.stdout['line' \n];
  }
  ```

  ```text
  item 42, 7.00!
  ```

  What it builds is what a `print` would have written, character for character — each
  is the same expression as the other, so a `d64` keeps its cohort here too. Without it
  a program could *show* a number and never hold the text of one, which left no way to
  build a message, a filename, or a line for a file.
- **`call as.i64[…]` goes the other way, and `call is.i64[…]` is the check you are
  expected to have made.** Bad input is the one failure a program has to survive, and
  Quench has no value that is either an answer or a reason — that wants generics. So it
  does what it already did for arrays: **you ask first.**

  ```quench
  START {
      var.immut.str ['line'] = [*42*];
      if call is.i64['line'] {
          var.immut.i64 ['n'] = [call as.i64['line']];
          print.stdout[str:*read * 'n' \n];
      }
  }
  ```

  ```text
  read 42
  ```

  `as.i64` on `hello` **stops the program**, exactly like `'xs'[*99*]` does — a writer
  who asked never reaches it, and one who did not has a bug rather than bad luck. These
  are the only two things the language provides that carry a chain, because they are the
  only two that cannot work the type out: text says nothing about what it holds, and
  `12` is an `i64`, a `b64`, a `d32` and an `e`.

  What each accepts is **the text a value of that type could have been written with** —
  one function decides both, so `*200*` being a `u8` and not an `i8` is one rule and not
  two. `infinity` is not a `b64` here for the same reason it is not one in a source
  file. See
  [notes/checking-comes-first.md](notes/checking-comes-first.md).
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
  `[('n' !== *0*) and ((*100* / 'n') > *5*)]` does not waste a division, it **stops the
  program**. That setting was in the free pile until functions arrived and gave the
  right side something it could do.
- **`^` answers by squaring**, in the runtime rather than as an instruction — a power
  needs a loop, and two engines each writing their own would be two chances to write it
  differently. `[*2* + *3* ^ *2*]` is 11. An `e` takes a negative exponent and gives a
  ratio; an `i64` stops, because the answer to that is a fraction and this is not one.
- **One spelling per operator, and it is the one on your keyboard**: `+` `-` `/` `^`
  `<` `>` `<==` `>==` `==` `!==`, plus **`x`** for multiply because `*` is the
  written-value mark and nothing else on a keyboard means multiply. There are no
  alternates: the wide symbols, the doubled letters and the spelled-out names all went.
  Two spellings for one thing is a decision a reader has to make for no reason.
- **`==` reaches the comparisons that include it.** `<==` and `>==` and `!==`, because
  `=` assigns and `==` is equal to — a comparison carrying a bare `=` would be the one
  thing `==` exists to avoid, hidden inside a longer token rather than standing alone.
  `<=` is named rather than read as a comparison and then an assignment.
- **Words are for what programming invented**, and only that: `mod`, `and`, `or`, `not`
  have no symbol because nothing ever settled where they bind, which is the whole of
  [notes/precedence-stops-where-maths-stopped.md](notes/precedence-stops-where-maths-stopped.md).
  Things get a symbol by being settled long enough for one to stick, so the two rules
  turn out to be one rule.
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
  **Both engines collect** — mark and sweep, nothing moving, one heap shared between
  them because an object model is a contract rather than each engine's own idea. Every
  array carries what its slots hold, since a slot is an `i64` whatever is in it. The
  interpreter's roots come off its own call stack; the Dev JIT has no such list, so
  every reference-typed value in a function gets a slot in a frame the runtime owns,
  written where the value is made — no stack maps, no unwinding, and nothing to keep in
  step with a code generator. It costs nine per cent of the oracle.
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
- **How a failure carries a reason.** *That* a program fails is settled — it stops, and
  a check comes first — but nothing yet carries *why* back to a caller, and files will
  want it, because no check can be made honest against a world that changes underneath
  it. `START` returns an `i64` exit status for now.
- Whether **`mut`** keeps that spelling, given visibility chose words over initials.
- **Whether a library can be something other than source.** An import reads `.qnl`,
  because a generic is a pattern and its body has to be at the call site. A `.qnlo`
  artefact cannot carry one, so shipping a compiled library is its own question.
- **A shorter name for a path.** Something binding `'maths'.'exact'` to a local alias.

## Types, iteration 1

| | |
| --- | --- |
| `b16` `b32` `b64` | IEEE 754 binary, all three built. `b64` is the widest — no `b128`, no `b256` |
| `d32` `d64` | IEEE 754 decimal, both built — in software, and whether they *stay* software is a *delivery* setting rather than a semantic one, because no program can see an encoding. See [notes/decimal-is-a-delivery-question.md](notes/decimal-is-a-delivery-question.md) |
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

Three of these allocate: `str`, `e` because it is unbounded, and `d32`/`d64` because a
coefficient and an exponent do not fit in a register on any machine Quench targets.
Capping *binary* floats at `b64` is what keeps those out of the heap.

## Errors

An error names the rule that was broken, points at the line, and ends with the fix,
because the fix is what should still be on screen when the reader stops reading.

```quench
START {
    var.immut.str ['name'] = [*Tankun*];
    var.immut.i64 ['name'] = [*1000*];
}
```

```text
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qnl, line: 3, column: 20 (src/main.qnl:3:20)

`'name'` is declared twice.

  2 |     var.immut.str ['name'] = [*Tankun*];
    |                    ~~~~~~ declared here first, as `str`
  3 |     var.immut.i64 ['name'] = [*1000*];
    |                    ^^^^^^ and declared again here, as `i64`

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

There are four semantic ones: `[defaults] division` (truncated or floored), `overflow`
(wrap or trap), `logic` (stops-early or asks-both) and `no-number` (carries-on or
stops). Each is threaded all the way through — the generator picks a configuration per
seed, both engines carry the choice as **separate QIR instructions** rather than as a
mode they interpret, and a disagreement names the settings it happened under. So the
oracle proves sixty-four languages rather than one, three ways of running each, since it
sweeps optimisation levels too. 200,000 programs, 600,000 comparisons, 28 seconds.

`logic` is the one worth reading the note for: it was **not** a semantic setting until
functions arrived, because before a program could call anything, nothing inside an
expression could *do* anything and both answers gave byte-for-byte identical programs.
A setting can move piles under you — not because anyone changed it, but because the
language grew a way to tell the difference. See
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
  one program at a time. Batching first — compiling a program costs about 352x what
  running it costs, so many programs go in one module and one compilation covers all
  of them — and then batches are *claimed* from a shared counter rather than dealt
  out, because this machine has fast cores and slow ones and a fixed share leaves the
  fast ones waiting.

Where it stands: **200,000 programs across two engines in 28 seconds**, 7,100 a second,
on ten cores. One worker manages 1,200, so the cores are worth 6x.

It was four times that rate a day ago, and the slowdown is the point: what a generated
program *does* has grown. It was arithmetic and loops; it now allocates arrays, reads
`e` and decimals, joins text, and prints — and every one of those is compared, so every
one of them costs. A fast oracle that generates a narrow program proves less per second
than a slow one that generates a wide one.

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
and code within 1.4x of optimised LLVM on work that cannot be optimised — both of
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
**1.001x C** on the benchmark that started this project. That is not "close to
C". That is C, with a different front end in front of it.

None of that half exists yet — no C++ is written and LLVM is not a dependency
today — so this credit is for a decision rather than for code. It is here because
the decision was load-bearing: three engines that must agree is a design that only
makes sense when the slowest one is worth waiting for, and LLVM is why it is. See
[notes/passes-are-a-thing-you-have-to-ask-for.md](notes/passes-are-a-thing-you-have-to-ask-for.md)
for what Quench has to be careful about when it gets there.

Licensed Apache-2.0 WITH LLVM-exception — the licence Cranelift's is named after.
