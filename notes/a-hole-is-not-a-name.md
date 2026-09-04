# A hole is not a name

```quench
fn.file.any ['first of'] [immut.arr.any (any) 'xs'] {
    give ['xs'[*1*]];
}
```

`any` is a hole: a place a type goes, filled by whoever calls. There is one per
function, and every `any` in a signature is the same one — which is what makes
`[immut.any 'a', immut.any 'b']` mean *two of the same thing* rather than two of
anything.

## The wrong turn, because it is the useful part

The first proposal was `'T'`, and then `'element'`, both between marks. The argument
was the language's own rule: a bare word is Quench's and a marked name is the writer's,
so a type the writer supplied must wear marks. It survived two rounds of "why the
single quotes?" and got a better-sounding defence each time.

It was wrong, and the sentence that ended it was **"they didn't invent `element`"**.

Marks do not mean *this is not Quench's*. They mean *this names something of mine*.
`'xs'` names an array that exists; `'first of'` names a function that exists;
`'element'` names nothing at all — it is a hole, and a hole is not a name. The rule had
been applied by elimination rather than read.

Written down because the mistake is a tempting one and it is not about generics: two
categories, a thing that is not in the first, therefore in the second — without
checking whether the second describes it.

## So it is a word the language provides

Bare, like `arr` and `nothing`, because it is Quench's word and not the writer's:

```quench
fn.file.i64 ['first number'] [immut.arr.i64 (3)   'xs'] { give ['xs'[*1*]]; }
fn.file.any ['first of']     [immut.arr.any (any) 'xs'] { give ['xs'[*1*]]; }
```

Nothing invented, nothing to look up, and no fourth kind of bracket in a language that
has three. The cost is that one word cannot say *two* holes, which a map will want — a
key type and a value type — and that is the day names become worth their price. Not
before.

## Two words, because Quench knows two things about a type

Every type answers `==`; only numbers answer `<`. That is the whole of what the
language currently knows about types it has not seen, so it is the whole vocabulary:

- **`any`** takes all sixteen, and its body may hold a value, copy it, hand it back,
  put it in an array and compare it for equality. Nothing else, because nothing else
  works on every type.
- **`number`** takes `i8`…`u64`, `b16`/`b32`/`b64`, `d32`, `d64` and `e`, and buys the
  body `+`, `-`, `x`, `/` and the four comparisons.

Neither is looser. **`any` takes every caller and forbids most of the body; `number`
takes fewer callers and allows most of it** — the same restriction, on opposite sides of
the wall.

`mod` and `^` are refused on a `number` for a reason worth stating, because it is the
rule the whole design rests on: **a hole has what all the types filling it have.** `mod`
is refused on a float, a decimal and an `e`; `^` is built for `b64` alone. Neither works
on every number, so neither works on `number`.

## The caller says nothing, because the caller does not have to

```quench
call 'first of'[share 'ws']
```

No type is written at the call. The argument is an `arr.str (3)`, so the hole is `str`,
and saying it again would be a second chance to disagree. That is the same rule `is` and
`as` follow from the other end — [a check comes first](checking-comes-first.md) says the
type on the chain *because text cannot say it*, and an argument can.

Which does mean a written value in a hole position has to carry its own type, since a
written value holds nothing until a type reads it:

```quench
call 'echo'[str:*kept*]     # `str`, said by the value because nothing else can
call 'echo'[*kept*]         # refused: nothing here says which type the hole is
```

## It compiles away, and that is forced rather than chosen

A generic function is a **pattern**, not a function. The checker writes out one real
function per type it was used at — `echo (str)`, `echo (i64)`, `echo (b64)` — and what
leaves the checker has no `Ty::Hole` in it anywhere. QIR, the interpreter and the Dev
JIT never learn the word.

The alternative would be one copy carrying a type tag, and it is not available. A slot
is an `i64` whatever is in it, and the only thing that says whether the collector should
follow one is the type at the call site — see
[the collector earns its place](the-collector-earns-its-place.md). One `first of`
serving both `arr.i64` and `arr.str` would have to tag every value and teach the
collector to read it, which is a runtime cost on every program, generic or not, to serve
the ones that are.

One thing follows from that and is worth saying out loud: **the oracle does not grow.**
There is no new instruction, no new host call and no new trap, so there is nothing here
for two engines to disagree about. Every copy is an ordinary function of ordinary types,
and the whole of what could go wrong lives in the checker — whether it makes the right
copies and points each call at the right one — which is what the tests are for.

So: a function per type, and nothing at all at runtime. The one thing it needs is a
limit. A pattern handed an array of itself asks for a wider type every time round and
the list of copies never ends, so there is a cap and a diagnostic rather than a wait.
Rust has the same limit for the same reason.

## What it cost the rest of the compiler

Almost nothing, which is the point of monomorphising in the checker. Lowering has always
assumed a function's place in the list is its id in the module, and that assumption is
the one thing generics could have broken — a pattern is one entry and needs to be
several. Doing the copying before lowering keeps it true: the patterns are dropped, the
copies are appended, and every call is pointed at the copy it meant.

Two passes, because a pattern may call a pattern — or itself — and the copy it wants
does not exist when the call is first seen. Discover every `(pattern, type)` the program
reaches, making copies as it goes and walking each copy for more; then, when they all
exist, rewrite the calls.

## A length is a hole too

A size is part of the type, so an `arr.i64 (3)` was not an `arr.i64 (grow)` and a
function taking an array had to name its length — which would have made `largest` a
function per length, and no use to anybody. That was true before holes and is not a
consequence of them, but it is the wall they walked into.

So the size position gets the same word:

```quench
fn.file.number ['largest'] [immut.arr.number (any) 'xs'] { … }
```

One `largest` per element type and **none per length**. It takes an `arr.i64 (3)`, an
`arr.i64 (5)` and an `arr.i64 (grow)`, because `any` claims to know nothing about the
length and none of them contradicts it. Nothing goes the other way: a slot declared
`(3)` may not be handed a length nobody counted.

`any` is deliberately **not** `grow`, and the difference is what each one grants:

| | `grow` | `any` |
| --- | --- | --- |
| what it says | there is no number *yet* | the number was never told to me |
| may be `add`ed to | yes | **no** |
| may be written as a literal | yes, `[[]]` and then filled | no — one of these arrived |
| `count` | asked while it runs | asked while it runs |

The `add` row is the whole reason they are two words. An array handed in may be one that
grows or one that does not, and a function assuming the first would be writing off the
end of the second.

Only the first size may say it, which is the rule `grow` already followed and for the
same reason: finding an element is `(i - 1) x stride + j`, a stride is the sizes *under*
a dimension, and the outermost is the one whose size the arithmetic never asks for.

**Two holes at once.** Maps want a key and a value. That is when a hole needs a name,
and then the question this note opens with comes back with a better answer than `'T'`.

**A vocabulary of what a type can do.** `any` and `number` are the two facts the
language currently has. A third — "a type that orders", which would let `str` be sorted
— wants `<` on text first, and that is its own decision.
