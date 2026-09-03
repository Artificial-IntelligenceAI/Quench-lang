# An array is the only thing two names can reach

Everything else in Quench is a value. Naming a number again is naming the number;
there is nothing behind it for a second name to get at. An array is not like that —
it lives on the heap, and a name holds a handle to it. So this line has two possible
meanings and they are not close together:

```quench
var.mut.arr.i64 (2) ['b'] = ['a'];
```

Either `'b'` is a second name for one array, and a change through either is seen
through both. Or `'b'` is a second array holding the same things, and the two go
their own ways. **Quench makes you say which**, and refuses the line above:

```text
this does not say whether it shares `'a'` or copies it.
Rule(s) broken: naming an array in a value says `share` or `copy`, and silence is
                not one of them
Tip(s): `share` makes a second name for one array, so a change through either is
        seen through both. `copy` makes a second array, and pays for it.
Suggested fix(s): `[share 'a']`, or `[copy 'a']`
```

```quench
var.mut.arr.i64 (2) ['shared'] = [share 'a'];
var.mut.arr.i64 (2) ['mine']   = [copy 'a'];
set ['a'[*1*]] = [*99*];
# a = [99 2], shared = [99 2], mine = [1 2]
```

## Why not just pick one

Every other collected language picks sharing, and it is a perfectly good answer:
free, and it is how you write a function that fills in an array you hand it. Picking
copying is also a good answer: no aliasing, ever, and comparing contents becomes the
obvious meaning of `==`.

What made neither of them right *as a default* is that both hide something, and they
hide opposite things:

- sharing hides that a `set` over here changes a thing over there, at a line that
  looks local;
- copying hides an allocation and a walk of the whole array, at a line that looks
  free. On a `(1000 1000)` that is a megabyte, silently, on an `=`.

This is the same shape as every other decision the language has made — `mut` against
`immut`, `temp` against `perm`, `nothing` against a type, `file` against `program`
against `export`. Each of those was somewhere a language could have had a default and
Quench made it say instead. This is a bigger silent difference than any of them.

Written where it is paid, which is the point. `share` costs nothing and compiles to
nothing: naming the variable already gave the handle, and sharing is what a handle
does. `copy` is a call into the runtime, and it is the one that had to be written
down because it is the one that costs.

## Which is why `==` could be built

While the two questions could not be told apart, comparing arrays was refused —
`['a' == 'b']` could have meant *the same array* or *the same contents*, and picking
one silently is the thing this note is about.

Once `share` exists, *the same array* has a way to be said and asked, and the other
question is free to be what `==` means:

```quench
var.immut.arr.i64 (2) ['a']    = [[*1* *2*]];
var.immut.arr.i64 (2) ['twin'] = [[*1* *2*]];
['a' == 'twin']                              # true. Two arrays, same contents.
```

Element by element, in a host call, so neither engine has its own idea of it. Two
arrays of different shapes are two different *types* and the comparison was already
refused before this — a `(2)` and a `(3)` never meet.

## What it does not fix

`share` through an `immut` name does not make the array immutable:

```quench
var.mut.arr.i64   (2) ['a'] = [[*1* *2*]];
var.immut.arr.i64 (2) ['b'] = [share 'a'];
set ['a'[*1*]] = [*99*];                      # 'b' shows [99 2]
```

`immut` means *this name cannot be used to change it*, which is what it has always
meant and is still true. It does not mean the thing never changes, and no language
with sharing gets that for free. Saying so here rather than letting somebody find it.
