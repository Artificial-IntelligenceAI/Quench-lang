# The declaration chain

Quench declares things the way Luarust does. A declaration says what it is, then the
things that are true about it, then the names, then the values:

```quench
var.str      ['name'] = [|Tankun|];
var.mut.b16  ['x']    = [|1000|];
```

Names live in quotes, so a name can be anything you can type. Written values wear
bars, so a quoted thing is a name wherever you meet one and never has to be read as
a value depending on where it sits. Statements end in a semicolon.

Several at once, sharing what they have in common:

```quench
var.str      ['name', 'name 2'] = [|Tankun|, |Ada|];
var.mut      [b16 'a', str 'b'] = [|1000|, |hello|];
var          [mut.b16 'a', str 'b'] = [|1000|, |hello|];
```

## What changed from Luarust, and why

**The visibility slot moved.** Luarust writes it on every declaration, variables
included: `var.local.str ['name']`. Quench does not, because a variable lives inside
a function and nothing outside can name it however much it would like to — there is
nothing to permit or deny. So the slot is gone from `var`, and appears instead on
top-level declarations, which are the only things two parts of a program could both
name. See [three lines a name can cross](three-lines-a-name-can-cross.md).

One consequence: Luarust has two "several at once" forms that differ only in whether
visibility is shared or written per name. Without visibility on variables those two
would collapse into one — except that `mut` inherits the position, so the pair
survives with `mut` as the thing that can be shared or said individually.

**Visibility is required.** Luarust's default is `restricted`, which is a joke it
makes on purpose: the declaration compiles and every use of it does not. It also
ships `defaults.no-visibility-stated.error;` for anyone who would rather hear about
it where they wrote it. Quench makes that opt-in the only behaviour, and does not
have the default at all.

That is not a criticism of the joke, which is a good one. It is that Quench's whole
claim is the quality of its errors, and a default of `restricted` moves the error
from the declaration that was wrong to the use that was innocent. A message then has
to explain that a line somewhere else silently meant "nobody" — rescue work that a
required word makes unnecessary.

**The words are Quench's**: `file`, `program`, `export`, rather than `local`,
`global`, `public`, `restricted`. Three rather than four, because the fourth existed
to be a default and there is no default.

## Not decided

- **How a top-level function is declared.** Luarust's parser has no routine in its
  AST to copy, so there is nothing to inherit and this is still open.
- **Whether the type is always written**, which waits on the type system.
- **Whether `mut` keeps its spelling**, given `file`/`program`/`export` were chosen
  as words rather than initials. Variables are far higher volume than visibility, so
  the argument that settled that one does not obviously carry.
