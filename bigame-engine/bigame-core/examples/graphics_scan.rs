//! Scan a game folder the way AI Graphics does, and print what it found.
//!
//! Usage: `graphics_scan <install-dir> [process-name]`
fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: graphics_scan <install-dir> [process-name]");
        std::process::exit(2);
    };
    let hint = args.next();
    let t = std::time::Instant::now();
    let s = bigame_core::graphics::scan::scan(std::path::Path::new(&root), hint.as_deref());
    println!("scanned in {:?} (truncated: {})", t.elapsed(), s.truncated);
    println!("executable: {:?}  engine: {:?}", s.executable, s.engine);
    if let Some(pe) = &s.executable_pe {
        let gfx: Vec<&String> = pe
            .imports
            .iter()
            .chain(&pe.delay_imports)
            .filter(|d| {
                [
                    "d3d", "dxgi", "vulkan", "opengl", "sl.", "xess", "ffx", "nvngx",
                ]
                .iter()
                .any(|k| d.contains(k))
            })
            .collect();
        println!("machine: {:?}  graphics links: {gfx:?}", pe.machine);
    }
    for c in &s.components {
        println!(
            "component {:?}  {}  {:?}",
            c.kind,
            c.path.display(),
            c.version
        );
    }
    for p in &s.proxies {
        println!("proxy {}  {:?}  {:?}", p.slot, p.owner, p.version);
    }
    for a in &s.anti_cheat {
        println!("anti-cheat {}  ({})", a.name, a.evidence.display());
    }
}
