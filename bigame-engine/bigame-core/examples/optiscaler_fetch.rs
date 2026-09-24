//! Download (or find in the cache) the recommended OptiScaler release,
//! verified, and list what was unpacked.
fn main() -> anyhow::Result<()> {
    let cache = bigame_core::graphics::optiscaler::cache_dir();
    let release = bigame_core::graphics::optiscaler::Release::recommended();
    let t = std::time::Instant::now();
    let c = bigame_core::graphics::optiscaler::fetch(&cache, &release)?;
    println!("{} {} in {} ({:?})", release.tag, release.sha256, c.dir.display(), t.elapsed());
    let mut names: Vec<String> = std::fs::read_dir(&c.dir)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    println!("{}", names.join("\n"));
    Ok(())
}
