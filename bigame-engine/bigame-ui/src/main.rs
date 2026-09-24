//! BiGame-mode Libadwaita application entry point.

mod app;
mod game_watch;
pub mod i18n;
mod profile_offer;
pub mod settings;
mod style;
mod tray;
mod views;
mod widgets;
mod window;

fn main() -> libadwaita::glib::ExitCode {
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
