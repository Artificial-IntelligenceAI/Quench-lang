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

## Juxtaposition builds a value, commas separate them

Items sit next to each other. There is no `+`, because nothing is being concatenated
into a third thing — the items *are* the value, used in order.

```quench
print[*Hello, World!* 'name' \n];
var.str ['s', 'ss'] = [*line one* \n *line two*, *idk* \n *Claude*];
```

The comma is the only thing that says where one value stops, which is exactly what
lets a value run to as many items as it likes.

This is also why C could not do it. A C string literal has to be **one value** — an
array of `char` to assign, pass and store — and `printf("Hello\n")` hands over a
single pointer, so there is nowhere for a separate escape to travel. Escapes had to
go inside because inside was the only place there was. Quench can put them outside
because `print[…]` takes a *list* and never builds a combined value at all.

## Open

`*` is spoken for, so **multiplication is `x` and `×`**, never `*`. The alternative
was making the lexer stateful — `*` meaning multiply inside a `math` block and text
outside — which costs an unbalanced brace turning the rest of a file into nonsense
tokens. That is the wrong thing for this language to trade for one character.

**A stored `str` cannot yet contain a newline.** Under the literal rule
`[*a\nb*]` is a backslash and an `n`, and juxtaposition is a print-time thing. Either
a value gets built from items the way a printed line is, or text with a newline in it
exists only at the moment of printing. Worth settling before `str` does anything real.
