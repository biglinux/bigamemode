//! Print the detected game library with executables and cover art.
fn main() {
    for g in bigame_core::games::detect_all() {
        println!("{} [{}] appid={:?}", g.name, g.source.label(), g.app_id);
        println!(
            "   profile key : {}  (real executable: {})",
            g.profile_key(),
            g.has_real_executable()
        );
        println!("   candidates  : {:?}", g.executables);
        println!("   cover       : {:?}", g.cover);
    }
}
