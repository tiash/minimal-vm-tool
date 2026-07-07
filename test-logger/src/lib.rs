//! A tiny test-only logger initializer that guards against double-init.
//!
//! Each integration test binary calls `test_logger::init()` at the top
//! of every test. The first call in the process installs the global
//! `stderrlog` logger, wrapped in a filter that drops noisy messages
//! from the `serial_test` crate; subsequent calls are no-ops, avoiding
//! `SetLoggerError` panics when tests run in parallel within the same
//! binary.
//!
//! The log level is read once from the `LOG_LEVEL` environment variable
//! (parsed as a `log::LevelFilter`: `off`, `error`, `warn`, `info`,
//! `debug`, `trace`). When unset or invalid it defaults to `Error` to
//! keep test output quiet.

static INIT: std::sync::Once = std::sync::Once::new();

/// Initialize the test logger, filtering out messages from the
/// `serial_test` crate.
///
/// The log level is determined by the `LOG_LEVEL` environment variable
/// (parsed as a `log::LevelFilter`, e.g. `error`, `warn`, `info`,
/// `debug`, `trace`, `off`). When unset or invalid, it defaults to
/// `Error` to keep test output quiet.
pub fn init() {
    INIT.call_once(|| {
        let level = std::env::var("LOG_LEVEL")
            .ok()
            .and_then(|s| s.parse::<log::LevelFilter>().ok())
            .unwrap_or(log::LevelFilter::Error);

        let mut inner = stderrlog::new();
        inner
            .verbosity(level)
            .timestamp(stderrlog::Timestamp::Millisecond)
            .color(stderrlog::ColorChoice::Auto)
            .show_module_names(true);

        log::set_boxed_logger(Box::new(FilteringLogger { inner }))
            .expect("Failed to initialize logger");
        log::set_max_level(level);
    });
}

struct FilteringLogger {
    inner: stderrlog::StdErrLog,
}

impl log::Log for FilteringLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if record.target().starts_with("serial_test") {
            return;
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}
