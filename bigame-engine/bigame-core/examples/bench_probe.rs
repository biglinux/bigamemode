//! Report what this machine looks like to BiGame-mode.
//!
//! Meant to be run on many machines: hardware detection is the easiest place
//! to encode one computer as an assumption -- that a `card0` exists, that
//! cpufreq is present, that a GPU exposes a DPM control -- and the only way to
//! find those assumptions is to run the detection somewhere they are false.
use bigame_core::benchmark::provider;
use bigame_core::{capabilities::Capabilities, hardware::Hardware, inventory};

fn main() {
    let hw = Hardware::detect();

    println!("== inventory ==");
    println!(
        "{}",
        serde_json::to_string_pretty(&inventory::build(&hw)).unwrap_or_default()
    );

    println!("\n== render GPU ==");
    match hw.render_gpu() {
        Some(gpu) => println!(
            "{} ({}, {}) discrete={} vram={:?}",
            gpu.card, gpu.pci_id, gpu.driver, gpu.discrete, gpu.vram_total_bytes
        ),
        None => println!("none identified"),
    }

    println!("\n== capabilities ==");
    let caps = Capabilities::detect();
    println!("{caps:#?}");

    println!("\n== benchmark providers ==");
    for workload in provider::all() {
        println!("{:<14} {:?}", workload.id(), workload.availability());
    }
}
