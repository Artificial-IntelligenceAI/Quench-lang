//! Run a program and say what the heap kept, which is the one thing the oracle cannot
//! see: a collector that changed what a program answered would be a bug.
//!
//!     cargo run --release -p quench-lower --example kept -- program.qnl

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: kept <file.qnl>");
        std::process::exit(2);
    };
    let source = std::fs::read_to_string(&path).expect("a file");
    let out = quench_lower::lower(&source);
    let Some(module) = out.module else {
        let file = quench_diag::SourceFile::new(&path, &source);
        print!("{}", quench_diag::report(&file, &out.errors));
        std::process::exit(1);
    };
    let (outcome, kept) = quench_interp::run_kept(&module).expect("it runs");
    let (arrays, texts, exacts) = kept.live;
    println!("{outcome:?}");
    println!("still alive  {arrays} arrays, {texts} texts, {exacts} exact numbers");
    println!("collections  {}", kept.collections);
}
