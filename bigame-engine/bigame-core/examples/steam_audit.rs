//! Report Steam per-game launch options that name a program which is absent.
fn main() {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let users = bigame_core::steam::users(&home);
    println!("steam running: {}", bigame_core::steam::is_running());
    for u in &users {
        println!("\naccount {} -> {}", u.id, u.config.display());
        let broken = bigame_core::steam::broken_launch_options(&u.config);
        if broken.is_empty() {
            println!("  no broken launch options");
        }
        for b in &broken {
            let name = app_name(&home, &b.app_id);
            println!(
                "  app {} {:<28} missing '{}'  options: {:?}",
                b.app_id, name, b.missing, b.options
            );
        }
    }
}

fn app_name(home: &std::path::Path, app_id: &str) -> String {
    for root in bigame_core::games::steam_libraries(home) {
        let m = root.join(format!("steamapps/appmanifest_{app_id}.acf"));
        if let Ok(c) = std::fs::read_to_string(&m) {
            if let Some(n) = bigame_core::games::acf_value(&c, "name") {
                return format!("({n})");
            }
        }
    }
    "(not installed)".into()
}
