# Five lines a name can cross

**Built.** Modules, the five words, nesting, paths, and `import` across files. Successor to
[three lines a name can cross](three-lines-a-name-can-cross.md), whose last section
said this note would have to exist.

## A module is a block inside a file, and it nests

```quench
module.file ['maths'] {
    module.file ['trig'] {
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

## How a name is found

Here first, then outward, and the innermost match wins:

```quench
module.file ['maths'] {
    fn.module.b64 ['reduce'] [immut.b64 'x'] { … }
    module.file ['trig'] {
        fn.parent.b64 ['quarter turn'] [immut.b64 'x'] { … }
    }
    fn.export.b64 ['sin'] [immut.b64 'x'] {
        give [call 'reduce'[call 'trig'.'quarter turn'['x']]];
    }
}
```

`'reduce'` is found in `maths` because that is where this is written.
`'trig'.'quarter turn'` is a *path*, and it goes through the same walk — so `maths` says
it without having to say `maths` again, and the top of the file says
`'maths'.'trig'.'quarter turn'` in full. One rule rather than two, and no word needed
for "start from the top", which is what Rust's `crate::` is.

What a module does not change is anything below the checker. A module decides which
names reach which code, and that is settled while checking; by the time anything is
lowered, a function is a function with a longer name — `maths.trig.quarter turn`. Two
modules may each hold a `'size'` and the file may hold a third.

## A module says who may see it, like everything at the top of a file

It was briefly the one exception to a rule the language states outright, and it is not
one now:

```quench
module.file ['maths'] {
    module.module ['trig'] { … }
    fn.export.b64 ['sin'] [immut.b64 'x'] { … }
}
```

`module.module` is the word twice and means what it says — the construct, then who may
see it. `trig` there is an implementation detail of `maths`: nothing outside may name
it, and what is *inside* it saying `export` does not change that. **A name never reaches
further than the module around it does**, which is the rule every nested privacy system
has and is worth writing down because it is the one people are surprised by.

So `'maths'.'trig'.'x'` asks three questions rather than one: may this see `maths`, may
it see `maths.trig`, and may it see `x`.

## A constant is reached the same way

```quench
module.file ['text'] { const.export.str ['MARK'] = [*!*]; }

START {
    var.immut.str ['m'] = ['text'.'MARK'];
}
```

Marks, dots, marks, in a value as much as at a call, and the same walk outward to find
it. Only a *constant* is ever named this way: a variable lives inside a function, so it
has no module to be in, and a function is reached with `call`, which has a path of its
own.

## `import`, and what a program is

Three things were getting called "modules". Two are built.

**What the program is** comes from `QNL-Config.toml`, not from the source — and each
file is given the name it will be imported by:

```toml
[program.files]
main = "main.qnl"
maths = "arithmetic.qnl"
text = "text.qnl"
```

**What a file uses** comes from the file:

```quench
import ['maths'];

START {
    print.stdout[call 'maths'.'sin'[*8.0*] \n];
}
```

Two mechanisms deliberately, and the split is the whole of the decision: the manifest is
the authority on *membership*, and `import` is a use-site record of *where a name came
from*. The alternative — every file sees every other, which is how Go works inside a
package — is fewer moving parts and gives up the thing `call` was made mandatory for.

**A file is a module named after itself.** `maths.qnl` is `maths`, so two files may each
hold a `'size'` and nothing collides, and a module *inside* an imported file is one more
link: `call 'maths'.'exact'.'third'[]`. Which does mean a module comes from two places, a
block and a file, exactly as it does in Rust.

### The name is chosen, not taken from the filename

`arithmetic.qnl` above is the module `maths`. The two need not agree, and that is the
point: **a filename is not an interface.** Renaming the file renames nothing and breaks
no caller; a file may sit in a directory without the directory leaking into the name; and
two files may share a stem, which the first cut had to refuse outright.

Rust splits this. Within a crate a module is named by its filename *and* declared with
`mod maths;` in the parent, so the two have to agree — and `#[path = "…"]` exists for
when they cannot. Across crates the name comes from the manifest instead, renameable
there or with `use x as y`. Quench has no dependencies yet, only files, and its files are
already doing the job crates do at that boundary — so the manifest names them, one level
down from where Rust does it.

What it costs is that a reader with `arithmetic.qnl` open cannot see what it is called
from inside it. Which is the argument for `import` naming it at the top of every file
that uses one: the name is written where it is *used*, which is where it matters.

### `file` finally means something

Until there could be a second file there was nowhere for `file` to be false, and the
three-lines note said so at the time. Now `fn.file.b64 ['halved']` in `maths.qnl` is
unreachable from `main.qnl`, and that is a real refusal rather than a recorded intention.

`program` and `export` still cannot be told apart, and the reason is the same one moved
along: telling *them* apart wants a second **program** using this one as a library, and
there is no such thing yet.

### What is still not built

**A shorter name.** Something binding `'maths'.'exact'` to a local alias, because writing
the path every time is tedious. Sugar, and not decided.

**A library that is not source.** An import reads `.qnl` and nothing else, for the reason
below.

### What `import` ran into

An exported **generic** function is a pattern, not a function, and the copies are made
where it is called — see [a hole is not a name](a-hole-is-not-a-name.md). So an exported
`largest` needs its **body** available in the file that calls it, not merely its
signature. Every monomorphising language hits this: C++ puts templates in headers, Rust
ships MIR inside an rlib.

Which decided what an import reads. Every file of a program is **source**, laid end to
end and checked as one, so a pattern's body is always there. A `.qnlo` artefact cannot
carry one — a hole is erased before QIR exists — so importing an artefact is a separate
question and not this one.

### And what it cost, which was less than expected

A `Span` is a byte range and carries no file, which is what lets it be `Copy` and be
handed around by the hundred. Widening it would have touched every site in the compiler.
So the files are **concatenated** and a span is a range into that, with a source map
turning one back into a file, a line and a column when a diagnostic is rendered. One lex,
one parse, and every item attributed to a file by where it sits.

Nothing below the checker learned anything. A function arrives at the lowering with a
longer name — `maths.sin` — and there is no other difference, which is the third time
that sentence has been true: modules, holes, and now files.
