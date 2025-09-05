use chrono::Utc;
use tracing::{Event, Level, Subscriber};
use tracing_error::ErrorLayer;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::{format::Writer, FormatEvent, FormatFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, EnvFilter, Registry};

// Pretty formatter is formatted for console readability
struct PrettyFormatter;

impl<S, N> FormatEvent<S, N> for PrettyFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let meta = event.metadata();
        let now = Utc::now().format("%m-%d %H:%M:%S").to_string();

        // Colors
        let ts_color = "\x1b[96m"; // Bright Cyan
        let level_color = match *meta.level() {
            Level::TRACE => "\x1b[90m",
            Level::DEBUG => "\x1b[34m",
            Level::INFO => "\x1b[32m",
            Level::WARN => "\x1b[33m",
            Level::ERROR => "\x1b[31m",
        };
        let file_color = "\x1b[90m"; // Bright Black
        let msg_color = "\x1b[97m"; // Bright White
        let fixed_field_color = "\x1b[92m"; // Bright Green
        let reset = "\x1b[0m";
        let function_color = "\x1b[35m"; // Magenta

        // Format line
        write!(writer, "{}{}{} ", ts_color, now, reset)?;
        write!(writer, "{}{:<5}{} ", level_color, *meta.level(), reset)?;

        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            let file_name = file.split('/').next_back().unwrap_or(file);
            let module_path = meta.module_path().unwrap_or("");
            let last_module = module_path.split("::").last().unwrap_or(module_path);

            let display_name =
                if file_name == "mod.rs" { format!("{}/{}", last_module, file_name) } else { file_name.to_string() };

            write!(writer, "{}{:<20}:{:<4} {}", file_color, display_name, line, reset)?;
        }

        write!(writer, "{}[{}]{} ", function_color, meta.name(), reset)?;

        // Add queue_type from span if available
        if let Some(span) = ctx.lookup_current() {
            if let Some(fields) = span.extensions().get::<fmt::FormattedFields<N>>() {
                // Apply color to the entire field string
                write!(writer, "{}[{}]{} ", fixed_field_color, fields, reset)?;
            }
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
        let fixed_field_color = "\x1b[92m"; // Bright Green
        let reset = "\x1b[0m";

        if field.name() == "message" {
            self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else {
            let formatted_value = format!("{:?}", value).trim_matches('"').to_string();
            let formatted_field = format!(
                "{}{}{}={}{}{}",
                fixed_field_color,
                field.name(),
                reset,
                fixed_field_color,
                formatted_value,
                reset
            );

            // Prioritize q and id fields
            if field.name() == "q" || field.name() == "id" {
                if !self.meta.is_empty() {
                    self.meta.push_str(", ");
                }
                self.meta.push_str(&formatted_field);
            } else {
                if !self.fields.is_empty() {
                    self.fields.push_str(", ");
                }
                self.fields.push_str(&formatted_field);
            }
        }
    }
}

/// Initialize the tracing subscriber with
/// - PrettyFormatter for console readability (when LOG_FORMAT != "json")
/// - JsonFormatter for json logging (when LOG_FORMAT = "json")
///
/// This will also install color_eyre to handle the panic in the application
pub fn init_logging() {
    color_eyre::install().expect("Unable to install color_eyre");

    // Read from `RUST_LOG` environment variable, with fallback to default
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // Fallback if RUST_LOG is not set or invalid
        EnvFilter::builder()
            .with_default_directive(Level::INFO.into())
            .parse("orchestrator=info")
            .expect("Invalid filter directive and Logger control")
    });

    // Check LOG_FORMAT environment variable
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_else(|_| "pretty".to_string());

    if log_format == "json" {
        // JSON format for Loki/Grafana integration
        let fmt_layer = fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true);

        let subscriber = Registry::default().with(env_filter).with(fmt_layer).with(ErrorLayer::default());
        tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");
    } else {
        // Pretty format for console readability
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .event_format(PrettyFormatter);

        let subscriber = Registry::default().with(env_filter).with(fmt_layer).with(ErrorLayer::default());
        tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");
    }
}
