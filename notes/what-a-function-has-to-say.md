# What a function has to say

```quench
fn.export.i64 ['add'] [immut.i64 'a', immut.i64 'b'] {
    give ['a' + 'b'];
}
```

The chain reads like a declaration's, because it is one: `fn`, then who can see it,
then what it gives back. Then the name, then what it takes, then the body. Nothing
about that order was invented for functions — it is the order `var` already used,
and the order `loop` borrowed after it.

## `nothing` is a word, not an omission

```quench
fn.file.nothing ['greet'] [immut.str 'name'] { … }
```

Most languages say a function returns nothing by not saying anything. C writes
`void`, which is a word; Rust and Go and Python leave the arrow off. Leaving it off
means the answer to *does this give me something back* is carried by an absence, and
an absence looks the same as a mistake.

So `nothing` is written. It costs seven characters and it means a reader never has
to read a body to find out whether there is an answer in it.

The same rule already applied twice before this: `immut` on a declaration and
`temp` on a counter. Three places now where silence used to be a third answer and
is not one any more.

## A function that answers, answers on every way out

```quench
fn.file.i64 ['bigger'] [immut.i64 'n'] {
    if 'n' > *0* {
        give ['n'];
    }
}
```

```text
this function says it gives back an `i64`, and does not always.
Rule(s) broken: every way out of a function that answers with something ends in a `give`
Tip(s): an `if` counts only when it has an `else` and every arm gives — otherwise
        there is a way through with no answer.
```

There is nothing honest to invent for the path that falls off the bottom. Zero is a
number somebody might have meant; so is the last value computed. A language that
picks one is picking on your behalf.

An `if` counts only with an `else`. A loop never counts — nothing here knows it runs
even once, and pretending otherwise would be the same kind of guess.

## Parameters are variables and say so

```quench
[immut.i64 'a', immut.i64 'b']
```

A parameter is a declaration with `var` taken off, because the bracket it sits in is
already the list. It says `immut` or `mut` like every other variable in the language,
and `mut` on one changes this function's copy and stops there — nothing in Quench is
a reference yet.

`[]` is written even when there is nothing to write in it. *Takes nothing* is a
thing to say, and saying it by leaving brackets off would be saying it with an
absence again.

## Calls cost no new syntax

```quench
'add'[*1*, *2*]      # a call, if `'add'` was declared with `fn`
'xs'[*2*]            # an index, if `'xs'` was declared with `var`
count['xs']          # a call to something the language provides
```

The first two are the same syntax. Which one a reader is looking at depends on what
the name was declared as, and a name cannot be declared as both — that is what makes
this a lookup rather than a guess, and it is why a function and a variable share one
namespace.

It was not always so. A call used to be a bare word before a bracket, which told it
apart from an index without any lookup at all. What that cost was a rule nothing else
in the language needed: a function's name was the only one written *twice*, once
between marks where it was declared and once bare at every call, so it was the only
one that could not hold a space or an emoji. Marks everywhere is one rule instead of
two.

And it buys something the bare word was too busy to do. A bare word before a bracket
now means exactly one thing — **this came with Quench** — so `count['xs']` and
`'count'['xs']` are visibly different at every use, and nothing the language provides
has to be held back from a writer who wanted that name.

Arguments are separated by commas, because juxtaposition already means something:
pieces side by side build one value. It cannot also separate two. An index writes its
dimensions side by side instead, matching the shape it indexes into.

## Constants outside, variables inside

```quench
const.export.i64 ['LIMIT'] = [*100*];
```

A constant is a value the compiler can work out. Anything needing code to run to
produce it would need that code to run before `START` — which is the model Quench
turned down when it decided the top level does not run. See
[the-top-level-does-not-run.md](the-top-level-does-not-run.md).

So a constant has **no storage**. Its value is written in wherever it is named,
which is why `set` on one is refused and why indexing one is too:

```text
`'A'` is a constant.
    ~~~ declared here
         ^^^ and wanted somewhere it lives, here
Rule(s) broken: a constant is written in wherever it is named, so there is no
                storage to index or change
```

And why there is no mutability link on the chain: `const` is already the answer to
whether it changes, which is the whole reason it is a different word from `var`. A
link that only ever says one thing is noise rather than explicitness — the two are
easy to confuse and this is where the line falls.

Constant arrays are refused for now. An array wants somewhere to live and a constant
has nowhere, so a real one would need the collector and a place to put it before
`START`, which is the model again.

## Visibility, finally

`file`, `program` and `export`, required, on everything at the top of a file. See
[three-lines-a-name-can-cross.md](three-lines-a-name-can-cross.md).

With one file and no linking, nothing that runs can yet tell them apart — there is
nowhere for `file` and `program` to differ. They are checked and recorded anyway,
because the alternative is adding them later, and a required word added later is a
required word that everything written before it does not have.

## `START` is not a function

It has no visibility, because nothing may call it. It has no return type, because
there is nobody to give an answer to:

```text
`START` has nobody to give an answer to.
Rule(s) broken: `START` is where the program begins, not something anything calls
Tip(s): `give;` on its own works here, and stops the program early.
```

`give;` does work in it, and stops the program. That is a different thing from
answering, and it is the same word because it is the same action: leave, now.

## What it cost to compile

Almost nothing, which is the point of having done QIR first. QIR already had
functions with parameters, a call instruction and a return — the generator has been
compiling a helper function and calling it since before the frontend had a way to
write one. Both engines already ran calls: the interpreter on its own explicit call
stack, the Dev JIT through Cranelift's.

So the whole of this note is frontend work. The one thing lowering had to learn is
that a function's place in the checked list is its id in the module, which is what
lets a body call something written underneath it.

The one shape that was new: an `if` whose every arm gives an answer. Nothing arrives
at the block below it, and an unreachable block still has to be well formed before
anything will delete it. It gets an answer made on the spot, of the type the
function was going to give back anyway — which needs nothing from anywhere, and so
is valid wherever it lands. The same trick now covers the loop case that `break`
introduced, and the special case that had for it is gone.
