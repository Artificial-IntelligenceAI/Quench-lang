# An array is the only thing two names can reach

Everything else in Quench is a value. Naming a number again is naming the number;
there is nothing behind it for a second name to get at. An array is not like that —
it lives on the heap, and a name holds a handle to it. So this line has two possible
meanings and they are not close together:

```quench
var.mut.arr.i64 (2) ['b'] = ['a'];
```

Either `'b'` is a second name for one array, and a change through either is seen
through both. Or `'b'` is a second array holding the same things, and the two go
their own ways. **Quench makes you say which**, and refuses the line above:

```text
this does not say whether it shares `'a'` or copies it.
Rule(s) broken: naming an array in a value says `share` or `copy`, and silence is
                not one of them
Tip(s): `share` makes a second name for one array, so a change through either is
        seen through both. `copy` makes a second array, and pays for it.
Suggested fix(s): `[share 'a']`, or `[copy 'a']`
```

```quench
var.mut.arr.i64 (2) ['shared'] = [share 'a'];
var.mut.arr.i64 (2) ['mine']   = [copy 'a'];
set ['a'[*1*]] = [*99*];
# a = [99 2], shared = [99 2], mine = [1 2]
```

## Why not just pick one

Every other collected language picks sharing, and it is a perfectly good answer:
free, and it is how you write a function that fills in an array you hand it. Picking
copying is also a good answer: no aliasing, ever, and comparing contents becomes the
obvious meaning of `==`.

What made neither of them right *as a default* is that both hide something, and they
hide opposite things:

- sharing hides that a `set` over here changes a thing over there, at a line that
  looks local;
- copying hides an allocation and a walk of the whole array, at a line that looks
  free. On a `(1000 1000)` that is a megabyte, silently, on an `=`.

This is the same shape as every other decision the language has made — `mut` against
`immut`, `temp` against `perm`, `nothing` against a type, `file` against `program`
against `export`. Each of those was somewhere a language could have had a default and
Quench made it say instead. This is a bigger silent difference than any of them.

Written where it is paid, which is the point. `share` costs nothing and compiles to
nothing: naming the variable already gave the handle, and sharing is what a handle
does. `copy` is a call into the runtime, and it is the one that had to be written
down because it is the one that costs.

## Which is why `==` could be built

While the two questions could not be told apart, comparing arrays was refused —
`['a' == 'b']` could have meant *the same array* or *the same contents*, and picking
one silently is the thing this note is about.

Once `share` exists, *the same array* has a way to be said and asked, and the other
question is free to be what `==` means:

```quench
var.immut.arr.i64 (2) ['a']    = [[*1* *2*]];
var.immut.arr.i64 (2) ['twin'] = [[*1* *2*]];
['a' == 'twin']                              # true. Two arrays, same contents.
```

Element by element, in a host call, so neither engine has its own idea of it. Two
arrays of different shapes are two different *types* and the comparison was already
refused before this — a `(2)` and a `(3)` never meet.

## What it does not fix

`share` through an `immut` name does not make the array immutable:

```quench
var.mut.arr.i64   (2) ['a'] = [[*1* *2*]];
var.immut.arr.i64 (2) ['b'] = [share 'a'];
set ['a'[*1*]] = [*99*];                      # 'b' shows [99 2]
```

`immut` means *this name cannot be used to change it*, which is what it has always
meant and is still true. It does not mean the thing never changes, and no language
with sharing gets that for free. Saying so here rather than letting somebody find it.

## Which is also what a call now says

An array is the first thing that could cross into a function and be changed there,
so the call site is where the two answers matter most:

```quench
fn.file.nothing ['zero_it'] [mut.arr.i64 (4) 'xs'] { … }

zero_it[copy 'xs'];      # 'xs' is untouched
zero_it[share 'xs'];     # 'xs' comes back zeroed
```

Nothing extra was needed for that. A call's argument is a value, and a value naming
an array already had to say which — so the rule wrote itself, and the reader of the
call can see what happens to their array without opening the function.

## What an array holds

Any type that is built: `arr.i64`, `arr.bool`, `arr.str`, `arr.e`. A slot is an
`i64` however wide the thing in it is, so room was never what was missing — what was
missing was *telling the runtime which*, since showing a slot and comparing two of
them both depend on it. The element kind travels beside the handle as a constant.

Text wears its marks inside an array and nowhere else:

```text
[*hello there* *world*]
```

`[hello there world]` could be two elements or three and there is no way to tell.
Outside an array a `str` prints bare, because nothing is beside it to run into.

An `e` inside an array compares by value like one outside it: an array of `1/2` and
an array of `0.5` hold the same numbers.

## Two `arr` links are two allocations

```quench
var.immut.arr.i64     (2 3) ['flat']   # one allocation of six
var.immut.arr.arr.i64 (2 3) ['nested'] # three: two of three, and two handles over them
```

Every `arr` link is one allocation, and every allocation says how big it is — so the
sizes are spent one per link, outside in, and **the innermost takes whatever is
left**. That one rule covers both lines above and keeps `arr.i64 (2 3)` the rectangle
it always was.

They are written the same way, flat, because the type already gave the shape:

```quench
= [[*1* *2* *3* *4* *5* *6*]]
```

