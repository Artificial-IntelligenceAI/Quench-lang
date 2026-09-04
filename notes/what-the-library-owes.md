# What the library owes

Quench has fifty-three runtime services and a namespace with twenty-eight of them in it. This note is a list of what
is missing, what each thing is waiting on, and the order that costs least — written
before there is code for any of it, because most of the list is blocked on three
decisions and it is worth seeing that before writing the easy parts. All three have
since been answered; each section below says what the answer was.

## The hole that matters most — filled

A number could not become text.

```quench
var.immut.i64 ['n'] = [*42*];
var.immut.str ['said'] = [*n is * 'n'];
```

That is E0411, *"this is an `i64`, and text is made of text"*, and it is correct — the
rule is that nothing converts on its own, and this is a conversion. But it means a
program can `print` a number and cannot *hold* the text of one. No message a user
writes, no line they log, no file they produce.

The formatting already exists. `PrintI64`, `PrintU64`, `PrintFloat`, `PrintExact` and
`PrintDecimal` each turn a number into exactly the characters it should be; they write
them to a stream rather than handing them back. What is missing is the same work
returning a `Text` instead of a stream. Cheapest thing on this list by a wide margin,
and the one most in the way.

### `stitch`

It takes the list a `print` takes:

```
var.immut.str ['said'] = [stitch[str:*n is * 'n' *!*]];
```

Which is the whole of it: `print.stdout[…]` and `stitch[…]` read the same list of
pieces, of any types, side by side. One writes them and the other builds them. Two
destinations, one implementation, and the grammar is one somebody has already learnt.

Doing it in one step rather than two is the point. A `convert` that turned one number
into one piece of text would leave every message as a conversion and then a join and
then another join, and the joins are the part Quench already does.

`join` was not free: `Value::Join` and `Host::TextJoin` are what juxtaposition compiles
to, text beside text. That is a genuinely different operation from building text out of
anything, and one word for both would blur two rules that are deliberately apart.

### What it does to "nothing converts on its own"

It does not break it, and the reason is already written in the error it replaces.

The rule is against **silent** conversion — a number wandering into a `str` because
something guessed. A `print` is exempt today because showing is not joining. A `stitch`
is exempt for a stronger reason: the word is in the source. It is a request, said out
loud, the way `call count['xs']` asks for a length rather than an array quietly becoming a
number.

So the rule reads: nothing converts on its own, and `stitch` is how a program says *do
it anyway*. Written down here because the next person wanting an implicit conversion
will point at this one, and the answer to them is that theirs has nowhere to say so.

## Three decisions the rest waited on

### Is a `str` bytes, or is it characters? — answered

Characters, and a character is a grapheme cluster.

`call count['café']` is 4 and `call count['🔥']` is 1, because that is what a person
counting them means. It is not free: it costs a walk of the whole string to answer
anything, where bytes would have made `count` a subtraction. And "character" itself had
two answers, since a scalar value and a cluster differ on exactly the emoji people test
with — so **both are built**, `[defaults] characters` picks between them, and the
oracle checks the language under each. `count['🧑‍🧑‍🧒‍🧒']` is 1 through one and 7
through the other.

Quench was already leaning this way without having said so: a name may hold anything a
line can hold, which the marks decided by doing the delimiting.

### How does something fail without stopping? — answered

It does not. It says beforehand whether it is going to.

A trap still ends the program, and there is still no recovering from one anywhere. What
changed is that every conversion now ships with the question that guards it, the way
indexing has always shipped with `count` and division has always shipped with a divisor
you can look at:

```quench
if call is.i64['line'] {
    var.immut.i64 ['n'] = [call as.i64['line']];
}
```

`as.i64` on `hello` stops, exactly like an index off the end, and a writer who asked
first never reaches it. No new statement, nothing added to a signature, and the oracle
does not grow — which the alternatives all did, since a value that is either an answer
or a reason wants generics of the kind Quench still has none of — a type that is one
thing *or* another. The holes it has since grown are the other kind.

The reasoning, the four shapes turned down, and the one place this genuinely will not
reach — files, where a check cannot be made honest — are in
[checking comes first](checking-comes-first.md).

### Which maths is Quench allowed to have?

