use chrono::Utc;
use tracing::{Event, Level, Subscriber};
use tracing_log::LogTracer;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::{format::Writer, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

// Pretty formatter is formatted for console readability
struct PrettyFormatter;

impl<S, N> FormatEvent<S, N> for PrettyFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let meta = event.metadata();
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);

        // Colors
        let ts_color = "\x1b[96m"; // Bright Cyan
        let level_color = match *meta.level() {
            Level::TRACE => "\x1b[90m",
            Level::DEBUG => "\x1b[34m",
            Level::INFO => "\x1b[32m",
            Level::WARN => "\x1b[33m",
            Level::ERROR => "\x1b[31m",
        };
        let thread_color = "\x1b[94m"; // Bright Blue
        let id_color = "\x1b[35m"; // Magenta
        let file_color = "\x1b[90m"; // Bright Black
        let msg_color = "\x1b[97m"; // Bright White
        let reset = "\x1b[0m";

        // Format line
        write!(writer, "{}{}{} ", ts_color, now, reset)?;
        write!(writer, "{}{:<5}{} ", level_color, *meta.level(), reset)?;

        if let Some(name) = std::thread::current().name() {
            write!(writer, "{}{}{} ", thread_color, name, reset)?;
        }

        write!(writer, "{}{:?}{} ", id_color, std::thread::current().id(), reset)?;

        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            write!(writer, "{}{}:{}:{} ", file_color, file, line, reset)?;
        }

        let mut visitor = FieldExtractor::default();
        event.record(&mut visitor);

        if !visitor.meta.is_empty() {
            write!(writer, " ({}{}{})", msg_color, visitor.meta, reset)?;
        }

        write!(writer, "{}{}{}", msg_color, visitor.message, reset)?;
        if !visitor.fields.is_empty() {
            write!(writer, " ({}{}{})", msg_color, visitor.fields, reset)?;
        }

        writeln!(writer)
    }
}

// Visitor to extract message and format fields
#[derive(Default)]
struct FieldExtractor {
    message: String,
    fields: String,
    meta: String,
}

impl tracing::field::Visit for FieldExtractor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else if field.name() == "yak" {
            if !self.meta.is_empty() {
                self.meta.push_str(", ");
            }
            self.meta.push_str(&format!("{}={:?}", field.name(), value));
        } else {
            if !self.fields.is_empty() {
                self.fields.push_str(", ");
            }
            self.fields.push_str(&format!("{}={:?}", field.name(), value));
        }
    }
}

/// Initialize the tracing subscriber with
/// - PrettyFormatter for console readability
/// - JsonFormatter for json logging (for integration with orchestrator)
///
/// This will also install color_eyre to handle the panic in the application
pub fn init_logging() {
    color_eyre::install().expect("Unable to install color_eyre");
    LogTracer::init().expect("Failed to set logger");
    let subscriber = fmt::Subscriber::builder()
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .event_format(PrettyFormatter)
        .finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();
}
