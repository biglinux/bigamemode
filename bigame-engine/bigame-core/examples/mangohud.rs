//! Set a game's `MangoHud` mode as the Profiles page does.
//!
//! Usage: `mangohud <process> off|on|forced`
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(process), Some(mode)) = (args.first(), args.get(1)) else {
        anyhow::bail!("usage: mangohud <process> off|on|forced");
    };
    let mode = match mode.as_str() {
        "on" => bigame_core::mangohud::Mode::On,
        "forced" => bigame_core::mangohud::Mode::Forced,
        _ => bigame_core::mangohud::Mode::Off,
    };
    println!("{:?}", bigame_core::mangohud::apply(process, mode)?);
    println!("saved: {:?}", bigame_core::mangohud::mode_for(process));
    Ok(())
}
