// Force a video config save round-trip to regenerate environment.d snippet.
fn main() {
    let cfg = bigame_core::video_config::load();
    bigame_core::video_config::save(&cfg).expect("save video config");
    println!(
        "video config re-saved; env file should be at ~/.config/environment.d/bigame-mode.conf"
    );
}