`^` and `mod` are refused on binary floats already, because no standard requires a
`pow` to round the same way twice —
[what a float is allowed to do](what-a-float-is-allowed-to-do.md). The same argument
sorts the whole of a maths library into two piles, and the line is sharp:

**IEEE 754 requires these to be correctly rounded**, and all of them are built: `sqrt`,
`fma`, `abs`, `floor`, `ceil`, `round`, `trunc`, `copysign`, `min`, `max`, `remainder`.
Every engine must agree, so they cost the oracle nothing. (`min` and `max` are specified too, but 754-2019 has four of
them because the 2008 pair handled not-a-number badly, so one has to be picked
deliberately.)

**IEEE 754 only recommends these**, and no library delivers it: `sin`, `cos`, `tan`,
`log`, `exp`, `pow`, `atan2`. Three engines calling three C libraries is three answers.

`exp`, `ln` and `pow` are **built**, the way `e` and decimal are: one implementation in
Quench's own tree that every engine calls. They are worked out in a float as wide as the
answer needs and rounded once, and the rounding is only accepted when every value in the
interval the answer might be in rounds the same way — Ziv's strategy, which makes them
provably correctly rounded rather than probably. `sin`, `cos`, `tan`, `atan` and `atan2` are built too. Their argument reduction is the
part a library cannot afford — which quarter-turn `1e300` falls in needs π to a thousand
bits — and here π is a `Big` asked for as many bits as the argument has exponent. `asin`, `acos` and the hyperbolics followed once the wide float grew a square root —
eighteen functions in all, and every one of them checked by asking which `b64` the true
value is *nearer*, worked out four hundred bits wide so the comparison itself does not
round.

## Where the maths lives — behind `maths`

Twenty-eight of the thirty-two provided functions are maths. `count`, `stitch`, `is` and
`as` are the only four that are the language rather than a library sitting inside it, and
a reader of that list would reasonably conclude Quench is a calculator with a compiler
attached. `sin` read like a keyword.

```quench
call maths.sqrt[*2.0*]     # Quench's module, Quench's function
call 'maths'.'sqrt'[*2.0*] # a module somebody wrote and called maths
```

Bare, because it is Quench's, and the marks rule tells the two apart the way it always
has. It cost five characters at every use site and it was the last cheap moment to spend
them.

**It is not an import, and could not be.** None of this can be written in Quench: `sin`
wants a mantissa that grows, an exponent, and Ziv's retry loop, and the language can
express none of it. So they are host calls whichever side of a namespace they sit on, and
a host call is a runtime service every engine must implement. Putting them behind a name
removes nothing from the interpreter, the Dev JIT or the LLVM half. That cost is fixed
and paid; what a namespace changes is what a reader has to look at, which was the whole
complaint.

### This note said the opposite, and was wrong about which cost mattered

It read: *"They go behind imports, when there are imports. Not before, and not behind a
qualified name like `call maths.sin[…]` in the meantime — that was considered and
declined."* The argument was that a prefix costs four characters at every use site
forever, to say something an import would say properly later.

`import` arrived, and it turned out to say nothing about this at all. An import names a
**file**, and the maths is not one and cannot become one. So the thing being waited for
was never going to come, and the price of waiting was paid in every program written
meanwhile — which is the argument for spending a rename early rather than late.

### What self-hosting it would take — less than it looks, and more

The first answer written here was that a float of *chosen precision* was the gap, and
that was wrong. It is not a possibility gap. Checked rather than assumed, all three legs
already exist:

**A float becomes an exact**, through text, losslessly:

```quench
var.immut.b64 ['x'] = [*0.1* + *0.2*];
var.immut.e ['exact'] = [call as.e[call stitch['x']]];
# 7500000000000001/25000000000000000 — and correctly *not* 3/10
```

**The arithmetic** is unbounded exact rationals, which `e` already is.

**An exact becomes a float** — not directly, since `as.b64` refuses a ratio, but by
writing the decimal digits and reading them back, and `as.b64` is correctly rounded:

```quench
var.immut.b64 ['near'] = [call as.b64[*0.30000000000000004*]];  # the same bits
```

So `sin` is writable in Quench source *today*: convert in, sum the series in `e`, run
Ziv's test with exact comparisons, long-divide to twenty digits, convert back. The
language is accidentally complete for it, and `stitch` with `is`/`as` are what made it
so — none of that was built with this in mind.

