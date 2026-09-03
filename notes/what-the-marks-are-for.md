# What the marks are for

Quench writes two things between marks, and uses a different mark for each:

| written | is | example |
| --- | --- | --- |
| `'…'` | a **name** | `var.str ['greeting']` |
| `*…*` | a **written value** | `= [*hello*]` |

Two, not three, and that is the whole point. "Is this text or is it a number?" was
never a question the marks had to answer, because **the type answers it**: `*1000*` is
the number one thousand under `b16` and the four characters `1000` under `str`. A
third mark would have been a second, worse answer to a question already settled
properly elsewhere.

What the marks *do* settle is the one question position cannot: a quoted thing is a
name wherever you meet it, and never has to be read as a value because of where it
happens to sit.

That is also why writing `"…"` gets an error offering both, since the mistake is not
knowing there were two questions.

## A written value is literal, and escapes stand outside it

Between the marks, everything is the character it looks like: emoji, braces,
punctuation, digits, semicolons, quotes.

The one exception is `\*`, which exists because the closing mark is the single
character that could not otherwise be written. The same rule applies inside a name,
with `\'`.

Escapes are **outside**:

```quench
var.str ['s'] = [*line one* \n *line two*];
```

`\n` is an item, sitting next to the text rather than hidden inside it. So `*a\nb*`
is a backslash and an `n` — not a newline — and that is not a trap, it is the design:
reading a piece of Quench text never means working out which of its characters were
secretly instructions.

The escapes are `\n`, `\t`, `\r` and `\\`. Anything else is an error that lists them.

## A written value states its type where nothing else can

A written value means nothing on its own. `*1000*` is the number one thousand under
`b16` and the four characters `1000` under `str`, and that is the type's decision.

In a declaration the chain has already said it, so nothing is repeated:

```quench
var.str ['s'] = [*line one*];
```

A print list has no chain, so the value says it itself:

```quench
print[str:** \n]
```

A name needs no annotation anywhere, because its declaration gave it one already:

```quench
print[str:*Hello, World!* 'name' \n];
```

A **bare written value in a print list is not valid** — `print[*Hello*];` states no
type, and there is nowhere for one to come from. This is the same rule as everywhere
else here: the information exists, so it is written down rather than guessed at from
what seems likely.

Every statement ends in a `;`, print included.

## Juxtaposition builds a value, commas separate them

Items sit next to each other. There is no `+`, because nothing is being concatenated
into a third thing — the items *are* the value, used in order. This is how a value is
written wherever a value is written, whether it is being stored or printed:

```quench
print[*Hello, World!* 'name' \n];
var.str ['s', 'ss'] = [*line one* \n *line two*, *idk* \n *Claude*];
```

The comma is the only thing that says where one value stops, which is exactly what
lets a value run to as many items as it likes.

C is the interesting comparison, because it came close. `"Hello" "World"` is already
juxtaposition — two literals that become one array at compile time — so the mechanism
was there. What C lacks is a *notation* for a value made of pieces that are not all
string literals, and it could not easily gain one, because an escape has to work in a
character literal too. `\'\n\'` is a single `char`, and there is no list beside it for a
separate escape item to sit in.

So C's escapes went inside because a literal there is one self-contained token that
must denote one value, in every position a value can appear. Quench writes *every*
value as a list of items, so there is always somewhere beside the text for an escape
to stand — whether the list is being printed or stored.

## Open

`*` is spoken for, so **multiplication is `x` and `×`**, never `*`. The alternative
was making the lexer stateful — `*` meaning multiply inside a `math` block and text
outside — which costs an unbalanced brace turning the rest of a file into nonsense
tokens. That is the wrong thing for this language to trade for one character.

**A stored `str` cannot yet contain a newline.** Under the literal rule
`[*a\nb*]` is a backslash and an `n`, and juxtaposition is a print-time thing. Either
a value gets built from items the way a printed line is, or text with a newline in it
exists only at the moment of printing. Worth settling before `str` does anything real.

## Pieces side by side join, whether or not they are known

```quench
var.immut.str ['hello'] = [*Hello, * 'name' *!*];
```

Juxtaposition has meant *one thing after another* since the marks were decided. What
took a while was letting one of the pieces be something nobody knows until the
program runs — and the reason it took a while was a wrong one written into the error:

```text
joining a name to something else builds a new value, and building one needs the
collector
```

It needs **allocation**, which exists — arrays and `e` have been allocating for
weeks — not **collection**, which does not. A built piece of text leaks exactly like
every array already does, which is stage one of the collector and the stage we are
at. Same shape of mistake as the constant array that was "not built yet": an error
promising a blocker that was not one.

### Where a built piece lives

A `Text` value is an index, and it was an index into the module's table. Now it is an
index into a table the *runtime* keeps, whose first entries **are** the module's —
laid out before anything runs, exactly like the constant array tables beside them.
Everything the program was written with comes first; everything it builds comes
after. Nothing else had to change: comparing text already compared what it holds
rather than which piece it is, which is now the only thing that could work.

### What still does not join

```quench
var.immut.str ['s'] = [*x* 'n'];   # 'n' is an `i64`
```

```text
this is an `i64`, and text is made of text.
Rule(s) broken: pieces side by side join, and nothing converts on its own
Tip(s): a `print` shows any type because showing is not joining — it writes one
        piece after another and builds nothing.
```

That tip is the whole distinction, and it is why `print` looked like it could already
do this. `print.stdout[str:*n = * 'n']` writes two things one after the other and
makes nothing; `[str:*n = * 'n']` under a `str` chain would have to *make* a piece of
text out of a number, and no rule in the language turns one into the other. Which
means there is a question outstanding: what turns a number into text, and what it is
called.
