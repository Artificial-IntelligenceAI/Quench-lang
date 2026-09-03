# The declaration chain

Quench declares things the way Luarust does. A declaration says what it is, then the
things that are true about it, then the names, then the values:

```quench
var.immut.str ['name'] = [*Tankun*];
var.mut.i64   ['x']    = [*1000*];
```

Names live in quotes, so a name can be anything you can type. Written values wear
marks, so a quoted thing is a name wherever you meet one and never has to be read as a
value depending on where it sits. Statements end in a semicolon.

Several at once, sharing everything they have in common:

```quench
var.immut.i64 ['a', 'b'] = [*1*, *2*];
```

And a shape where the type has one, which sits between the chain and the names because
a shape is part of the type rather than part of a name:

```quench
var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
var.mut.arr.i64 (grow)  ['xs'] = [[]];
```

## The chain outgrew declarations

It is not `var`'s any more. Four things wear it, and they read alike on purpose:

```quench
var.immut.arr.i64 (2 3) ['m'] = [ … ];
const.export.i64        ['LIMIT'] = [*100*];
fn.export.i64           ['add'] [immut.i64 'a', immut.i64 'b'] { … }
loop.temp.range.i64     ['i'] = [*1*, *5*] { … }
```

Each one is *what it is*, then *what is true about it*, then *the name*, then the rest.
A counting loop declares a variable, so it says how long that variable lives and what
it is, in the places a declaration says whether it changes and what it is. A parameter
is a declaration with `var` taken off, because the bracket it sits in is already the
list.

None of that was planned. The shape was chosen for `var` and then each new thing that
had to say something about itself found the slot already there.

## What changed from Luarust, and why

**The visibility slot moved.** Luarust writes it on every declaration, variables
included: `var.local.str ['name']`. Quench does not, because a variable lives inside a
function and nothing outside can name it however much it would like to — there is
nothing to permit or deny. So the slot is gone from `var`, and appears instead on
top-level declarations, which are the only things two parts of a program could both
name. See [three lines a name can cross](three-lines-a-name-can-cross.md).

Luarust has two *several at once* forms, differing only in whether visibility is shared
or written per name. Quench has one. Without visibility on a variable there was nothing
for the second form to say differently, and `mut` did not save it: a declaration that
mixed `mut` and `immut` in one line would be two declarations wearing one keyword, and
the language already has a way to write two declarations.

**Visibility is required.** Luarust's default is `restricted`, which is a joke it makes
on purpose: the declaration compiles and every use of it does not. It also ships
`defaults.no-visibility-stated.error;` for anyone who would rather hear about it where
they wrote it. Quench makes that opt-in the only behaviour, and does not have the
default at all.

That is not a criticism of the joke, which is a good one. It is that Quench's whole
claim is the quality of its errors, and a default of `restricted` moves the error from
the declaration that was wrong to the use that was innocent. A message then has to
explain that a line somewhere else silently meant "nobody" — rescue work that a
required word makes unnecessary.

**The words are Quench's**: `file`, `program`, `export`, rather than `local`, `global`,
`public`, `restricted`. Three rather than four, because the fourth existed to be a
default and there is no default.

## The three that were open, and what they came to

**How a top-level function is declared.** Luarust's parser had no routine in its AST to
copy, so there was nothing to inherit. It became the chain like everything else:
`fn.<visibility>.<what it gives back>`, with `nothing` as a real link rather than a
missing one. See [what a function has to say](what-a-function-has-to-say.md).

**Whether the type is always written.** Always. Nothing is inferred, and a written
value means nothing until a type reads it — `*1000*` is a number under `i64` and four
characters under `str`, which is the whole of
[what the marks are for](what-the-marks-are-for.md).

**Whether `mut` keeps its spelling.** It does, and it gained `immut` beside it, and
both are required. The note doubted this: visibility got words rather than initials
because the volume was low, and variables are far higher volume. What settled it was
not volume — it was that a missing `mut` used to mean *immutable* by silence, and
silence is the one thing this language has taken out of every position it held.
