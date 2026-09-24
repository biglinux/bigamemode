//! Print AI Graphics' status for the game running now, as the Home card
//! reads it.
fn main() {
    match bigame_core::running::detect() {
        None => println!("no game running"),
        Some(g) => println!(
            "{} (pid {}): {:?}",
            g.process_name,
            g.pid,
            bigame_core::graphics::status_running(&g)
        ),
    }
}
