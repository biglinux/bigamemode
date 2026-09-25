//! Print what each GPU is doing now, as the Details page reads it.
//!
//! Usage: `gpu_telemetry [samples] [interval-ms]`
fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (n, ms) = (
        args.first().copied().unwrap_or(1),
        args.get(1).copied().unwrap_or(1000),
    );
    let hw = bigame_core::hardware::Hardware::detect();
    for i in 0..n {
        for g in &hw.gpus {
            let s = bigame_core::gpu_telemetry::sample(g);
            println!("{} {:?} {} {s:?}", g.card, g.vendor, g.driver);
            if g.driver == "nvidia" {
                println!(
                    "  graphics contexts: {:?}",
                    bigame_core::gpu_telemetry::nvidia_graphics_pids(&g.pci_slot)
                );
            }
        }
        if i + 1 < n {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }
}
