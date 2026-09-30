//! Big Game Mode Libadwaita application entry point.

mod app;
mod game_watch;
mod gpu_reading;
pub mod i18n;
mod profile_offer;
pub mod settings;
mod style;
mod theme;
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

/// Where Big Game Mode's own records go.
///
/// To the journal, directly ([`bigame_core::logs::JournalSink`]), however the
/// application was started: a menu entry some desktops launch as a systemd
/// service, a scope (most desktops, a file manager), the login autostart, a
/// terminal. Its standard output reaches the journal only in the first case,
/// and the Logs page and support reports read the journal.
///
/// Also to standard output when that is a terminal (coloured, timed) or a
/// file or pipe someone is capturing — but not when it already is the
/// journal, where every line would be recorded twice. Without a journal the
/// output is what it always was.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let sink = bigame_core::logs::JournalSink::open().map(std::sync::Arc::new);

    // The journal timestamps each record itself, and colour codes would show
    // up as `[2m…[0m`.
    let journal = sink.clone().map(|sink| {
        tracing_subscriber::fmt::layer()
            .with_writer(JournalWriter { sink })
            .with_target(true)
            .with_ansi(false)
            .without_time()
            .compact()
    });
    let echo = terminal || sink.is_none() || !bigame_core::logs::stdout_is_journal();
    let coloured = (echo && terminal).then(|| {
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_ansi(true)
            .compact()
    });
    let plain = (echo && !terminal).then(|| {
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_ansi(false)
            .without_time()
            .compact()
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(journal)
        .with(coloured)
        .with(plain)
        .try_init();

    if sink.is_some() {
        route_glib_to_journal();
    }
}

/// The toolkit's own warnings go to the journal too. `GLib` does that by
/// itself only when standard error already is the journal; otherwise it
/// prints them there and, started from a menu, they are lost. Informational
/// and debug messages keep the default handling, which drops them unless
/// asked for with `G_MESSAGES_DEBUG`.
fn route_glib_to_journal() {
    use libadwaita::glib;
    glib::log_set_writer_func(|level, fields| {
        let notable = matches!(
            level,
            glib::LogLevel::Error
                | glib::LogLevel::Critical
                | glib::LogLevel::Warning
                | glib::LogLevel::Message
        );
        if notable && glib::log_writer_journald(level, fields) == glib::LogWriterOutput::Handled {
            return glib::LogWriterOutput::Handled;
        }
        glib::log_writer_default(level, fields)
    });
}

/// Makes one [`JournalRecord`] per event, at the event's priority.
struct JournalWriter {
    sink: std::sync::Arc<bigame_core::logs::JournalSink>,
}

impl JournalWriter {
    fn record(&self, priority: u8) -> JournalRecord {
        JournalRecord {
            sink: std::sync::Arc::clone(&self.sink),
            priority,
            line: Vec::new(),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for JournalWriter {
    type Writer = JournalRecord;

    fn make_writer(&'a self) -> Self::Writer {
        self.record(6)
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        // syslog priorities: 3 error, 4 warning, 6 info, 7 debug.
        self.record(match *meta.level() {
            tracing::Level::ERROR => 3,
            tracing::Level::WARN => 4,
            tracing::Level::INFO => 6,
            _ => 7,
        })
    }
}

/// One formatted event, sent as one journal record when the formatter is
/// done with it.
struct JournalRecord {
    sink: std::sync::Arc<bigame_core::logs::JournalSink>,
    priority: u8,
    line: Vec<u8>,
}

impl std::io::Write for JournalRecord {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.line.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for JournalRecord {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.line);
        let text = text.trim_end();
        if !text.is_empty() {
            // There is nowhere to report a failure to log.
            let _ = self.sink.send("bigame-ui", self.priority, text);
        }
    }
}
