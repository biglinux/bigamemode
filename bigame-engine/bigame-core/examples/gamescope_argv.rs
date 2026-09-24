//! Print the argv the builder produces for the gamescope actually installed.
fn main() {
    use bigame_core::gamescope::{Config, Filter, FrameLimit};
    let caps = bigame_core::capabilities::Capabilities::detect()
        .gamescope
        .expect("gamescope must be installed for this example");
    println!("gamescope version {:?}", caps.version);

    let cfg = Config {
        render_width: 2560,
        render_height: 1080,
        output_width: 3440,
        output_height: 1440,
        filter: Filter::Fsr,
        sharpness: 3,
        frame_limit: FrameLimit::NestedRefresh(75),
        mangoapp: false,
        adaptive_sync: true,
        hdr: true,
        fullscreen: true,
    };
    let built = cfg.to_args(&caps);
    println!("ARGS {}", built.args.join(" "));
    for u in &built.unsupported {
        println!("UNSUPPORTED --{} ({})", u.flag, u.effect);
    }
}
