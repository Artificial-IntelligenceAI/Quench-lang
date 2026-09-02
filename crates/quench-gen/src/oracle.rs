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

use crate::write::{batch, name_of};
use quench_interp::Outcome;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One program that two engines answered differently. The seed is enough to rebuild it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Disagreement {
    pub seed: u64,
    pub interpreted: Told,
    pub compiled: Told,
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

                    let module = batch(seeds);
                    // One compilation, however many programs are in it.
                    let compiled = match quench_dev::compile(&module) {
                        Ok(compiled) => compiled,
                        Err(why) => {
                            let mut found = found.lock().expect("no worker panics holding this");
                            found.push(Disagreement {
                                seed: seeds[0],
                                interpreted: Told::Refused("(not reached)".into()),
                                compiled: Told::Refused(why.to_string()),
                            });
                            continue;
                        }
                    };

                    let mut disagreed = Vec::new();
                    for &seed in seeds.iter() {
                        let name = name_of(seed);
                        let walked = match quench_interp::run_named(&module, &name) {
                            Ok(Outcome::Returned(v)) => Told::Answered(v),
                            Ok(Outcome::Trapped(t)) => Told::Stopped(format!("{t:?}")),
                            Err(why) => Told::Refused(why.to_string()),
                        };
                        let ran = match compiled.call(&name) {
                            Some(v) => Told::Answered(v),
                            None => Told::Refused("no such function once compiled".into()),
                        };
                        if walked != ran {
                            disagreed.push(Disagreement {
                                seed,
                                interpreted: walked,
                                compiled: ran,
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
