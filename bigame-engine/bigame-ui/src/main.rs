//! BiGame-mode Libadwaita application entry point.

mod app;
pub mod i18n;
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

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,bigame_ui=debug,bigame_core=debug"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_names(false)
        .compact()
        .try_init();
}