They print differently, because the difference is real:

```text
[1 2 3 4 5 6]
[[1 2 3] [4 5 6]]
```

Six numbers in one place, or six numbers in two places with something pointing at
both. Only the second can be taken apart, and taking it apart is the whole reason to
write it:

```quench
var.mut.arr.i64 (3) ['row'] = [share 'nested'[*2*]];
set ['row'[*1*]] = [*99*];        # 'nested' is [[1 2 3] [99 5 6]]
```

An index may stop where an allocation ends and nowhere else. Stopping hands back the
array that lives there — which is a thing, with a name of its own, that `share` and
`copy` already knew how to talk about.

## The first thing the collector will have to follow

Everything Quench holds has been a **leaf** until now. A number is a number, an `e` is
two magnitudes in a row, an array of `i64` is a run of numbers — nothing in any of
them points anywhere.

An array of arrays does. It is the first value in the language whose slots hold
handles, and so the first that a tracing collector would have to walk rather than
just mark. Nothing is freed yet, so nothing depends on this today; it is written down
because stage two is where it starts to. See
[the-collector-earns-its-place.md](the-collector-earns-its-place.md).

## A size may say `grow`

```quench
var.mut.arr.i64 (grow) ['xs'] = [[*1* *2* *3*]];
add ['xs'] = [*4*];
```

A growing array is not a second type. It is an `arr` whose first size says there is
no number yet, and everything already true of an `arr` stays true of it: indexing,
`share` and `copy`, `arr.arr`, every element type, `==` on contents. There was one
new thing to say and it is said in the one place that says how big something is.

Three other spellings were tried first and each broke a rule the language already
had:

- **`dyn-arr`** abbreviates a word that does not describe it. "Dynamic" is inherited
  jargon for *at runtime*; the size being unknown then is a consequence of growing,
  not the thing itself. It would also have been the first hyphenated chain link.
- **`grow`** alone names the behaviour and not the thing. *Grow what?*
- **`grow.arr`** puts the noun back, and then `arr.grow.arr.i64 (3)` has an adjective
  sitting between two nouns with nothing saying which it attaches to. That is a
  precedence question in the middle of a type, in the language that refuses
  precedence questions on principle. The chain has been binding-free because every
  link is a complete thing, and an adjective breaks that the moment there are two
  nouns to bind to.

`(grow)` has none of those problems because **sizes are already positional**, one per
`arr` link, outside in. `(2 grow)` cannot be read two ways.

### Only the first size of an allocation

```quench
var.mut.arr.i64 (3 grow) ['xs'];    # refused
```

Finding an element is `(i - 1) x stride + j`, and a stride is the sizes *under* a
dimension. The outermost has nothing above it to be a stride for, so it is the one
dimension whose size the arithmetic never asks for — and therefore the only one that
can be left unsaid. A growing inner dimension is what `arr.arr.i64 (2 grow)` says,
and that one works.

### What `grow` costs

`count['xs']` folds to a number on a fixed array, because a shape is part of the type
and a type does not change while a program runs. On a growing one it is a question,
and costs one call. That is the whole of what a reader pays, and it is visible in
the declaration.

`count` takes any array now, not only a named one — `count['jagged'[*2*]]` is how
long the second row is, and a row of a jagged array is exactly the thing whose length
nothing else can tell you.

### Written empty when nothing says where a row ends

```quench
var.mut.arr.arr.i64 (grow grow) ['jagged'] = [[]];
```

Elements are written flat and cut into rows. A row of something that itself grows
has no length to cut at, so such an array is written empty and filled with `add`. A
*fixed* number of growing rows — `(2 grow)` — starts as that many empty ones, which
falls out of the same rule rather than being a second one.

## A constant array lives in the module

```quench
const.file.arr.i64 (3) ['PRIMES'] = [[*2* *3* *5*]];
```

This was refused for a while, and the refusal said *not built yet*, which was the
wrong word for it. The reasoning in that error was right — a constant is written in
wherever it is named, and an array is a thing rather than a value — and the answer
was already in the codebase and went unnoticed: **`Module::text`**.

Every `str` a program is written with lives in a table in the module. Every engine
lays that table out before the entry function is called — the interpreter reads it
straight, the Dev JIT builds a `Piece` table whose address it bakes into the code.
**No code runs before `START` to make that happen.** It is data in the artefact, not
a program.

A constant array is that with numbers in it. `Module::tables` sits beside
`Module::text`, and every engine lays those into its heap in order before anything
runs — so **table `i` is handle `i`**, and the handle is known while compiling. A
constant array is therefore a *constant*: it costs nothing at all where it is named,
which is what the word should have meant from the start.

Everything else falls out rather than being decided:

- there is **one** of it, so `share` names that one and `copy` gives you one you may
  change;
- **indexing works**, because it does have somewhere it lives;
- **`set` is refused**, because a program does not rewrite what it was written with;
- a **nested** one works too, because an inner table's handle is known by the time
  the outer table is written — inner tables are laid out first.

Two are refused with reasons rather than promises. A constant array cannot say
`grow`: what is written down is however many were written. And an array of `e` is
not built, because every other element is a number, a nought-or-one or which piece
of text — all known before anything runs — while an `e` slot holds a handle the
runtime makes.
