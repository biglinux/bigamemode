//! Who controls falcond's service, and handing it back or taking it again.
//!
//! `cargo run -p bigame-core --example falcond_control -- <status|release|take>`
//!
//! `release` and `take` go through the privileged helper (Polkit). With
//! `BIGAME_WAIT=<seconds>` the program prints its pid and waits before asking,
//! so a text Polkit agent can be attached to it.

fn main() -> anyhow::Result<()> {
    use bigame_core::turbo;
    let what = std::env::args().nth(1).unwrap_or_else(|| "status".into());
    if what != "status"
        && let Some(secs) = std::env::var("BIGAME_WAIT")
            .ok()
            .and_then(|s| s.parse().ok())
    {
        println!("pid {}", std::process::id());
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
    match what.as_str() {
        "release" => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let released = rt.block_on(async {
                let proxy = bigame_core::dbus_client::daemon_proxy().await?;
                anyhow::Ok(proxy.release_game_backend().await?)
            })?;
            println!("released: {released}");
        }
        "take" => println!("took: {:?}", turbo::take_back_blocking()?),
        _ => {}
    }
    println!("control: {:?}", turbo::control_blocking()?);
    Ok(())
}
