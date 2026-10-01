//! `gen-datasets`: writes (or verifies) the committed datasets.
//!
//! Usage:
//!
//! ```text
//! cargo run -p slim-bench --bin gen-datasets            # write datasets/*.json
//! cargo run -p slim-bench --bin gen-datasets -- --check # verify byte equality
//! ```
//!
//! The datasets are committed to the repository and must not be edited after
//! Phase 3 starts. `--check` exists so a later change to the generator or a
//! manual edit cannot silently alter a benchmark input: it fails if any
//! committed file differs from what the seeds produce.

use slim_bench::gen::{dataset_path, render_all, write_dataset};

fn main() {
    let check = std::env::args().any(|a| a == "--check");

    let mut failures = Vec::new();
    for (name, text) in render_all() {
        let path = dataset_path(&name);
        match (check, std::fs::read(&path)) {
            (true, Ok(existing)) if existing == text.as_bytes() => {
                println!("ok    {name} ({} bytes)", text.len());
            }
            (true, Ok(existing)) => {
                println!(
                    "DIFF  {name}: committed {} bytes, generated {} bytes",
                    existing.len(),
                    text.len()
                );
                failures.push(name);
            }
            (true, Err(e)) => {
                println!("MISS  {name}: {e}");
                failures.push(name);
            }
            (false, _) => {
                write_dataset(&name, &text).unwrap_or_else(|e| panic!("write {name}: {e}"));
                println!("write {name} ({} bytes)", text.len());
            }
        }
    }

    if check && !failures.is_empty() {
        eprintln!(
            "\n{} dataset(s) differ from the committed files: {}",
            failures.len(),
            failures.join(", ")
        );
        std::process::exit(1);
    }
    if !check {
        println!("\nregenerate with `--check` to verify reproducibility");
    }
}
