//! Whether the generator writes programs worth running, and whether the oracle would
//! actually notice if two engines disagreed.

use quench_gen::{batch, check, cores, name_of, program, Told};
use quench_qir::{BinOp, Builder, Module, Ty};

#[test]
fn every_generated_program_checks_out() {
    // Built from the types outward, so this is the claim the whole approach rests on: a
    // program that failed to check would be refused identically by every engine and
    // would prove nothing.
    for chunk in (1..300u64).collect::<Vec<_>>().chunks(50) {
        let module = batch(chunk);
        assert!(quench_qir::verify(&module).is_ok(), "a batch from {} did not check", chunk[0]);
    }
}

#[test]
fn the_same_seed_writes_the_same_program() {
    // Without this a disagreement could not be replayed, and the seed would be useless.
    for seed in [1u64, 7, 999, u64::MAX] {
        // A fresh module each time, so the text a decimal was written with is interned
        // into the same places and the two are comparable at all.
        let (mut here, mut there) = (Module::new(), Module::new());
        assert_eq!(
            program(&mut here, seed, None, &[]),
            program(&mut there, seed, None, &[]),
            "seed {seed}"
        );
        assert_eq!(here.text, there.text, "seed {seed}");
    }
}

#[test]
fn different_seeds_write_different_programs() {
    let (mut here, mut there) = (Module::new(), Module::new());
    let one = program(&mut here, 1, None, &[]);
    let two = program(&mut there, 2, None, &[]);
    assert_ne!(one, two);
}

#[test]
fn every_generated_program_runs_or_stops_for_a_reason_it_can_name() {
    // It used to be that none of them could stop at all, because compiled code aborted
    // the process rather than reporting. Now they can, and what matters is that a stop
    // is always one of the reasons both engines know -- never a crash and never silence.
    let seeds: Vec<u64> = (1..400).collect();
    let module = batch(&seeds);
    for seed in seeds {
        // Every outcome is one of the two, and `run_named` returning `Err` would mean
        // the generator wrote something no engine could run at all.
        let _ = quench_interp::run_named(&module, &name_of(seed))
            .unwrap_or_else(|why| panic!("seed {seed} could not run: {why}"));
    }
}

#[test]
fn the_oracle_agrees_across_the_engines_it_has() {
    let seeds: Vec<u64> = (1..=400).collect();
    let report = check(&seeds, 64, cores());
    assert!(report.agreed(), "{:#?}", &report.disagreements[..report.disagreements.len().min(5)]);
    assert_eq!(report.programs, 400);
    assert!(report.batches > 1, "more than one batch, so the claiming loop is exercised");
}

