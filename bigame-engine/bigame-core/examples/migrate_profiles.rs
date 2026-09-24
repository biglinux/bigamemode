//! Show — and with `--apply`, carry out — the migration of profiles written
//! by older BiGame-mode versions. A backup is taken before anything changes.
use bigame_core::migration::{self, Action};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let user = Path::new(bigame_core::profiles::USER_PROFILES_DIR);
    let installed = bigame_core::games::detect_all();
    let plan = migration::plan(user, &installed);
    for action in &plan {
        match action {
            Action::Rekey { file, from, to, .. } => println!("REKEY      {}  '{from}' → '{to}'", file.display()),
            Action::Clean { file, name, .. } => println!("CLEAN      {}  '{name}' (drop fields falcond ignores)", file.display()),
            Action::Keep { file, reason } => println!("KEEP       {}  ({reason})", file.display()),
            Action::Unresolved { file, name } => println!("UNRESOLVED {}  '{name}': no installed game has that title", file.display()),
        }
    }
    if !apply {
        println!("\n(dry run — nothing changed; pass --apply to migrate)");
        return Ok(());
    }
    let state = std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/state/bigame-mode")).unwrap();
    let dest = migration::backup(user, &state)?;
    println!("backup: {}", dest.display());
    let proxy = bigame_core::dbus_client::daemon_proxy_blocking()?;
    for action in &plan {
        match action {
            Action::Rekey { file, to, content, .. } => {
                proxy.save_profile(to, content)?;
                let old = file.file_stem().unwrap().to_string_lossy();
                proxy.delete_profile(&old)?;
                println!("migrated {old} → {to}");
            }
            Action::Clean { file, content, .. } => {
                let stem = file.file_stem().unwrap().to_string_lossy();
                proxy.save_profile(&stem, content)?;
                println!("cleaned {stem}");
            }
            _ => {}
        }
    }
    Ok(())
}
