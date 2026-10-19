//! Big Game Mode Libadwaita application entry point.

mod app;
mod crash_watch;
mod game_info;
mod game_watch;
pub mod i18n;
mod profile_offer;
pub mod settings;
mod style;
mod theme;
mod tray;
mod views;
mod widgets;
mod window;

/// The options `bigame-ui` takes, for `--help`.
const USAGE: &str = "\
Usage: bigame-ui [OPTION]

Big Game Mode, BigLinux's game mode.

Options:
  --background            start in the tray, without opening the window
  --diagnostics           print the support report and exit
  --network               with --diagnostics: add the DNS measurements
  --restore-all-graphics  put back every game AI Graphics changed, and exit
  --version, -V           print the version and exit
  --help, -h              print this help and exit
";

/// What the command line asks for before anything else runs.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    /// Run (the window, or the tray with `--background`).
    Run,
    /// Print this and exit successfully.
    Print(String),
    /// An option this program does not take: say so and exit.
    Unknown(String),
}

/// Read the command line. A question (`--version`, `--help`) or an option
/// it does not know is answered here, before an instance already running is
/// reached: starting the application, even to fail on an option, would ask
/// an older running one to give way.
fn cli(args: &[String]) -> Cli {
    const KNOWN: [&str; 4] = [
        "--background",
        "--diagnostics",
        "--network",
        "--restore-all-graphics",
    ];
    for a in args {
        match a.as_str() {
            "--version" | "-V" => {
                return Cli::Print(format!("bigame-ui {}\n", env!("CARGO_PKG_VERSION")));
            }
            "--help" | "-h" => return Cli::Print(USAGE.to_owned()),
            _ => {}
        }
    }
    // GApplication's own options pass through to it.
    match args.iter().find(|a| {
        a.starts_with('-') && !KNOWN.contains(&a.as_str()) && !a.starts_with("--gapplication-")
    }) {
        Some(other) => Cli::Unknown(other.clone()),
        None => Cli::Run,
    }
}

fn main() -> libadwaita::glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli(&args) {
        Cli::Run => {}
        Cli::Print(text) => {
            print!("{text}");
            return libadwaita::glib::ExitCode::SUCCESS;
        }
        Cli::Unknown(option) => {
            eprint!("bigame-ui: unknown option {option}\n\n{USAGE}");
            return libadwaita::glib::ExitCode::from(2);
        }
    }
    // The support report the Diagnostics page shows, for a terminal or a
    // bug report: no window, no display needed.
    if args.iter().any(|a| a == "--diagnostics") {
        // --network adds the DNS measurements, which take a few seconds and
        // send queries, so they are asked for rather than assumed.
        let network = args.iter().any(|a| a == "--network");
        print!("{}", bigame_core::diagnostics::report(network));
        return libadwaita::glib::ExitCode::SUCCESS;
    }
    // Before any thread starts, so all of them inherit it.
    bigame_core::processes::yield_disk_to_games();
    bigame_core::processes::limit_malloc_arenas();
    // Every game AI Graphics changed, put back: before removing the package,
    // from a terminal (the package's removal runs as root and cannot reach
    // each user's games and backups).
    if args.iter().any(|a| a == "--restore-all-graphics") {
        let results = bigame_core::graphics::restore_all();
        if results.is_empty() {
            println!("No game has files from AI Graphics.");
        }
        let mut failed = false;
        for (game, result) in results {
            match result {
                Ok(_) => println!("restored: {game}"),
                Err(e) => {
                    failed = true;
                    println!("NOT restored: {game} — {e:#}");
                }
            }
        }
        return if failed {
            libadwaita::glib::ExitCode::FAILURE
        } else {
            libadwaita::glib::ExitCode::SUCCESS
        };
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
        // Before the journal layer: it records the event's fields that the
        // journal layer's record then sends as fields of their own.
        .with(sink.is_some().then_some(FieldCapture))
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

thread_local! {
    /// The structured fields of the event being formatted on this thread,
    /// recorded by [`FieldCapture`] and taken by the [`JournalRecord`] that
    /// sends it.
    static EVENT_FIELDS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Records, for each event, the fields named in
/// [`bigame_core::logs::STRUCTURED_FIELDS`], and its target as
/// `BIGAME_SUBSYSTEM`. The text of the record stays what the Logs page reads;
/// the fields let `journalctl BIGAME_GAME=…` select one game's records.
struct FieldCapture;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for FieldCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        struct Visit(Vec<(String, String)>);
        impl tracing::field::Visit for Visit {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.add(field, value.to_owned());
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.add(field, format!("{value:?}"));
            }
        }
        impl Visit {
            fn add(&mut self, field: &tracing::field::Field, value: String) {
                if let Some((_, name)) = bigame_core::logs::STRUCTURED_FIELDS
                    .iter()
                    .find(|(f, _)| *f == field.name())
                {
                    self.0.push(((*name).to_owned(), value));
                }
            }
        }
        let mut visit = Visit(vec![(
            "BIGAME_SUBSYSTEM".to_owned(),
            event.metadata().target().to_owned(),
        )]);
        event.record(&mut visit);
        EVENT_FIELDS.with(|f| *f.borrow_mut() = visit.0);
    }
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
        let fields = EVENT_FIELDS.with(|f| std::mem::take(&mut *f.borrow_mut()));
        if !text.is_empty() {
            // There is nowhere to report a failure to log.
            let _ = self
                .sink
                .send_with("bigame-ui", self.priority, text, &fields);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, cli};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_question_is_answered_before_anything_runs() {
        assert_eq!(
            cli(&args(&["--version"])),
            Cli::Print(format!("bigame-ui {}\n", env!("CARGO_PKG_VERSION")))
        );
        assert!(matches!(cli(&args(&["-h"])), Cli::Print(t) if t.contains("--background")));
        // Even beside an acting option: a question never acts.
        assert!(matches!(
            cli(&args(&["--restore-all-graphics", "--version"])),
            Cli::Print(_)
        ));
    }

    #[test]
    fn an_unknown_option_is_refused_and_the_known_ones_run() {
        assert_eq!(cli(&args(&["--versoin"])), Cli::Unknown("--versoin".into()));
        assert_eq!(cli(&args(&[])), Cli::Run);
        assert_eq!(cli(&args(&["--background"])), Cli::Run);
        assert_eq!(cli(&args(&["--diagnostics", "--network"])), Cli::Run);
        assert_eq!(cli(&args(&["--gapplication-service"])), Cli::Run);
    }
}
