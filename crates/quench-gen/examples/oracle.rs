//! Run the oracle over a range of seeds and say what it found.
//!
//!     cargo run --release -p quench-gen --example oracle -- [programs] [per-batch]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let programs: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(20_000);
    let per_batch: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(64);
    let workers = quench_gen::cores();

    let seeds: Vec<u64> = (1..=programs).collect();
    println!("{programs} programs, {per_batch} per batch, {workers} workers, 3 ways each");

    let report = quench_gen::check(&seeds, per_batch, workers);

    println!("took     {:?}", report.elapsed);
    println!("rate     {:.0} programs/second", report.rate());
    println!("batches  {}", report.batches);
    if report.agreed() {
        println!("agreed   all {} of them", report.programs);
    } else {
        println!("DISAGREED on {}:", report.disagreements.len());
        for d in report.disagreements.iter().take(10) {
            println!("  seed {} under {:?}:", d.seed, d.settings);
            for (way, told) in &d.answers {
                println!("      {way:<16} {told:?}");
            }
        }
    }
}
