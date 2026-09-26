fn main() {
    if let Err(error) = rynd::benchmarks::run() {
        eprintln!("Benchmark failed: {error}");
        std::process::exit(1);
    }
}
