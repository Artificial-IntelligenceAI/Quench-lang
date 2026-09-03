//! Running every generated program every way, and insisting on one answer.
//!
//! # Where the time goes, and what to do about it
//!
//! Compiling a small program with the Dev JIT costs about 103µs; running the result
//! costs about 292ns. Compilation is roughly **352 times** the execution, and Cranelift
//! is the fast compiler. Two things follow, and they are the whole design:
//!
//! - **Batch.** Many programs go into one module, so one compilation covers all of them.
//!   Each is still called and compared on its own; only the expensive part is shared.
//!   This is worth more than any amount of parallelism, because it removes work rather
//!   than spreading it.
//! - **Then spread what is left.** Batches are claimed from a shared counter rather than
//!   dealt out in advance, which matters on this machine specifically: an Apple M5 has
//!   performance cores and efficiency cores, so a fixed share per worker leaves the fast
//!   ones idle while the slow ones finish. Claiming keeps every core busy to the end.
//!
//! No thread pool crate is used, because none is needed for this and the project has no
//! third-party dependencies at all.

use crate::write::{batch, name_of, settings_for};
use quench_conf::{Optimise, Settings};
use quench_interp::Outcome;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One program that two engines answered differently. The seed is enough to rebuild it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Disagreement {
    pub seed: u64,
    /// The settings the program meant something under. Without this a disagreement
    /// cannot be reproduced, because the same seed is a different language under
    /// different settings.
    pub settings: Settings,
    /// What each way of running it said, named. A list rather than a pair, because a
    /// third engine and further optimisation levels join this rather than replace it.
    pub answers: Vec<(String, Told)>,
}

/// What an engine said.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Told {
    Answered(i64),
    Stopped(String),
    /// The engine could not run it at all, which is a bug in the generator or the IR.
    Refused(String),
}

/// What a run found.
#[derive(Debug, Default)]
pub struct Report {
    pub programs: usize,
    pub batches: usize,
    pub workers: usize,
    pub elapsed: Duration,
    pub disagreements: Vec<Disagreement>,
}

impl Report {
    pub fn agreed(&self) -> bool {
        self.disagreements.is_empty()
    }

    /// Programs per second, which is the number worth watching.
    pub fn rate(&self) -> f64 {
        self.programs as f64 / self.elapsed.as_secs_f64().max(f64::MIN_POSITIVE)
    }
}

/// How many workers to use by default: everything the machine has.
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Every way a program is run and compared.
///
/// The interpreter is first because it is the reference: it generates no code and
/// transforms nothing, so when these disagree it is the one to believe.
///
/// The two Cranelift levels are both here even though neither may change an answer —
/// that is exactly the claim being checked. An optimisation level is not a different
/// language, so it multiplies nothing that has to be *proven*; it is a different path
/// through the backend, so it multiplies what is worth *testing*, and it is free.
const WAYS: [(&str, Option<Optimise>); 3] = [
    ("interpreter", None),
    ("dev-jit", Some(Optimise::None)),
    ("dev-jit @ speed", Some(Optimise::Speed)),
];

/// Check every seed in `seeds`, in batches, across `workers` threads.
pub fn check(seeds: &[u64], per_batch: usize, workers: usize) -> Report {
    let batches: Vec<&[u64]> = seeds.chunks(per_batch.max(1)).collect();
    let next = AtomicUsize::new(0);
    let found: Mutex<Vec<Disagreement>> = Mutex::new(Vec::new());
    let began = Instant::now();

    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            scope.spawn(|| {
                loop {
                    // Claimed rather than dealt out, so a worker that finishes early
                    // takes more instead of waiting for the slowest one.
                    let mine = next.fetch_add(1, Ordering::Relaxed);
                    let Some(seeds) = batches.get(mine) else { return };

                    let written = batch(seeds);

                    // Every module goes to bytes and back before anything runs it. The
                    // artefact is the thing a program *is* once it stops being source,
                    // so a format that lost something would be a wrong answer rather
                    // than a broken file — and this is where a wrong answer is caught.
                    let bytes = quench_qir::write(&written);
                    let module = match quench_qir::read(&bytes, "a generated module") {
                        Ok(back) => back,
                        Err(why) => {
                            let mut found = found.lock().expect("no worker panics holding this");
                            found.push(Disagreement {
                                seed: seeds[0],
                                settings: settings_for(seeds[0]),
                                answers: vec![(
                                    "the artefact".to_string(),
                                    Told::Refused(why.message.clone()),
                                )],
                            });
                            continue;
                        }
                    };
                    if module != written {
                        let mut found = found.lock().expect("no worker panics holding this");
                        found.push(Disagreement {
                            seed: seeds[0],
                            settings: settings_for(seeds[0]),
                            answers: vec![(
                                "the artefact".to_string(),
                                Told::Refused("what came back is not what went in".to_string()),
                            )],
                        });
                        continue;
                    }

                    // One compilation per level, however many programs are in the batch.
                    let mut compiled = Vec::new();
                    let mut refused = None;
                    for (name, level) in WAYS {
                        let Some(level) = level else { continue };
                        match quench_dev::compile_with(&module, level) {
                            Ok(built) => compiled.push((name, built)),
                            Err(why) => refused = Some((name, why.to_string())),
                        }
                    }
                    if let Some((name, why)) = refused {
                        let mut found = found.lock().expect("no worker panics holding this");
                        found.push(Disagreement {
                            seed: seeds[0],
                            settings: settings_for(seeds[0]),
                            answers: vec![(name.to_string(), Told::Refused(why))],
                        });
                        continue;
                    }

                    let mut disagreed = Vec::new();
                    for &seed in seeds.iter() {
                        let name = name_of(seed);
                        let mut answers: Vec<(String, Told)> = Vec::new();

                        answers.push((
                            "interpreter".to_string(),
                            match quench_interp::run_named(&module, &name) {
                                Ok(Outcome::Returned(v)) => Told::Answered(v),
                                Ok(Outcome::Trapped(t)) => Told::Stopped(t.describe().to_string()),
                                Err(why) => Told::Refused(why.to_string()),
                            },
                        ));
                        for (way, built) in &compiled {
                            answers.push((
                                (*way).to_string(),
                                match built.call_outcome(&name) {
                                    Some(Outcome::Returned(v)) => Told::Answered(v),
                                    Some(Outcome::Trapped(t)) => Told::Stopped(t.describe().to_string()),
                                    None => Told::Refused("no such function once compiled".into()),
                                },
                            ));
                        }

                        // Every way must say the same thing as the first.
                        if answers.iter().any(|(_, told)| *told != answers[0].1) {
                            disagreed.push(Disagreement {
                                seed,
                                settings: settings_for(seed),
                                answers,
                            });
                        }
                    }
                    if !disagreed.is_empty() {
                        let mut found = found.lock().expect("no worker panics holding this");
                        found.extend(disagreed);
                    }
                }
            });
        }
    });

    let mut disagreements = found.into_inner().expect("every worker has finished");
    // Sorted so a run is reproducible in its reporting as well as its programs.
    disagreements.sort_by_key(|d| d.seed);

    Report {
        programs: seeds.len(),
        batches: batches.len(),
        workers: workers.max(1),
        elapsed: began.elapsed(),
        disagreements,
    }
}
