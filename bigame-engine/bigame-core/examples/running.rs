//! Identify the running game the way Turbo does, and the profile falcond
//! would give it. `--known` lists the native games this machine knows.
use bigame_core::running;

fn main() {
    if std::env::args().any(|a| a == "--known") {
        let mut known: Vec<_> = running::known_native_games().into_iter().collect();
        known.sort();
        for (process, name) in known {
            println!("{process:32} {name}");
        }
        return;
    }
    let t = std::time::Instant::now();
    let game = running::detect();
    let took = t.elapsed();
    match game {
        None => println!("no game running ({took:?})"),
        Some(g) => {
            let mode = bigame_core::config::read()
                .map(|c| c.profile_mode)
                .unwrap_or_default();
            println!("{g:#?}");
            println!("detected in {took:?}");
            println!(
                "falcond profile for '{}': {:?}",
                g.process_name,
                running::matching_profile(&g.process_name, &mode)
            );
        }
    }
}
