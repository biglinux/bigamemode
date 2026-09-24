//! Identify the running game the way Turbo does, and the profile falcond
//! would give it.
use bigame_core::running;

fn main() {
    let t = std::time::Instant::now();
    let game = running::detect();
    let took = t.elapsed();
    match game {
        None => println!("no game running ({took:?})"),
        Some(g) => {
            let mode = bigame_core::config::read()
                .map(|c| c.profile_mode)
                .unwrap_or_default();
            println!("{:#?}", g);
            println!("detected in {took:?}");
            println!(
                "falcond profile for '{}': {:?}",
                g.process_name,
                running::matching_profile(&g.process_name, &mode)
            );
        }
    }
}