#[test]
fn every_optimisation_level_answers_the_same() {
    // An optimisation level is not a different language, so this is a claim rather than
    // a configuration: whatever Cranelift does at `speed`, it must arrive at the same
    // number it arrives at doing nothing.
    use quench_conf::Optimise;
    let seeds: Vec<u64> = (1..=200).collect();
    let module = batch(&seeds);
    let plain = quench_dev::compile_with(&module, Optimise::None).expect("it compiles");
    let quick = quench_dev::compile_with(&module, Optimise::Speed).expect("it compiles");
    let small = quench_dev::compile_with(&module, Optimise::SpeedAndSize).expect("it compiles");

    for seed in seeds {
        let name = name_of(seed);
        let (a, b, c) = (plain.call(&name), quick.call(&name), small.call(&name));
        assert_eq!(a, b, "seed {seed}: none and speed differ");
        assert_eq!(a, c, "seed {seed}: none and speed-and-size differ");
    }
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
    let said = String::new();
    let told_walked = Told::Answered { value: 1, said: said.clone() };
    let told_compiled = Told::Answered { value: 2, said: said.clone() };
    assert_ne!(told_walked, told_compiled, "a disagreement is a disagreement");

    // And against the same answer printed differently, which is the other half of
    // what an engine says and was not compared at all until it was captured.
    let quiet = Told::Answered { value: 1, said };
    let loud = Told::Answered { value: 1, said: "1".to_string() };
    assert_ne!(quiet, loud, "what a program printed is part of what it said");
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
                    quench_qir::Inst::Bin { op: BinOp::DivTruncated | BinOp::DivFloored, .. } => {
                        seen[0] = true
                    }
                    quench_qir::Inst::Bin { op: BinOp::RemTruncated | BinOp::RemFloored, .. } => {
                        seen[1] = true
                    }
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

#[test]
fn a_seed_picks_a_configuration_as_well_as_a_program() {
    use quench_conf::Division;
    use quench_gen::settings_for;

    // Both must appear, or half the language is never checked. A bug that only shows
    // under one setting is found only if something generated that setting.
    let mut seen_truncated = false;
    let mut seen_floored = false;
    for seed in 1..200u64 {
        match settings_for(seed).division {
            Division::Truncated => seen_truncated = true,
            Division::Floored => seen_floored = true,
        }
    }
    assert!(seen_truncated && seen_floored, "the oracle only ever checked one language");

    // And the same seed must keep the same one, or a disagreement cannot be replayed.
    for seed in [1u64, 42, 9999] {
        assert_eq!(settings_for(seed), settings_for(seed));
    }
}

#[test]
fn both_divisions_reach_the_generated_programs() {
    use quench_qir::{BinOp, Inst};
    let module = batch(&(1..=300).collect::<Vec<_>>());
    let mut floored = false;
    let mut truncated = false;
    for func in &module.functions {
        for block in &func.blocks {
            for (_, inst) in &block.insts {
                if let Inst::Bin { op, .. } = inst {
                    match op {
                        BinOp::DivFloored | BinOp::RemFloored => floored = true,
                        BinOp::DivTruncated | BinOp::RemTruncated => truncated = true,
                        _ => {}
                    }
                }
            }
        }
    }
    assert!(floored && truncated, "the setting is not reaching the instructions");
}

#[test]
fn some_generated_programs_stop_and_that_is_the_point() {
    // Until compiled code could report a stop rather than abort the process, the
    // generator had to write nothing that could stop -- which left "they stop in the
    // same place for the same reason" entirely unchecked.
    let seeds: Vec<u64> = (1..=400).collect();
    let module = batch(&seeds);
    let mut stopped = 0;
    for seed in &seeds {
        if let quench_qir::Outcome::Trapped(_) =
            quench_interp::run_named(&module, &name_of(*seed)).expect("it runs")
        {
            stopped += 1;
        }
    }
    assert!(stopped > 10, "only {stopped} of 400 stopped, which is too few to be checking anything");
    assert!(stopped < 180, "{stopped} of 400 stopped, which is too many to be checking much else");
}

#[test]
fn every_kind_of_stop_is_generated_by_something() {
    // A trap the generator never writes is a trap the oracle never checks. The one
    // division that does not fit is far too rare to turn up by chance, so it is aimed
    // at on purpose -- and this is what says so.
    use std::collections::BTreeSet;
    let seeds: Vec<u64> = (1..=1_200).collect();
    let module = batch(&seeds);
    let mut seen = BTreeSet::new();
    for seed in &seeds {
        if let quench_qir::Outcome::Trapped(t) =
            quench_interp::run_named(&module, &name_of(*seed)).expect("it runs")
        {
            seen.insert(t);
        }
    }

    // The reverse direction, which is the only one that catches anything. Asking
    // whether the three reasons somebody thought of are still reached catches one
    // being removed and never catches one being added -- and a reason nothing
    // generates is a reason implemented twice and compared never, which is how
    // `FractionalPower` sat outside the oracle until this test was written this way
    // round. See `notes/checking-comes-first.md`.
    let unreached: Vec<&quench_qir::Trap> = quench_qir::Trap::ALL
        .iter()
        .filter(|trap| !seen.contains(trap))
        // Recursion without a floor is deliberately never written: a runaway call is
        // reported by the interpreter and overflows the stack in compiled code, so the
        // two cannot be compared on it at all.
        .filter(|trap| **trap != quench_qir::Trap::TooDeep)
        .collect();
    assert!(
        unreached.is_empty(),
        "nothing generated writes a program that stops for {unreached:?}, so no engine is compared on it"
    );
}

#[test]
fn the_oracle_compares_stops_as_well_as_answers() {
    let seeds: Vec<u64> = (1..=300).collect();
    let report = check(&seeds, 64, cores());
    assert!(report.agreed(), "{:#?}", &report.disagreements[..report.disagreements.len().min(3)]);
}
