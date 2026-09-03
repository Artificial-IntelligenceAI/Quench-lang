fn main() {
    let source = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let module = quench_lower::lower(&source).module.expect("a program");
    let (_, kept) = quench_interp::run_kept(&module).expect("it runs");
    let compiled = quench_dev::compile(&module).expect("it compiles");
    let (outcome, _) = compiled.run_capturing();
    let (a, t, e, c) = compiled.kept();
    println!("interpreter  live {:?}  collections {}", kept.live, kept.collections);
    println!("dev jit      live ({a}, {t}, {e})  collections {c}   {outcome:?}");
}
