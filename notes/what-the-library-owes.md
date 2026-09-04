# What the library owes

Quench has thirty-three runtime services and no library. This note is a list of what
is missing, what each thing is waiting on, and the order that costs least — written
before there is code for any of it, because most of the list is blocked on three
decisions and it is worth seeing that before writing the easy parts.

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

## Three decisions the rest waits on

### Is a `str` bytes, or is it characters?

`call count['café']` is 5 or it is 4. `call count['🔥']` is 4 or it is 1. Nothing can be built on
top of `str` until that is settled: length, indexing, slicing, searching, and any loop
that walks it all mean something different under each answer.

It is not a free choice in either direction. Bytes are what the heap holds and what a
file gives back, and they make `count` a subtraction. Characters are what a person
counting them means, and they cost a walk of the whole string to answer anything — and
"character" itself has two answers, since a scalar value and a grapheme cluster differ
on exactly the emoji people test with.

Quench already leans one way without having said so: a name may hold anything a line
can hold, which was decided by the marks doing the delimiting rather than by anyone
choosing it.

### How does something fail without stopping?

A trap ends the program. There is no recovering from one, anywhere, and that is
deliberate: Quench stops where another language would have undefined behaviour, because
undefined behaviour is where a differential oracle stops working — a program entitled to
do anything cannot be compared against itself. See
[the collector earns its place](the-collector-earns-its-place.md).

Which is fine for a mistake in the program and useless for a mistake in the *input*:
reading a number from a file that turns out to say `hello` has to be sayable without
killing the process that asked.

Every input function, every parse, every file that might not open needs an answer here.
It is a language question rather than a library one, and it gates most of the list
below.

### Which maths is Quench allowed to have?

`^` and `mod` are refused on binary floats already, because no standard requires a
`pow` to round the same way twice —
[what a float is allowed to do](what-a-float-is-allowed-to-do.md). The same argument
sorts the whole of a maths library into two piles, and the line is sharp:

**IEEE 754 requires these to be correctly rounded.** Every engine must agree, so they
cost the oracle nothing: `sqrt`, `fma`, `abs`, `floor`, `ceil`, `round`, `trunc`,
`copysign`, `remainder`. (`min` and `max` are specified too, but 754-2019 has four of
them because the 2008 pair handled not-a-number badly, so one has to be picked
deliberately.)

**IEEE 754 only recommends these**, and no library delivers it: `sin`, `cos`, `tan`,
`log`, `exp`, `pow`, `atan2`. Three engines calling three C libraries is three answers.
The way out is the one `e` and decimal already take — one implementation, in Quench's
own tree, that every engine calls — and until somebody writes it these stay out.

## The rest of the list

Each of these is waiting on something, and the something is named.

| | waiting on |
| --- | --- |
| ~~Number → text — `stitch`~~ | **built** |
| Text: length, slice, search, case, trim, split | bytes-or-characters |
| ~~The IEEE-required maths~~ | **built** |
| The IEEE-recommended maths | somebody writing it, once, for every engine |
| Input: stdin, arguments, files | failure that is not a stop |
| Output: files, streams beyond the two | failure that is not a stop |
| Sorting, searching, reversing | generics, and a way to say "a type that orders" |
| Maps | generics, hashing, and a decided iteration order |
| Matrices | modules |
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
"the standard library". Those are Quench source once generics and modules exist, and
they cost nothing per engine forever after.

Which is an argument for building **generics and modules before the bulk of the
library**, rather than writing forty host calls that did not have to be host calls.

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
