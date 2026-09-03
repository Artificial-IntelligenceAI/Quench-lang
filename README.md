# The Quench website

The site for [Quench](https://github.com/Artificial-IntelligenceAI/Quench-lang), on its
own orphan branch: the same repository as the language, no shared history, and a working
tree of its own so that two people can write at once without one sweeping up the other's
half-finished files.

`main` is the language — Rust, three engines, and a README that is executed as a test.
This branch is the page about it. Nothing here is built by `cargo`, and nothing on `main`
knows this branch exists.

## Where the material is

Read, do not write: `/Users/ts/Quench`.

- `README.md` — every ```` ```quench ```` block in it is run by
  `crates/quench-cli/tests/readme.rs`, and every ```` ```text ```` block after one is
  checked character for character against what that program actually printed. It is the
  only prose about Quench that cannot be wrong.
- `notes/` — one file per decision, with the reasoning kept rather than summarised.
- `examples/` — programs.

**Copy examples from the README verbatim.** The syntax moves: `call` in front of every
call, marks around every name, `#3` for a three-line comment — all of that landed today.
Anything written from memory is already out of date.
