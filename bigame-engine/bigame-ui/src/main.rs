//! BiGame-mode Libadwaita application entry point.

mod app;
mod game_watch;
mod gpu_reading;
pub mod i18n;
mod profile_offer;
pub mod settings;
mod style;
mod tray;
mod views;
mod widgets;
mod window;

fn main() -> libadwaita::glib::ExitCode {
    // The support report the Diagnostics page shows, for a terminal or a
    // bug report: no window, no display needed.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--diagnostics") {
        // --network adds the DNS measurements, which take a few seconds and
        // send queries, so they are asked for rather than assumed.
        let network = args.iter().any(|a| a == "--network");
        print!("{}", bigame_core::diagnostics::report(network));
        return libadwaita::glib::ExitCode::SUCCESS;
    }
    init_tracing();
    i18n::init();
    app::run()
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Started from the menu, the output is the journal: it timestamps each
    // line itself, and colour codes would show up as `[2m…[0m`.
    let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_names(false)
        .with_ansi(terminal)
        .compact();
    let _ = if terminal {
        builder.try_init()
    } else {
        builder.without_time().try_init()
    };
}
