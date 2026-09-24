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
            Action::Rekey { file, from, to, .. } => {
                println!("REKEY      {}  '{from}' → '{to}'", file.display());
            }
            Action::Clean { file, name, .. } => println!(
                "CLEAN      {}  '{name}' (drop fields falcond ignores)",
                file.display()
            ),
            Action::Keep { file, reason } => println!("KEEP       {}  ({reason})", file.display()),
            Action::Unresolved { file, name } => println!(
                "UNRESOLVED {}  '{name}': no installed game has that title",
                file.display()
            ),
        }
    }
    if !apply {
        println!("\n(dry run — nothing changed; pass --apply to migrate)");
        return Ok(());
    }
    let state = std::env::var_os("HOME")
        .map(|h| Path::new(&h).join(".local/state/bigame-mode"))
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let (backup, done) = migration::apply(&plan, user, &state)?;
    println!("backup: {}", backup.display());
    for line in done {
        println!("migrated: {line}");
    }
    Ok(())
}
