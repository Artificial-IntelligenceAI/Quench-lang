# The top level does not run

A Quench file is a list of declarations. Functions, types, constants — things that
*exist*. None of them execute. Execution begins in one place, `START`, and nowhere
else.

The alternative was the one Luarust inherited from Lua, where the file is the
program: lines run top to bottom as it loads, and there is no entry point because
the file is the entry point. That model is not wrong, and it is genuinely nicer for
scripts. It is the wrong fit here for one reason: **Quench has an entry point.** A
language with both an entry point and a top level that executes has two answers to
"where does this begin", and the second one is invisible. Python's
`if __name__ == "__main__"` exists entirely to paper over that, which is a good
indication of how it goes.

## Declarations are order-free, and that is the point

Because nothing at the top level happens, nothing at the top level has an order.
`START` can call a function written below it. Two types can refer to each other. A
reader can start anywhere in the file rather than at the top, because there is no
accumulated state to have missed.

That property is lost the moment one line executes.

## Which is why the top level holds constants, not variables

A variable needs an initialiser, and an initialiser has to run somewhere.

- If its value can be computed at compile time, nothing runs: the compiler folds it
  and writes the answer down. That is a **constant**, and it is allowed.
- If it cannot — `read_config()`, `now()`, anything reading the world — then code
  must execute *before* `START`. Which is the model that was just rejected, arrived
  at sideways, and worse than arriving at it honestly: the order in which those
  initialisers run is now something the language has to define, the programmer
  cannot see, and a reader cannot check. C++ has a name for the bugs this produces
  and the name is well known, which says something about how often it happens.

So: **a constant is a value the compiler can work out. A variable lives inside a
function.**

## This argument never depended on the memory model

An earlier draft leaned on ownership here: a mutable global has no owner and no
bounded lifetime, so there is nothing for a borrow checker to check. Quench collects
now, and the section survives without that, because it was never the load-bearing
part.

The reason stands on execution order alone. A top-level variable needs an initialiser
that runs before `START`, and once initialisers run, the order they run in is
something the language must define, the programmer cannot see, and a reader cannot
check. A collector does not make that better.

What is lost is a second, independent argument arriving at the same answer. The first
one was always the stronger.

## What it costs

Shared mutable state has to be passed. A cache, a counter, a logger: each one
becomes a parameter rather than a name that is simply in scope everywhere.

That is more typing, and it is the real price. What it buys is that "who can change
this?" is answerable by reading a function's signature, instead of by reading the
whole program and hoping.

## What it means for the implementation

- There is **no phase before `START`**. No initialisers to sequence, no ordering
  rule to specify, and nothing for the three execution methods to disagree about —
  which matters, because a pre-main phase is exactly the sort of thing three
  backends would sequence three ways.
- QIR needs no init section. Constants are folded by the frontend and reach the
  backends already as values.
- A missing `START` is a diagnostic, not a program that does nothing.
