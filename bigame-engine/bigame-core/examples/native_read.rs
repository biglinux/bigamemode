//! Read a game's own benchmark output and print what the lab would record.
//!
//! ```sh
//! cargo run -p bigame-core --example native_read -- <frametimes.txt | benchmark_dir>...
//! ```
use std::path::Path;

use bigame_core::benchmark::native;

fn main() -> anyhow::Result<()> {
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let run = if path.is_dir() {
            native::read_cyberpunk(path)?
        } else {
            native::read_crystal(path)?
        };
        println!("{}", path.display());
        println!("  {}", run.describe());
        match run.capture.stats() {
            Some(s) => println!(
                "  avg {:.1} fps | 1% low {:.1} | 0.1% low {} | p99 {:.2} ms | stutters {}",
                s.avg_fps,
                s.low_1_fps,
                s.low_0_1_fps.map_or("n/a".into(), |v| format!("{v:.1}")),
                s.p99_ms,
                s.stutters
            ),
            None => println!("  too few frames for statistics"),
        }
    }
    Ok(())
}
