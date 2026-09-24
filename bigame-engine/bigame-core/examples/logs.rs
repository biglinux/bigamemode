//! Read the journal the way the Logs page does, and summarise it.
fn main() -> anyhow::Result<()> {
    let t = std::time::Instant::now();
    let (entries, cursor) = bigame_core::logs::read(400, None)?;
    let took = t.elapsed();
    let mut by = std::collections::BTreeMap::<String, usize>::new();
    for e in &entries {
        *by.entry(format!("{:<9} {}", e.source.label(), e.level.label()))
            .or_default() += 1;
    }
    for (k, v) in by {
        println!("{k}  {v}");
    }
    println!(
        "{} entries in {took:?}; cursor {}",
        entries.len(),
        cursor.is_some()
    );
    for e in entries
        .iter()
        .filter(|e| e.level >= bigame_core::logs::Level::Warning)
        .rev()
        .take(6)
    {
        println!(
            "  {} [{}] {}",
            e.level.label(),
            e.source.label(),
            e.message.chars().take(110).collect::<String>()
        );
    }
    // An incremental read right after returns only what is new.
    let (more, _) = bigame_core::logs::read(400, cursor.as_deref())?;
    println!("incremental read: {} new", more.len());
    Ok(())
}
