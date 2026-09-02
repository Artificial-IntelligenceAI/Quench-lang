# Precedence stops where mathematics stopped

Quench keeps the precedence mathematics established, and refuses to invent any beyond
it. Where mathematics never settled an order, brackets say what was meant.

```quench
[*1* + *2* x *3*]          # 7. You learned this before you saw a computer.
[*a* + *b* < *c*]          # (a + b) < c. Also settled.

[*a* mod *b* + *c*]        # an error. Nothing ever agreed where `mod` sits.
[*a* and *b* or *c*]       # an error. Invented by language designers, not derived.
```

## What mathematics actually settled

Three levels, and one more for comparison:

| | |
| --- | --- |
| exponent | binds tightest |
| `x` `/` | then these |
| `+` `-` | then these |
| comparison | looser than all of it |

That is not arbitrary — it is what makes `ax² + bx + c` readable without brackets, and
it is taught everywhere before anybody meets a keyboard. Fortran's table is essentially
this, and is untroubled by it, because Fortran's operators are close to mathematics'
own.

## What it did not settle, and the proof

Everything programming added — bitwise operators, shifts, `mod` written infix, `and`
against `or` — has no agreed order, because nothing outside programming ever needed one.

The proof is not an argument, it is two languages:

- **C** put `&` *looser* than `==`, so `val & mask == 0` means `val & (mask == 0)`.
  Kernighan and Ritchie wrote in K&R that "some of the operators have the wrong
  precedence". Ritchie wanted to fix it and could not, because changing it would have
  silently broken every existing `if (a == b & c == d)`.
- **Python** put `&` *tighter* than comparison, so `arr > 5 & arr < 10` means
  `arr > (5 & arr) < 10`. NumPy inherited that table and repurposed `&` for element-wise
  logical-and, so this is now one of the most common mistakes its users make.

Two languages, opposite choices, both producing a famous trap for the same operator.
So it is not that C got `&` wrong. **`&` has no right answer**, and any table is a trap
— they merely trap differently.

A table that is obvious for two operators and a coin-flip for the rest is worse than no
table, because it teaches a reader to trust it.

## Why not brackets for everything

Because `(*a* x *b*) + (*c* x *d*)` is heavier than `a x b + c x d` for something every
human on earth reads identically, and refusing the one convention that *is* universal
buys nothing. The rule a reader has to already know should be a rule they already know.

## COBOL got there first

COBOL carries both forms and says when to use each. `COMPUTE A = B + C * D` has full
precedence — brackets, then `**`, then `* /`, then `+ -`. `ADD B TO C GIVING A` has one
operation per statement, so the question cannot arise. Its own guidance is to use the
verb for a single operation and `COMPUTE` for a formula.

Which is this rule with different clothes on: precedence where it is safe, and an
unambiguous form where it is not.

## What the error has to be

An ambiguous expression is not a parse failure to be reported as "unexpected token". It
is a reader and a writer disagreeing, and the message should say so and offer both
readings:

```text
`mod` and `+` have no agreed order, so this could be read two ways.

  3 | var.i64 ['x'] = [*a* mod *b* + *c*];
    |                  ^^^^^^^^^^^^^^^^^ which of these first?

Rule(s) broken: Quench keeps the precedence mathematics settled and invents none
Tip(s): `x`, `/`, `+` and `-` need no brackets. Everything else does.
Suggested fix(s):
  - `(*a* mod *b*) + *c*`
  - `*a* mod (*b* + *c*)`
```

## The operators

| | written |
| --- | --- |
| add, subtract | `+` `-` |
| multiply | `x` or `×` — **never `*`**, which is the mark a written value wears |
| divide | `/` or `÷` |
| remainder | `mod` |
| exponent | `^` or `xx` — multiplying twice |
| compare | `<` `>` `</=` `>/=` `==` `!=` `≠` |

`x`, `xx`, `mod` and `eq-to` need no tokens at all: Quench reserves no words, so an
operator spelled with letters costs nothing, and `x` is still available as a variable's
name anywhere a name is wanted, because names are quoted.

## Two spellings that were not available

**`**` for an exponent is impossible**, not merely ugly. `*a* ** *b*` lexes that `**`
as an *empty written value* — the first `*` opens one, the second closes it. There is no
arrangement where `**` is an exponent while `*` marks a value. Hence `^`, which is how
mathematics writes it anyway, and `xx`.

Taking `^` costs an exclusive-or, since most languages spend `^` on that. Quench cannot
follow them and also keep the rule at the top of this note.

**`=` for equality is impossible for a subtler reason**, and the reason is a good
illustration of how one decision reaches another.

Luarust writes equality `=` and gets away with it, because its arithmetic lives inside a
`math { … }` block. Inside that block `=` cannot possibly be a declaration, so there is
nothing to confuse. The block was doing disambiguating work as well as grouping work.

Quench put expressions in the value list directly and has no such block. So `=` would
have meant a declaration outside the brackets and a comparison inside them, decided by
where it sat — which is precisely what the marks exist to prevent, and would have made
`=` the only symbol in the language that was not itself wherever you met it.

So equality is `==`, or the word `eq-to`. Removing a block cost an operator, which is
the sort of thing that is only visible once both decisions are on the table.
