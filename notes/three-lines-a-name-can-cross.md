# Three lines a name can cross

A top-level declaration — a function, a type, a constant — says who may name it.
There are three answers, because Quench has three boundaries a name can be on the
far side of.

| level | who may name it |
| --- | --- |
| **file-private** | only code in this file |
| **program-wide** | anything compiled together |
| **exported** | code that imports this as a library |

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

## What is not decided

Modules. Three levels assume a file is the unit of privacy. If Quench grows modules
inside a file, the ladder grows a rung and this note gets rewritten.