What is missing is **speed**, and fatally rather than annoyingly. Every term of the
series is an exact rational whose denominator multiplies out and never shrinks; reducing
`1e300` needs π to a thousand bits *as a ratio*; and the long division is hand-rolled per
call. `Wide` exists in the Rust implementation for exactly this reason — a mantissa held
at a fixed width per attempt, rather than a denominator nobody bounded.

So the answer in three parts:

- **To be possible:** nothing. It already is.
- **To be usable:** a float of chosen precision — a mantissa width picked while running,
  which Ziv's loop doubles.
- **To be tolerable to write:** direct `b64` to `e` and back. Quench has **no conversion
  between number types at all** — every one goes through text — and that is a bigger
  question than the maths, since it is the rule that nothing converts on its own meeting
  the fact that sometimes something must.

Everything else on the list below is already Quench source waiting to be typed. Sorting,
searching, reversing, minimum-of-an-array, matrices: they wanted generics, arrays whose
length the type does not fix, and modules, and all three are built.

## The rest of the list

Each of these is waiting on something, and the something is named.

| | waiting on |
| --- | --- |
| ~~Number → text — `stitch`~~ | **built** |
| ~~Text → number — `is` and `as`~~ | **built** |
| Text: length, slice, search, case, trim, split | nothing — `characters` settled it |
| ~~The IEEE-required maths~~ | **built** |
| ~~The IEEE-recommended maths~~ | **built** |
| Input: stdin, arguments | nothing — a check comes first |
| Input and output: files | a check that cannot race the world |
| Sorting, searching, reversing | somebody writing them — they are Quench source now |
| Maps | two holes at once, hashing, and a decided iteration order |
| Matrices | somebody writing them |
| A library that is not source | an artefact that can carry a generic, which QIR cannot |
| ~~Generics~~ | **built** — `any` and `number`, one hole per function, `(any)` for a length |
| ~~Modules~~ | **built** — a block, nesting, five visibility words |
| ~~`import`~~ | **built** — `[program] files` says what a program is, `import` what a file uses |
| ~~The maths behind a namespace~~ | **built** — `call maths.sin[…]`, and not an import |
| Random numbers | a *specified* algorithm — see below |
| Time | nothing good |

## The two that fight the oracle

**Random** is fine as long as the algorithm is written down rather than borrowed. A
seeded generator whose steps are specified gives every engine the same sequence, and
the oracle never notices it exists. A `random` that asks the host is a disagreement
generator with a friendly name.

**Time** has no such answer. There is no `now` that three engines agree on, because
they do not run at the same moment. It is the first thing on this list that the oracle
would have to be told to skip, and that is worth knowing before it is added rather
than after.

## Which of these must be a runtime service

A host call is written once per engine. A function written *in Quench* is written once,
full stop, and every engine gets it by running the same QIR. So the size of this list
is not the cost — the number of **host calls** in it is.

Irreducibly a host call: anything that asks for memory, anything that touches the
outside, anything the machine does that Quench cannot express. Number formatting,
`sqrt`, opening a file, the bytes of a string.

Not: sorting, searching, reversing, minimum of an array, most of what a person means by
"the standard library". Those are Quench source now — holes exist, an array can be taken
without its length, and a module gives them somewhere to live — and they cost nothing
per engine forever after.

Which was an argument for building **generics and modules before the bulk of the
library**, and both are done, rather than writing forty host calls that did not have to be host calls.

## Before or after the other two backends

The library first, and specifically the part of it that moves the IR.

The case for backends first is real and it is about the oracle: today's three ways are
one interpreter and one Cranelift backend at two optimisation levels — *one* code
generator, not two. A C++/LLVM engine would be the first genuinely independent second
opinion, and every semantics bug the library introduces would be caught harder with it
present.

Against it, two things. The first is that Quench cannot currently put a number in a
string, so what the backends would make faster is a language nobody can write a program
in. The second is that the decisions above — how a string is measured, how something
fails, which maths exists — change the IR and the shape of a host call, and changing
those is cheaper with one runtime to update than with two.

So: number → text, the failure model, the string decision, the required maths,
generics and modules. Then the LLVM half. Then the bulk of the library, which by then
is Quench source and costs nothing per engine.

The work does not disappear in either order. What changes is how many places each
decision has to be made twice.
