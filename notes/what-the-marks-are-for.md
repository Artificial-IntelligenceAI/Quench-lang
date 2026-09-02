# What the marks are for

Quench writes three different things between marks, and uses a different mark for each:

| written | is | example |
| --- | --- | --- |
| `'…'` | a **name** | `var.str ['greeting']` |
| `\|…\|` or `` `…` `` | a **written value** | `= [\|1000\|]` |
| `*…*` | **text**, to print | `print[*Hello, World!*]` |

The point of three marks rather than one is that a mark answers the question by
itself. A quoted thing is a name wherever you meet it, and never has to be read as a
value because of where it happens to sit. Nothing about position decides what
something is.

That is also why a single reader habit — writing `"…"` — gets an error offering all
three, since the mistake is not knowing the question was three questions.

## Text is literal, and escapes stand outside it

Between `*` marks, everything is the character it looks like: emoji, braces,
punctuation, digits, semicolons, `|` bars, `'` quotes. None of it is a token, none of
it is interpreted.

The one exception is `\*`, which exists because the closing mark is the single
character that could not otherwise be written.

Escapes are **outside**:

```quench
print[*Hello, World!* 'name' \n];
```

`\n` is an item in the list, sitting next to the text rather than hidden inside it.
Which means `*a\nb*` is a backslash and an `n` — not a newline — and that is not a
trap, it is the whole design: reading a piece of Quench text never involves working
out which of its characters were secretly instructions.

The escapes are `\n`, `\t`, `\r` and `\\`. Anything else is an error that lists them.

## Joining is juxtaposition

Items in a print list sit next to each other. There is no `+`, and nothing to
concatenate, because nothing is being built — the list is the list, and it is printed
in order.

## Open

`*` is now spoken for, so **multiplication needs another spelling**. Luarust accepted
both `*` and `x`; Quench can only have the second, or something else entirely. Worth
settling before arithmetic is written rather than after.
