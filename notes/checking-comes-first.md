# Checking comes first

A trap ends the program. There is no recovering from one, anywhere, and that was
settled long before this note: a program entitled to carry on after an undefined
operation is a program a differential oracle cannot compare against itself. See
[the collector earns its place](the-collector-earns-its-place.md).

Which is right for a mistake in the *program* and useless for a mistake in the
*input*. `'xs'[*99*]` is a bug in the code and should stop. A file that says `hello`
where a number should be is not a bug in anything — it is Tuesday — and killing the
process over it is not an answer.

So: how does something fail without stopping?

## What the usual answer would have cost

Every language people reach for here hands back a value that is *either* an answer or
a reason — `Result`, `Either`, `Maybe`, a tuple with an `ok` in it. Quench cannot say
that. There are no sum types, no generics, and no enums a writer can declare. `give`
gives one thing. Scalars have no references, so an out-parameter would work for
`arr.i64` and for nothing else — `share` aliases an allocation, and a number is not
one.

Building the type properly means building generics first, and generics is a bigger
decision than this one. Four shapes were written out to avoid that:

- **`fails` on the chain.** `fn.file.i64.fails`, a `fail [reason]` beside `give`, and
  a call site that must branch. The reason would be a `str`, since `stitch` already
  builds those out of anything.
- **A fallback on the line.** `[call as.i64['line'] or *0*]`. Cheap, and it throws the
  reason away — a program could say that it failed and never what went wrong.
- **Two names.** `as number` stops, `try as number` branches. Nothing new in the
  language, everything fallible written twice, and which one you called is a name
  rather than something the checker holds you to.
- **Catchable traps.** One mechanism for everything, and a program that can catch its
  own index-off-the-end and carry on in a state nobody designed for. The eight trap
  reasons carry no payload either, so none of them could ever say *which line of which
  file*.

## What was chosen instead, and why

None of them. The language already had an answer and was not using it.

`'xs'[*99*]` stops. Dividing by nought stops. Neither has a safe twin — **you check
`count` first, you check the divisor first**, and if you did not, the trap is yours.
That is a complete and consistent failure model, it has been in the language since
arrays were, and nothing about input is different enough to need a second one.

So input works the same way. Every conversion ships with the question that guards it:

```quench
if call is.i64['line'] {
    var.immut.i64 ['n'] = [call as.i64['line']];
    print.stdout[str:*read * 'n' \n];
}
```

`as.i64` on `hello` stops, exactly like an index off the end. No new statement, no new
chain link on `fn`, nothing added to a signature, and **the oracle does not grow**: a
knob would have doubled sixty-four languages to a hundred and twenty-eight, and this
adds none.

## Why it is not a setting

Two answers usually means a `.toml` key here, and this looked like it qualified. It
does not, for two reasons. A semantic knob multiplies the oracle, and both arms have
to be built and to agree. More than that, a knob on *this* changes what a function
signature is — so a program written under one arm would not be readable under the
other, which is not a knob, it is two languages.

## The chain, because a bare word is one word

`call is number[…]` does not parse, and could not have. A bare word after `call` is
Quench's own name and a bare word is one token — which is why everything the language
provides is one word (`count`, `stitch`, `copysign`, `atan2`) and why those are the
only names in the language that cannot hold a space or an emoji, when a writer's can.
The marks are what buy spaces, and a provided name has none.

The only way something provided says a second thing is the chain, which `var.immut.i64`
and `print.stdout` are already. So the type goes there:

```quench
call is.i64['line']      call as.i64['line']
call is.b64['x']         call as.d64['price']
call is.e['ratio']       call as.bool['flag']
```

And it has to be said, because text says nothing about what it holds. `12` is an
`i64`, a `b64`, a `d32` and an `e`; `3.5` is three of those and not the first. "Is
this a number" was never one question.

## One reader, so the pair cannot drift

`is.i64` promises something about `as.i64`, and a promise two functions make separately
is a promise that comes apart. So they are not two functions. Both reach one entry in
`quench_num::read`, one asking whether there was an answer and the other asking what it
was — the same trick [`Host::PrintFloat` and `Host::SayFloat`](what-the-library-owes.md)
already use, where one implementation either writes the answer or hands it back.

The same functions read **written values in a source file**. `*42*` is an `i64` because
`read_whole` says so, and `call as.i64['42']` is an `i64` because `read_whole` says so.
There is no second grammar for numbers-at-runtime to be subtly wrong about, and no way
for one to accept something the other refuses.

Which fixes what each accepts, and it is worth being able to say it in one line: **the
text a value of that type could have been written with.** `infinity` is not a `b64`,
here or in a source file, because it is an answer a program can reach and not a thing
it can write. `-1` is not a `u8`. `3/4` is an `e`. `True` is not a `bool` and `true`
is.

## Where this holds, and where it will not

It holds for every question about a value already in hand — parsing, converting,
indexing, dividing. The check is honest there because nothing can change between
asking and doing.

It breaks on **files**. `call file exists['x']` and then `call open['x']` is a check
that cannot be made true: between those two lines the file can be deleted, and the
thing being asked about is not the program's to hold still. No predicate fixes that,
and pretending otherwise would be worse than not having one.

That hole is only a hole once files exist. Everything before them — a number out of
text, text out of a number, lines off standard input, the whole inverse of `stitch` —
works under "check first". So this is the answer now, and the day files land is the day
to look again, by which point there may be reasons to want generics that are about more
than one case.
