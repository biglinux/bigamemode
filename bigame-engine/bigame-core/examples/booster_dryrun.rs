//! Show the plan the Booster would apply, without applying it.
use bigame_core::booster::BoosterEngine;

fn main() {
    let engine = BoosterEngine::detect();
    let (_snapshot, plan) = engine.dry_run();

    println!("== would change ==");
    if plan.changes.is_empty() {
        println!("  (nothing)");
    }
    for change in &plan.changes {
        println!(
            "  {} : {} -> {}",
            change.knob.title(),
            change.from,
            change.to
        );
    }

    println!("\n== skipped ==");
    for skipped in &plan.skipped {
        println!("  {skipped:?}");
    }
}
