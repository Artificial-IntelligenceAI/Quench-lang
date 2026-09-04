# Five lines a name can cross

**Decided, not built.** Nothing in this note exists in the compiler yet. It is written
now because the decisions were made in one sitting and would otherwise have to be made
again, worse. Successor to
[three lines a name can cross](three-lines-a-name-can-cross.md), whose last section
said this note would have to exist.

## A module is a block inside a file, and it nests

```quench
module ['maths'] {
    module ['trig'] {
        fn.parent.b64 ['quarter turn'] [immut.b64 'x'] { … }
    }

    fn.module.b64 ['reduce'] [immut.b64 'x'] { … }
    fn.export.b64 ['sin'] [immut.b64 'x'] { … }
}
```

Not a file. A file may hold several, and a module may hold others.

The alternative — a file *is* a module, the way Go, Haskell, OCaml and Zig have it —
was the cheaper answer and would have left the ladder at three words untouched. It was
turned down because a module inside a file is the only way to say **a helper shared by
some modules and hidden from the rest of the file**: `reduce` above is callable from
`trig` and from `roots`, and by nothing else in the file. Flat modules can only offer
"this module" or "the whole file", and the case falls between them.

That is the same argument the three-lines note makes for why `program` is not
redundant — *visible to my own code, invisible to my users* — asked one level down.

## The ladder, narrowest first

| word | who may name it |
| --- | --- |
| `module` | this module, and everything nested inside it |
| `parent` | the module around this one, and everything under that — siblings included |
| `file` | only code in this file |
| `program` | anything compiled together |
| `export` | code that imports this as a library |

`module` reaching **downward into descendants** is not a preference, it is what the case
above needs: `reduce` is declared in `maths` and called from `maths.trig`, so a child
must be able to see its ancestors. The consequence is that a parent cannot see into a
child, which is what `parent` is for and why it is a separate word.

`parent` goes **one rung up and no further**. Rust's `pub(in path)` generalises it to
any depth; that is turned down here, because wanting to expose something three levels up
is evidence the nesting is too deep rather than evidence the language is missing a word.

Five is where it stops. What made Rust's ladder five and Go's two is nesting, and Quench
now nests — so the fifth word was written down at the same time as the fourth rather
than discovered later, since a ladder that grows a rung after people have written code
is a ladder that was wrong when they wrote it.

### Where each word is refused

`module` and `parent` name boundaries that may not exist. At the top level of a file
there is no enclosing module, and in a module with no module around it there is no
parent. Both are refused there, the way `any` outside a function is: a hole with nothing
to fill it, said as such rather than quietly widened.

## A path is marks, dots, marks

```quench
call 'maths'.'sin'[b64:*1.0*]
call 'maths'.'trig'.'quarter turn'[b64:*1.0*]
```

The rule is the one `call` already had and is unchanged by any of this: **a bare word is
Quench's and a marked name is the writer's.** A module is something the writer named, so
it wears marks; so does what is inside it; and the dots between them are the dots the
language already has.

Which means the marks say whether the *module* is Quench's, too. If the maths ever moves
behind a namespace — the thing on
[what the library owes](what-the-library-owes.md)'s list, which would take
twenty-eight trigonometry words out of `quench words` — it reads:

```quench
call maths.sin[b64:*1.0*]        # bare: Quench's module, Quench's function
call 'maths'.'sin'[b64:*1.0*]    # marked: a module somebody wrote and called maths
```

Both may sit in one program, which is the property that already lets `call count` and
`call 'count'` coexist. A path is uniformly one or the other — nobody adds to a module
Quench ships, so there is no half-marked path to worry about.

### The spelling that was considered and dropped

`call 'maths.sin'[…]`, with the dot inside one pair of marks, reads better and was
rejected for three reasons, in increasing order of weight:

1. Marks wrap **one** name everywhere else in the language.
2. There is only **one kind of dot**: `var.immut.i64`, `print.stdout`, `fn.file.any`,
   `call is.i64` all separate links *outside* marks. A dot inside marks would be a
   second dot with a different job.
3. It **narrows what a name may hold.** A dot would stop being an ordinary character, so
   a function called `f.g` would need escaping. Quench's line is that a name holds
   anything a line holds, and the three-lines note is explicit that widening later is
   safe while narrowing is not.

The objection to the version that won is that the second pair of marks carries no
information about *ownership* — once `'maths'` is marked, nothing after the dot could
be Quench's. That is true, and it is an argument about verbosity rather than a rule, so
it lost to three rules.

## `import` is a different feature, and it is the big one

Three things get called "modules" and only the first is settled here.

- **Grouping inside a file.** This note. Buildable against the compiler as it stands.
- **Naming across files.** What `import` means, and what was asked for. The syntax is
  the small part: nothing in the compiler reads more than one file, so a program has no
  way to say which files it *is*, no resolution across them, and no rule for two files
  declaring one name. It is also the only thing that makes `file` and `program` differ —
  they are checked and recorded today against a boundary that does not exist.
- **A shorter name.** Something that binds `'maths'.'trig'` to a local alias, because
  writing the path every time is tedious. Sugar, on either of the other two, and not
  decided.

### What `import` will run into

An exported **generic** function is a pattern, not a function, and the copies are made
where it is called — see [a hole is not a name](a-hole-is-not-a-name.md). So an exported
`largest` needs its **body** available in the file that calls it, not merely its
signature. Every monomorphising language hits this: C++ puts templates in headers, Rust
ships MIR inside an rlib.

It decides what an exported library physically *is*, so it wants answering before
`import` is designed rather than after.
