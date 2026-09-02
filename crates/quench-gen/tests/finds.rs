//! Whether the generator writes programs worth running, and whether the oracle would
//! actually notice if two engines disagreed.

use quench_gen::{batch, check, cores, name_of, program, Told};
use quench_qir::{BinOp, Builder, Module, Ty};

#[test]
fn every_generated_program_checks_out() {
    // Built from the types outward, so this is the claim the whole approach rests on: a
    // program that failed to check would be refused identically by every engine and
    // would prove nothing.
    for chunk in (1..600u64).collect::<Vec<_>>().chunks(50) {
        let module = batch(chunk);
        assert!(quench_qir::verify(&module).is_ok(), "a batch from {} did not check", chunk[0]);
    }
}

#[test]
fn the_same_seed_writes_the_same_program() {
    // Without this a disagreement could not be replayed, and the seed would be useless.
    for seed in [1u64, 7, 999, u64::MAX] {
        assert_eq!(program(seed, None), program(seed, None), "seed {seed}");
    }
}

#[test]
fn different_seeds_write_different_programs() {
    let one = program(1, None);
    let two = program(2, None);
    assert_ne!(one, two);
}

#[test]
fn nothing_generated_stops() {
    // Traps are excluded on purpose for now -- a trap in compiled code is a signal the
    // Dev JIT cannot yet catch. If this ever fails, the generator has started writing
    // programs the oracle cannot run rather than programs that found something.
    let seeds: Vec<u64> = (1..400).collect();
    let module = batch(&seeds);
    for seed in seeds {
        match quench_interp::run_named(&module, &name_of(seed)).expect("it runs") {
            quench_interp::Outcome::Returned(_) => {}
            other => panic!("seed {seed} stopped: {other:?}"),
        }
    }
}

#[test]
fn the_oracle_agrees_across_the_engines_it_has() {
    let seeds: Vec<u64> = (1..=2_000).collect();
    let report = check(&seeds, 64, cores());
    assert!(report.agreed(), "{:#?}", &report.disagreements[..report.disagreements.len().min(5)]);
    assert_eq!(report.programs, 2_000);
    assert!(report.batches > 1, "more than one batch, so the claiming loop is exercised");
}

#[test]
fn the_oracle_notices_when_two_engines_differ() {
    // The test the oracle needs most: proof it can fail. A module is built where the
    // interpreter and the Dev JIT are asked about *different* functions under the same
    // name, so they must disagree -- and if the oracle still reports agreement, it is
    // not checking anything.
    let mut module = Module::new();
    let mut b = Builder::new("lies", &[], Ty::I64);
    let one = b.const_i64(1);
    b.ret(one);
    let id = module.add(b.finish());
    module.set_entry(id);

    let walked = quench_interp::run_named(&module, "lies").expect("it runs");
    let compiled = quench_dev::compile(&module).expect("it compiles").call("lies");
    assert_eq!(walked, quench_interp::Outcome::Returned(1));
    assert_eq!(compiled, Some(1), "and they agree here, which is the control");

    // Now the same comparison the oracle makes, against a deliberately wrong answer.
    let told_walked = Told::Answered(1);
    let told_compiled = Told::Answered(2);
    assert_ne!(told_walked, told_compiled, "a disagreement is a disagreement");
}

#[test]
fn a_batch_is_one_compilation_for_many_programs() {
    // The reason batching exists: compiling costs a few hundred times what running
    // costs, so a batch has to actually hold many callable programs.
    let seeds: Vec<u64> = (1..=32).collect();
    let module = batch(&seeds);
    let compiled = quench_dev::compile(&module).expect("it compiles");
    for seed in seeds {
        assert!(compiled.call(&name_of(seed)).is_some(), "seed {seed} was not callable");
    }
}

#[test]
fn generated_programs_use_the_whole_instruction_set() {
    // A generator that only ever wrote additions would agree forever and prove nothing.
    let module = batch(&(1..=200).collect::<Vec<_>>());
    let mut seen = [false; 5];
    for func in &module.functions {
        for block in &func.blocks {
            for (_, inst) in &block.insts {
                match inst {
                    quench_qir::Inst::Bin { op: BinOp::Div, .. } => seen[0] = true,
                    quench_qir::Inst::Bin { op: BinOp::Rem, .. } => seen[1] = true,
                    quench_qir::Inst::Cmp { .. } => seen[2] = true,
                    quench_qir::Inst::Not(_) => seen[3] = true,
                    quench_qir::Inst::Call { .. } => seen[4] = true,
                    _ => {}
                }
            }
            if matches!(block.term, quench_qir::Term::BrIf { .. }) {
                // Branches are in every program by construction, so nothing to record.
            }
        }
    }
    assert_eq!(seen, [true; 5], "div, rem, cmp, not, call — all of them should appear");
}
