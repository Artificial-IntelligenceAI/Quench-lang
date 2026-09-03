//! A module written down and read back, and every way a file can be wrong.

use quench_qir as qir;

/// A module with one of most things in it.
fn something() -> qir::Module {
    let mut module = qir::Module::new();
    let hello = module.intern("hello");
    module.table(vec![2, 3, 5]);

    let mut b = qir::Builder::new("twice", &[qir::Ty::I64], qir::Ty::I64);
    let n = b.param(0);
    let two = b.const_i64(2);
    let doubled = b.mul(n, two);
    b.ret(doubled);
    let twice = module.add(b.finish());

    let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);
    let one = b.const_i64(1);
    let called = b.call(twice, &[one], qir::Ty::I64);
    let text = b.const_text(hello);
    b.print(qir::Host::PrintText, qir::Stream::Out, text);
    let half = b.const_float(0.5f64.to_bits(), qir::Ty::F64);
    let sum = b.bin(qir::BinOp::FAdd, half, half);
    let same = b.fcmp(qir::CmpOp::Eq, sum, half);
    let held = b.const_handle(0);
    let len = b.call_host(qir::Host::ArrayLen, &[held]);

    let yes = b.block(&[]);
    let no = b.block(&[]);
    let join = b.block(&[qir::Ty::I64]);
    b.br_if(same, (yes, &[]), (no, &[]));
    b.switch_to(yes);
    b.jump(join, &[called]);
    b.switch_to(no);
    b.jump(join, &[len]);
    b.switch_to(join);
    let answer = b.block_param(join, 0);
    b.ret(answer);

    let start = module.add(b.finish());
    module.set_entry(start);
    module
}

#[test]
fn a_module_survives_being_written_down() {
    let module = something();
    let bytes = qir::write(&module);
    let back = qir::read(&bytes, "twice.qnlo").expect("it reads");
    assert_eq!(back, module, "what came back is what went in");
}

#[test]
fn a_file_that_is_not_one_says_so() {
    let wrong = qir::read(b"not a Quench artefact at all", "elsewhere.qnlo").expect_err("refused");
    assert_eq!(wrong.code, "E0801");
    assert!(wrong.message.contains("does not begin the way a Quench artefact does"), "{wrong:?}");
}

#[test]
fn a_copy_that_stopped_early_says_so() {
    let bytes = qir::write(&something());
    for cut in [8, bytes.len() / 2, bytes.len() - 1] {
        let wrong = qir::read(&bytes[..cut], "half.qnlo").expect_err("refused");
        assert_eq!(wrong.code, "E0801", "cut at {cut}");
    }
}

#[test]
fn a_byte_that_changed_says_so() {
    // What the sum is for. Not a defence -- anybody editing on purpose recomputes it
    // in a line -- but a bad copy or a disk going wrong looks exactly like this.
    let bytes = qir::write(&something());
    let mut damaged = bytes.clone();
    let at = bytes.len() - 4;
    damaged[at] ^= 0xff;
    let wrong = qir::read(&damaged, "bad.qnlo").expect_err("refused");
    assert_eq!(wrong.code, "E0801");
    assert!(wrong.message.contains("does not add up"), "{wrong:?}");
}

#[test]
fn a_version_this_does_not_know_is_refused_rather_than_read_halfway() {
    let mut bytes = qir::write(&something());
    bytes[4] = 99;
    let wrong = qir::read(&bytes, "later.qnlo").expect_err("refused");
    assert!(wrong.message.contains("version 99"), "{wrong:?}");
    assert!(
        wrong.fixes.iter().any(|f| f.contains("read it with the one that wrote it")),
        "{wrong:?}"
    );
}

#[test]
fn a_file_is_checked_the_way_an_arrival_is() {
    // `verify` runs on load, and its findings are addressed to somebody holding a file
    // rather than to whoever is writing the compiler.
    let mut module = something();
    // A jump to a block that is not there. Nothing this compiler builds looks like this.
    let last = module.functions.len() - 1;
    module.functions[last].blocks[0].term = qir::Term::Jump { to: qir::BlockId(99), args: Vec::new() };
    let bytes = qir::write(&module);
    let wrong = qir::read(&bytes, "odd.qnlo").expect_err("refused");
    assert_eq!(wrong.code, "E0801", "an arrival, not a bug in Quench: {wrong:?}");
}

#[test]
fn every_type_and_every_operation_makes_the_round_trip() {
    // The codes are written out one by one rather than derived from declaration order,
    // so this is what says none of them was missed.
    let mut module = qir::Module::new();
    let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);
    let l = b.const_i64(7);
    let r = b.const_i64(3);
    for op in [
        qir::BinOp::Add, qir::BinOp::Sub, qir::BinOp::Mul,
        qir::BinOp::AddTrapping, qir::BinOp::SubTrapping, qir::BinOp::MulTrapping,
        qir::BinOp::DivTruncated, qir::BinOp::RemTruncated,
        qir::BinOp::DivFloored, qir::BinOp::RemFloored,
    ] {
        b.bin(op, l, r);
    }
    for op in [qir::CmpOp::Eq, qir::CmpOp::Ne, qir::CmpOp::Lt, qir::CmpOp::Le, qir::CmpOp::Gt, qir::CmpOp::Ge] {
        b.cmp(op, l, r);
    }
    let f = b.const_float(1.5f64.to_bits(), qir::Ty::F64);
    for op in [
        qir::BinOp::FAdd, qir::BinOp::FSub, qir::BinOp::FMul, qir::BinOp::FDiv,
        qir::BinOp::FAddChecked, qir::BinOp::FSubChecked, qir::BinOp::FMulChecked, qir::BinOp::FDivChecked,
    ] {
        b.bin(op, f, f);
    }
    for op in [qir::CmpOp::Eq, qir::CmpOp::Lt] {
        b.fcmp(op, f, f);
    }
    let yes = b.const_bool(true);
    b.bin(qir::BinOp::And, yes, yes);
    b.bin(qir::BinOp::Or, yes, yes);
    b.not(yes);
    b.ret(l);
    let start = module.add(b.finish());
    module.set_entry(start);

    let back = qir::read(&qir::write(&module), "all.qnlo").expect("it reads");
    assert_eq!(back, module);
}
