//! How a game from the library is started through its own launcher, and the
//! launchers whose games would miss the session's settings.
//!
//! `cargo run -p bigame-core --example launch_through -- <title part> [--run]`
fn main() -> anyhow::Result<()> {
    let wanted = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    let run = std::env::args().any(|a| a == "--run");
    for game in bigame_core::games::detect_all() {
        if !game.name.to_lowercase().contains(&wanted) {
            continue;
        }
        match bigame_core::launchers::Start::for_game(&game) {
            Some(start) => {
                println!(
                    "{} [{}] by {}: {:?}",
                    game.name,
                    game.source.label(),
                    start.by,
                    start.argv
                );
                if run {
                    start.spawn()?;
                    println!("started");
                }
            }
            None => println!("{} [{}]: no launcher start", game.name, game.source.label()),
        }
    }
    println!(
        "behind the session: {:?}",
        bigame_core::launchers::behind_the_session()
    );
    Ok(())
}
