# Passes are a thing you have to ask for

Inherited from Luarust the hard way, and written down here before the C++ side exists
rather than after it ships wrong.

LLVM will happily produce machine code without ever optimising it, and will not
mention that it did. Luarust's JIT shipped like that: a module built at
`OptimizationLevel::Aggressive` was full of allocas and branches against a literal
divisor, because **that flag is the codegen level and runs no IR passes**. Nothing
had ever asked LLVM to optimise anything. Turning it on took the language from 1.85x
C to 1.05x — the largest single win it ever had, from one function call.

In C++ the same trap is waiting, wearing the same clothes. `CodeGenOptLevel` on a
`TargetMachine` is not the IR pipeline. The pipeline is built and run explicitly:

```cpp
llvm::PassBuilder PB;
llvm::ModuleAnalysisManager MAM;   // and the other three managers
PB.registerModuleAnalyses(MAM);    // ...and cross-register them
auto MPM = PB.buildPerModuleDefaultPipeline(llvm::OptimizationLevel::O2);
MPM.run(module, MAM);              // this line is the whole subject
```

Leave that last line out and everything still works. The code is correct, the tests
pass, the oracle agrees on every generated program, and the result is a third slower
for no visible reason.

## The oracle cannot catch this, and never will

This is the part worth internalising. Quench's correctness bar is three execution
methods that agree. A missing pass pipeline produces **correct** code. Every engine
answers identically, every generated program agrees, and the suite is green.

Agreement testing is blind to performance by construction. It is not that the oracle
is weak here; it is that the oracle is answering a different question, and no amount
of strengthening it will make it answer this one. Performance needs its own class of
guard.

## Write the guard, not the comment

Luarust's rule, and it holds: a comment cannot fail, and a timing assertion is flaky
and needs a quiet machine. Assert instead that two things which must match still
match.

Quench has three guards to write, and they go in with the C++ side rather than after
it:

1. **The Hot JIT and AOT must emit identical optimised IR for the same QIR module *at
   the same level*.** If they ever disagree, one of them is missing a pass — and this
   also catches the mistake in the other direction, a pass added to one path and not
   the other, which no benchmark would ever report.

   The words "at the same level" were added later and matter. An earlier draft said the
   two paths differ in where the machine code goes and not in what it says, which
   stopped being true the moment ahead-of-time output was told to optimise fully and
   take its time while the Hot JIT decides for itself, per function, as things prove
   worth it. They now have different *defaults* on purpose. Ask both for the same level
   and they must still agree; compare their defaults and you are comparing two correct
   answers to different questions.
2. **Optimised IR must differ from unoptimised IR** for a module chosen because it is
   obviously optimisable. This is the cheap smoke test that would have caught
   Luarust's bug on day one: it fails if the pipeline never ran at all.
3. **A named optimisation must be observably present.** Compile something whose
   folding is visible in the IR — a constant-folded arithmetic chain, a call that
   must inline — and assert the result. Deterministic, no benchmark, no quiet machine
   needed.

And one guard in the other direction, because Quench has a path Luarust did not:

4. **The Dev JIT must stay unoptimised.** Cranelift runs at `opt_level = none` on
   purpose — it is what makes it fast enough to sit behind an editor, and what makes
   it the reference the other two are measured against. Optimising it by accident
   would be the same class of mistake with the sign flipped, and would quietly cost
   the oracle the thing that makes its verdicts mean anything.

## The general shape

> Do not assert that an answer is good; assert that two things which must match still
> match.

That is Luarust's line and it is the reason everything else here works: three
execution methods agreeing, and now the two optimised paths agreeing with each other.
A guard of that shape needs no benchmark, no threshold, and no judgement about what
"fast enough" means — and it fails on the day somebody adds a fourth path and forgets.
