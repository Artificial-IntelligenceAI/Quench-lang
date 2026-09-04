# Three lines a name can cross

A top-level declaration — a function, a type, a constant — says who may name it.
There are three answers, because Quench has three boundaries a name can be on the
far side of.

| word | who may name it |
| --- | --- |
| `file` | only code in this file |
| `program` | anything compiled together |
| `export` | code that imports this as a library |

Words, not initials. `fp` / `pw` / `ex` were considered and dropped: the reason to
abbreviate is volume, and the volume is already gone — a visibility word appears on
top-level declarations only, so a file carries a dozen rather than one per counter.
That leaves initials saving a few characters at the cost of the thing `START` was
named for, since a reader who has not met `fp` cannot recover it from the letters.
All three also land on something else first: `fp` on floating point (free here, as
Quench's floats are `b` and `d`, but the habit arrives with the reader), `pw` on
password, `ex` on extern.

Variables do not appear here. They live inside functions, where nothing outside can
name them however much it would like to, so there is nothing to permit or deny. See
[the top level does not run](the-top-level-does-not-run.md). This is why Quench needs
far fewer visibility words than Luarust did: Luarust wrote one on every counter,
because in Lua's lineage a variable had somewhere else it could have gone.

## Why the middle one is not redundant

File-private and exported look like they cover it. They do not, and the gap is the
common case.

Write a parser across three files — a lexer, a parser, and the `parse` that callers
use. The parser has to reach the lexer, so the lexer cannot be file-private. But no
*user* of the library should ever call the lexer directly, because the moment one
does, its shape is frozen: rewriting it becomes someone else's broken build.

So there is a real need for "visible to my own code, invisible to my users", and it
is not a convenience. It is the difference between a promise and an implementation
detail, which is the whole subject.

## Why not fewer, for now

Two levels would do until Quench can import anything, since program-wide and
exported are the same thing when nobody outside exists. They were both written down
anyway because the distinction is *already true* — a program-wide name is not a
promise and an exported one is — and because widening later is safe while narrowing
is not. A name that ships as exported cannot quietly become internal afterwards.

## What was not decided, and now is

Modules. Three levels assumed a file was the unit of privacy, and this note said that
if Quench grew modules inside a file the ladder would grow a rung.

It grew two. A module is a block inside a file, it nests, and the ladder is `module`,
`parent`, `file`, `program`, `export` — see
[five lines a name can cross](five-lines-a-name-can-cross.md). None of it is built yet,
so everything above is still what the compiler does; the successor note is what it is
going to do.
