//! The same failure, told to the two people it could be about.

use quench_diag::SourceFile;
use quench_qir::{verify, Audience, Builder, CmpOp, Module, Ty};

/// A module that does not check out: adding a bool to an i64.
fn malformed() -> Module {
    let mut b = Builder::new("START", &[], Ty::I64);
    let one = b.const_i64(1);
    let two = b.const_i64(2);
    let flag = b.cmp(CmpOp::Lt, one, two);
    let nonsense = b.bin(quench_qir::BinOp::Add, one, flag);
    b.ret(nonsense);
    let mut m = Module::new();
    let id = m.add(b.finish());
    m.set_entry(id);
    m
}

fn rendered(audience: Audience, origin: &str) -> String {
    let wrong = verify(&malformed()).unwrap_err();
    let diag = quench_qir::diagnose(&wrong, audience, origin);
    // No source to point at -- the trouble is in a module, not in a line -- so this
    // exercises the renderer's no-labels path as well.
    quench_diag::report(&SourceFile::new("<none>", ""), &[diag])
}

#[test]
fn a_module_we_built_ourselves_is_our_bug_and_says_so() {
    let out = rendered(Audience::Ourselves, "the Dev JIT");
    assert!(out.contains("This is a bug in Quench, not in your program."), "{out}");
    assert!(out.contains("Nothing you wrote can cause this."), "{out}");
    assert!(out.contains("please report it"), "{out}");
    assert!(out.contains("Error code: E9001"), "{out}");
    // It must not tell the reader to do something that cannot help.
    assert!(!out.contains("build it again from source"), "{out}");
}

#[test]
fn a_module_that_arrived_is_not_our_bug_and_does_not_claim_to_be() {
    let out = rendered(Audience::AFileWeWereGiven, "hello.qnl");
    assert!(out.contains("`hello.qnl` is not a Quench module this version can run."), "{out}");
    assert!(out.contains("stopped early"), "{out}");
    assert!(out.contains("build it again from source"), "{out}");
    assert!(out.contains("Error code: E0801"), "{out}");
    // The lie this whole change exists to prevent.
    assert!(!out.contains("bug in Quench"), "{out}");
}

#[test]
fn both_still_say_what_was_actually_wrong() {
    for (audience, origin) in
        [(Audience::Ourselves, "the Dev JIT"), (Audience::AFileWeWereGiven, "hello.qnl")]
    {
        let out = rendered(audience, origin);
        assert!(out.contains("what was wrong:"), "{out}");
        assert!(out.contains("wants i64"), "{out}");
    }
}

#[test]
fn a_diagnostic_with_nothing_to_point_at_still_renders() {
    // A module is not a line of source, so these carry no labels at all. The greeting,
    // the code, the rule and the fix all have to survive that.
    let out = rendered(Audience::AFileWeWereGiven, "hello.qnl");
    assert!(out.starts_with(quench_diag::GREETING), "{out}");
    assert!(out.contains("Rule(s) broken:"), "{out}");
    assert!(out.contains("Suggested fix(s):"), "{out}");
    assert!(out.ends_with("1 error.\n"), "{out}");
}
