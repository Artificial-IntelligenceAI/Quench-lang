//! Whether the generator writes the shapes it means to write.
//!
//! A differential oracle only checks what somebody generated. For a long while nothing
//! generated a function with a float in its signature, and a bug that crashed the Dev
//! JIT outright sat behind 200,000 programs a run saying everything agreed — because a
//! parameter and a return type are places a type appears that no amount of arithmetic
//! in a body will ever reach.
//!
//! So this counts. Not that the shapes *run* correctly, which is the oracle's job, but
//! that they are written down at all — because a step that never fires proves nothing
//! and looks exactly like a step that fires and passes.

use std::collections::BTreeMap;

use quench_qir::Inst;

/// How many times each shape appears across a batch of generated programs.
fn written(seeds: std::ops::RangeInclusive<u64>) -> BTreeMap<String, usize> {
    let module = quench_gen::batch(&seeds.collect::<Vec<_>>());
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for func in &module.functions {
        for ty in &func.params {
            *seen.entry(format!("takes {}", ty.name())).or_default() += 1;
        }
        *seen.entry(format!("gives {}", func.ret.name())).or_default() += 1;
        for block in &func.blocks {
            for (_, inst) in &block.insts {
                let name = match inst {
                    Inst::CallHost { host, .. } => host.name().to_string(),
                    Inst::Bin { op, .. } => format!("{op:?}"),
                    Inst::CmpU { .. } => "cmp-unsigned".to_string(),
                    Inst::Call { .. } => "call".to_string(),
                    Inst::Narrow { .. } => "narrow".to_string(),
                    Inst::FCmp { .. } => "fcmp".to_string(),
                    _ => continue,
                };
                *seen.entry(name).or_default() += 1;
            }
        }
    }
    seen
}

#[test]
fn every_shape_the_generator_means_to_write_is_written() {
    let seen = written(1..=2000);
    let missing: Vec<&str> = [
        // Whole numbers, signed and unsigned, wrapping and stopping.
        "Add", "Sub", "Mul", "DivTruncated", "DivFloored", "RemTruncated", "RemFloored",
        "AddTrapping", "SubTrapping", "MulTrapping",
        "DivU", "RemU", "AddTrappingU", "SubTrappingU", "MulTrappingU", "cmp-unsigned",
        "pow-i64", "pow-i64-trapping",
        // Binary floats, plain and stopping, and the narrow one being put back.
        "FAdd", "FSub", "FMul", "FDiv",
        "FAddChecked", "FSubChecked", "FMulChecked", "FDivChecked", "to-b16",
        // The maths IEEE 754 requires, which every engine must round identically.
        "float-alone", "float-paired", "float-fused", "float-slow", "float-power",
        // Decimals, whose arithmetic both engines share and whose plumbing they do not.
        "decimal-read", "decimal-add", "decimal-sub", "decimal-mul", "decimal-div",
        "decimal-compare",
        // Arrays, and with them the collector and the Dev JIT's shadow root slots.
        "array-new", "array-get", "array-set", "array-len", "array-push", "array-copy",
        "array-equal",
        // `e`, whose arithmetic both engines share and whose plumbing they do not.
        "exact-read", "exact-add", "exact-sub", "exact-mul", "exact-div", "exact-pow",
        "exact-compare",
        // Text, which is the other thing built while a program runs.
        "text-join", "text-compare", "text-clusters", "text-letters",
        // `stitch`, which is the same formatting as a `print` reached another way.
        "say-i64", "say-u64", "say-float", "say-decimal", "say-exact", "say-bool",
        "say-array",
        // Both shapes of `and`/`or`, and a call.
        "And", "Or", "call",
    ]
    .into_iter()
    .filter(|shape| !seen.contains_key(*shape))
    .collect();
    assert!(missing.is_empty(), "never generated: {missing:?}\nwritten: {seen:#?}");
}

#[test]
fn a_type_appears_where_only_a_signature_can_put_it() {
    // The gap that let a compiler crash through. A body reaches every type it uses; a
    // parameter and a return type are reached by nothing but a declaration.
    let seen = written(1..=200);
    for shape in ["takes b64", "takes b32", "takes b16", "gives b64", "gives b32", "gives b16"] {
        assert!(seen.contains_key(shape), "never generated: {shape}\nwritten: {seen:#?}");
    }
}

#[test]
fn no_shape_is_so_rare_that_generating_it_is_luck() {
    // A shape written once in two thousand programs is one a run can miss. These are
    // the ones a step has to reach often enough to be worth calling tested.
    let seen = written(1..=2000);
    for shape in [
        "array-get", "array-set", "array-push", "array-copy", "decimal-div",
        "exact-div", "exact-pow", "text-join", "text-compare",
    ] {
        let n = seen.get(shape).copied().unwrap_or(0);
        assert!(n > 100, "`{shape}` was written {n} times in two thousand programs");
    }
}

#[test]
fn what_a_program_prints_is_compared_too() {
    // An answer is one `i64`. What a program *prints* is everything else it says, and
    // every number type has its own way of being written down -- the shortest text
    // that reads back as the same float, a `u64` read as unsigned, a decimal's cohort,
    // an `e` with or without a denominator. None of that was compared until now, so
    // seven host calls' worth of formatting sat outside the oracle entirely.
    let seen = written(1..=2000);
    for shape in [
        "print-i64", "print-u64", "print-float", "print-decimal", "print-exact",
        "print-text", "print-array",
    ] {
        let n = seen.get(shape).copied().unwrap_or(0);
        assert!(n > 100, "`{shape}` was written {n} times in two thousand programs");
    }
}
