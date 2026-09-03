# A counter belongs to its loop

Quench has two loops, and the rule between them is flat: **`range` always has a
counter, `while` never has one.** Nothing in between.

```quench
loop.temp.range.i64 ['i'] = [*1*, *5*] {    # 1 2 3 4 5. Both ends included.
    print.stdout['i' \n];
}

loop.while 'delta' > *0* {                  # no counter, and so nothing to name
    set ['delta'] = ['delta' - *1*];
}
```

There was a third form for a while — a `while` that also carried a counter, so you
could ask a question *and* know which pass you were on. It was dropped, and dropping
it is what made the rule sayable in one line. A counted `while` is a `var.mut` above
the loop and a `set` at the bottom of it, written out; it saved two lines and cost
the reader a special case. The chain is meant to be read left to right by somebody
who has seen one before, and a chain with an optional middle is not.

## The counter says how long it lives, and never whether it changes

```quench
loop.temp.range.i64 ['i'] = [*1*, *5*] { … }    # gone at the closing brace
loop.perm.range.i64 ['i'] = [*1*, *5*] { … }    # afterwards, `'i'` is 5
```

`temp` and `perm` are required, exactly as `mut` and `immut` are on a declaration,
and for the same reason: silence is a third answer and Quench does not take it.

What a counter does **not** say is `mut` or `immut`, and the reason is that neither
would be true. It changes every pass, so `immut` is a lie; nothing you write may
change it, so `mut` is a lie too. It is a third thing, and it gets a third word —
which is `range`, the link that says a counter exists at all.

So `set ['i']` inside the body is refused, and not with the `immut` error. That
error says *your declaration never said it could change*, which is the wrong
sentence here — it does change. The right one names who is doing it:

```text
`'i'` is a loop's counter, and the loop is what moves it.
    ~~~ the loop counts this
                ^^^ and this would move it too
Rule(s) broken: a counter belongs to its loop: the bounds say where it starts and
                stops, and nothing else may say otherwise
Tip(s): `break` is how you leave early, and `perm` is how you keep where it stopped.
```

Two mistakes that would render identically under one error are two errors.

## `perm` is the only thing that escapes its block

Everywhere else in Quench a name is gone at the closing brace. `perm` is the one
exception, and it is written down on the line that does it — you cannot get it by
accident, and you cannot get it without having typed the word.

It exists for one thing: after a `break`, the counter is the answer.

```quench
loop.perm.range.i64 ['i'] = [*1*, *100*] {
    if 'i' x 'i' > *10* { break; }
}
print.stdout[str:*stopped at * 'i' \n];        # 4
```

A `perm` counter holds **the last value it actually took** — four, not five. This
costs something real: the counter is one past the end by the time the loop asks its
question for the last time, so keeping the value it had means carrying a second
number around the loop. Only loops that wrote `perm` pay for it; a `temp` loop
carries the counter and nothing else.

A range that runs no passes at all — `[*1*, *0*]` — leaves a `perm` counter holding
`1`. It never took a value, so there is no last one, and where it would have started
is the only honest answer available.

## Both bounds, once, before the first pass

`[*1*, call count['xs']]` is worked out before the loop begins and never asked again. A
loop whose end moved underneath it is a loop nobody can read, and the cost of
promising otherwise is that every pass pays for the question.

`call count['xs']` is not even a question at runtime. A shape is written into the
declaration and never changes, so the checker answers it and the loop is bounded by
a constant — which is the one call in the language that usually is not one by the time
anything runs. The `call` in front of it is there because every call says so, and the
bare `count` because it came with Quench rather than with the writer.

## `break`, and nothing under it

```quench
if 'i' == *3* { break; }
```

There was going to be a `break when 'i' == *3*` form. It is the same thing as an
`if` with a `break` in it, and having both would mean two ways to write one line —
so it went, and `if` does the work it already did.

`break` ends the block it is written in, so anything below it in that block never
runs, and Quench says so rather than dropping it quietly:

```text
nothing here can run.
    ~~~~~~ the loop is left here
           ^^^^^^^^^^^^^^^^^ and this is under it
```

## What it costs to compile

A loop is the join an `if` already makes, with one edge going backwards. The head
block takes one parameter per variable the loop carries, exactly as a join does, and
one more for the counter — and what makes it a loop rather than a join is only that
the body's last edge points back at the head.

Which means loops needed no new IR at all. Block parameters were put in for `if`,
and a back edge is the same construction pointed the other way. See
[architecture.md](architecture.md).

The one new shape is an `if` inside a loop whose every arm leaves. Nothing arrives
at the block it would have joined into — so that block is unreachable, and an
unreachable block still has to be well formed before anything will delete it. It
gets an end made only out of its own parameters, which is valid by construction and
reaches nothing.
