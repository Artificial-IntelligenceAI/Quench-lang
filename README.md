# Quench

Project under development. Nothing here is stable, and stability is **not a guarantee**.

**Quench** is a language focused on three things: **explicit syntax**, **high
performance**, and **very helpful error messages**.

It runs three ways, and all three must agree:

| Method | Backend | For |
| --- | --- | --- |
| **Dev JIT** | Cranelift | Editing. Compiles in milliseconds, checks everything, and says the most about what went wrong. |
| **Hot JIT** | LLVM | Running. Starts cheap and recompiles a function once it has proved it is worth the optimiser's time. |
| **AOT native** | LLVM | Shipping. One binary that needs nothing on the machine it lands on. |

Agreement between them is not a hope, it is a test. See [The oracle](#the-oracle).

## Status

| Part | State |
| --- | --- |
| Diagnostics (`quench-diag`) | Working — the error format, spans, and grapheme/byte/cell-correct columns |
| Lexer, parser | Not started |
| Type checker, borrow checker | Not started |
| QIR (the shared IR) | Not started |
| Dev JIT (Cranelift) | Not started |
| Hot JIT / AOT (LLVM, C++) | Not started |
| Program generator, oracle | Not started |

## Decisions made

- **Memory is owned.** Ownership and moves, with a borrow checker. Not refcounting,
  not a collector.
- **Two host languages.** Rust for the frontend and the Cranelift Dev JIT; C++ for
  the LLVM Hot JIT and AOT native backend. They meet at a versioned, serialised IR
  rather than a shared header. See [notes/architecture.md](notes/architecture.md).
- **The error format comes from [Luarust](https://github.com/Artificial-IntelligenceAI/Luarust)**,
  unchanged in shape. It is the same author's, MIT, and it was already right.

## Decisions not made yet

- The **type system**.
- The **surface syntax**. Everything you see in a test fixture or a doc comment is a
  placeholder, and is labelled as one. `.qn` as the file extension is provisional too.

## Errors

An error names the rule that was broken, points at the line, and ends with the fix,
because the fix is what should still be on screen when the reader stops reading.

```text
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qn, line: 3, column: 6 (src/main.qn:3:6)

`greeting` was given away on line 2, so it cannot be used here.

  2 | give greeting to shout;
    | ~~~~ given away here, and `text` is not copied
  3 | show greeting;
    |      ^^^^^^^^ used here, after it was given away

Error code: E0301
Rule(s) broken: a value has one owner, and giving it away ends the old owner's use of it
Tip(s): `text` owns a buffer, so giving it moves the buffer rather than copying it.
Suggested fix(s): line 2 — `lend greeting to shout;`, if `shout` only needs to read it

1 error.
```

The greeting is printed once however many errors follow, and the count once at the
end, so a program with twelve mistakes apologises once rather than twelve times.

A position is reported three ways at once, because a position is three different
numbers and only one of them is the one a person means: the column a reader is given
counts **graphemes**, the column in `file:line:column` counts **bytes** so it can be
pasted into an editor or a `grep`, and the caret is placed by **terminal cells**, so
an emoji that draws two cells wide gets two carets.

## The oracle

A language with three execution methods has three places for the same bug to hide.
So the methods are not trusted, they are tested against each other:

- a **program generator** writes Quench programs that are guaranteed to compile —
  built from the types outward, so the interesting case (a program that *runs*, and
  can therefore be answered differently by two engines) is the only case generated;
- every program is run by **every method, at every optimisation level**, and the
  answers must match — including the way a program *stops*, since stopping in the
  same place for the same reason is as much an agreement as printing the same number;
- the generator is built to **saturate the machine it runs on** rather than testing
  one program at a time.

Any disagreement is a bug in at least one engine, and the seed that produced it is
kept so it can be replayed.

## Building

```bash
cargo test
```

The LLVM half needs LLVM 22 and is not wired up yet. `llvm-config` is expected at
`/opt/homebrew/opt/llvm/bin/llvm-config` on macOS.

## Licence

MIT, © 2026 Tankun Sriket.
