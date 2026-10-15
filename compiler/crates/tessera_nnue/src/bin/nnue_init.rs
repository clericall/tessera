//! `nnue_init <path> [hsize] [seed]` — writes a deterministic placeholder
//! weight file so the engine's NNUE inference path can be tested before any
//! real training exists.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: nnue_init <path> [hsize] [seed]");
        std::process::exit(2);
    }
    let path = &args[1];
    let hsize: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(32);
    let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20261014);
    if hsize <= 0 {
        eprintln!("hsize must be positive");
        std::process::exit(2);
    }
    match tessera_nnue::write_fake(path, hsize, seed) {
        Ok(n) => println!("wrote {path} (hsize={hsize}, {n} bytes)"),
        Err(e) => {
            eprintln!("failed to write {path}: {e}");
            std::process::exit(1);
        }
    }
}
