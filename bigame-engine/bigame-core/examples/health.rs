//! Print the system health checks the Diagnostics page shows.
fn main() {
    for c in bigame_core::health::collect() {
        println!(
            "{:<14} {:<24} {}",
            format!("{:?}", c.status),
            c.title,
            c.detail
        );
        if let Some(fix) = c.fix {
            println!("{:<39}→ {fix}", "");
        }
    }
}
