use chrono::Utc;
use serde_json::{Map, Value};
use std::collections::HashMap;
use tracing::{
    field::{Field, Visit},
    Event, Level, Subscriber,
};
use tracing_error::ErrorLayer;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::{format::Writer, FormatEvent, FormatFields};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, EnvFilter, Layer, Registry};

const FIELDS_TO_SKIP: &[&str] = &["subject_id", "correlation_id", "trace_id", "span_type"];

#[derive(Debug, Clone)]
pub struct CustomSpanFields {
    pub filtered_display: String,
    pub raw_fields: HashMap<String, String>,
}

impl CustomSpanFields {
    fn new() -> Self {
        Self { filtered_display: String::new(), raw_fields: HashMap::new() }
    }

    fn add_field(&mut self, name: &str, value: String) {
        self.raw_fields.insert(name.to_string(), value.clone());

        if !FIELDS_TO_SKIP.contains(&name) {
            if !self.filtered_display.is_empty() {
                self.filtered_display.push_str(", ");
            }
            self.filtered_display.push_str(&format!("{}={}", name, value));
        }
    }
}

struct SpanFieldCollector {
    fields: CustomSpanFields,
}

impl SpanFieldCollector {
    fn new() -> Self {
        Self { fields: CustomSpanFields::new() }
    }
}

impl Visit for SpanFieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let formatted_value = format!("{:?}", value).trim_matches('"').to_string();
        self.fields.add_field(field.name(), formatted_value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.add_field(field.name(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.add_field(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.add_field(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.add_field(field.name(), value.to_string());
    }
}

pub struct FieldCollectorLayer;

impl<S> Layer<S> for FieldCollectorLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");

        let mut collector = SpanFieldCollector::new();
        attrs.record(&mut collector);

        // Store the collected fields in the span's extensions
        span.extensions_mut().insert(collector.fields);
    }

    fn on_record(&self, id: &tracing::span::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("Span not found, this is a bug");

        // Get existing fields or create new
        let mut extensions = span.extensions_mut();
        let existing_fields = extensions.remove::<CustomSpanFields>().unwrap_or_else(CustomSpanFields::new);

        let mut collector = SpanFieldCollector::new();
        collector.fields = existing_fields;
        values.record(&mut collector);

        // Reinsert the updated fields (extensions was already removed above)
        extensions.insert(collector.fields);
    }
}

// Pretty formatter is formatted for console readability
pub struct PrettyFormatter;

impl<S, N> FormatEvent<S, N> for PrettyFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let meta = event.metadata();
        let now = Utc::now().format("%y-%m-%d %H:%M:%S").to_string();

        // Colors
        let ts_color = "\x1b[96m"; // Bright Cyan
        let level_color = match *meta.level() {
            Level::TRACE => "\x1b[90m",
            Level::DEBUG => "\x1b[34m",
            Level::INFO => "\x1b[32m",
            Level::WARN => "\x1b[33m",
            Level::ERROR => "\x1b[31m",
        };
        let msg_color = "\x1b[97m"; // Bright White
        let queue_color = "\x1b[92m"; // Bright Green
        let reset = "\x1b[0m";
        let dim_color = "\x1b[90m"; // Dim gray for separators

        // Extract queue from span fields
        let mut queue = (String::from("-"), String::from("-"));
        if let Some(span) = ctx.lookup_current() {
            if let Some(custom_fields) = span.extensions().get::<CustomSpanFields>() {
                if let Some(q_value) = custom_fields.raw_fields.get("q") {
                    queue = queue_type_to_parts(q_value);
                }
            }
        }

        let mut visitor = FieldExtractor::default();
        event.record(&mut visitor);

        // Table-like format with fixed widths:
        // Timestamp (14 chars) | Level (5 chars) | Queue (24 chars) | Service (12 chars) | Message and fields
        write!(writer, "{}{}{} ", ts_color, now, reset)?;
        write!(writer, "{}|{} ", dim_color, reset)?;
        write!(writer, "{}{:<5}{} ", level_color, *meta.level(), reset)?;
        write!(writer, "{}|{} ", dim_color, reset)?;
        write!(writer, "{}{:<20}{} ", queue_color, queue.0, reset)?;
        write!(writer, "{}|{} ", dim_color, reset)?;
        write!(writer, "{}{:<14}{} ", queue_color, queue.1, reset)?;
        write!(writer, "{}|{} ", dim_color, reset)?;

        // Add service/package column
        let service = extract_service_name(meta.target());
        write!(writer, "{}{:<12}{} ", queue_color, service, reset)?;
        write!(writer, "{}|{} ", dim_color, reset)?;

        // Write the main message
        write!(writer, "{}{}{}", msg_color, visitor.message, reset)?;

        // Write fields separately with proper formatting (excluding queue which is already shown)
        if !visitor.meta.is_empty() || !visitor.fields.is_empty() {
            write!(writer, " (")?;
            let mut first = true;

            // Write meta fields (id, etc.) but skip 'q' since it's in the queue column
            if !visitor.meta.is_empty() {
                write!(writer, "{}{}{}", msg_color, visitor.meta, reset)?;
                first = false;
            }
            if !visitor.fields.is_empty() {
                if !first {
                    write!(writer, ", ")?;
                }
                write!(writer, "{}{}{}", msg_color, visitor.fields, reset)?;
            }
            write!(writer, ")")?;
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
        let fixed_field_color = "\x1b[90m"; // Dark Grey
        let reset = "\x1b[0m";

        if field.name() == "message" {
            self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else if field.name() == "q" {
            // Skip 'q' field - it's displayed in the queue column
        } else {
            let formatted_value = format!("{:?}", value).trim_matches('"').to_string();
            let formatted_field = format!("{}{}={}{}", fixed_field_color, field.name(), formatted_value, reset);

            // Prioritize id field
            if field.name() == "id" {
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

// JSON formatter for structured logs suitable for Loki/Grafana
pub struct JsonEventFormatter;

#[derive(Default)]
struct JsonFieldVisitor {
    message: Option<String>,
    fields: Map<String, Value>,
}

impl tracing::field::Visit for JsonFieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        // Use Debug to avoid requiring Serialize everywhere, then to_string
        let v = format!("{:?}", value);
        let v = v.trim_matches('"').to_string();
        if field.name() == "message" {
            self.message = Some(v);
        } else {
            self.fields.insert(field.name().to_string(), Value::String(v));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), Value::from(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }
}

impl<S, N> FormatEvent<S, N> for JsonEventFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let meta = event.metadata();
        let ts = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // Extract event message and fields
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);

        // Base object
        let mut root = Map::new();
        root.insert("timestamp".to_string(), Value::String(ts));
        root.insert("level".to_string(), Value::String(meta.level().to_string()));
        root.insert("target".to_string(), Value::String(meta.target().to_string()));
        root.insert("service".to_string(), Value::String(extract_service_name(meta.target()).to_string()));
        if let Some(file) = meta.file() {
            root.insert("filename".to_string(), Value::String(file.to_string()));
        }
        if let Some(line) = meta.line() {
            root.insert("line_number".to_string(), Value::from(line));
        }

        // message at root level (clean, just the message text)
        if let Some(message) = visitor.message.take() {
            root.insert("message".to_string(), Value::String(message));
        }

        // Collect all fields (both event fields and span fields)
        let mut all_fields = visitor.fields;

        // Extract span fields and merge them into the main fields object
        if let Some(span) = ctx.lookup_current() {
            // Add span name as a field
            let span_name = span.metadata().name().to_string();
            all_fields.insert("span_name".to_string(), Value::String(span_name));

            if let Some(custom_fields) = span.extensions().get::<CustomSpanFields>() {
                for (key, value) in &custom_fields.raw_fields {
                    all_fields.insert(key.clone(), Value::String(value.clone()));
                }
            }
        }

        // Insert all fields at once under "fields" object
        if !all_fields.is_empty() {
            root.insert("fields".to_string(), Value::Object(all_fields));
        }

        // Write one-line JSON
        let line = serde_json::to_string(&Value::Object(root)).map_err(|_| std::fmt::Error)?;
        writeln!(writer, "{}", line)
    }
}

/// Initialize the tracing subscriber with
/// - PrettyFormatter for console readability (when LOG_FORMAT != "json")
/// - JsonEventFormatter for json logging (when LOG_FORMAT = "json")
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
        // JSON format (one JSON object per line)
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .event_format(JsonEventFormatter);

        let field_collector_layer = FieldCollectorLayer;
        let subscriber = Registry::default()
            .with(env_filter)
            .with(field_collector_layer)
            .with(fmt_layer)
            .with(ErrorLayer::default());
        tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");
    } else {
        // Pretty format for console readability
        let fmt_layer = fmt::layer()
            .with_target(true)
            .with_thread_ids(false)
            .with_file(true)
            .with_line_number(true)
            .event_format(PrettyFormatter);

        let field_collector_layer = FieldCollectorLayer;
        let subscriber = Registry::default()
            .with(env_filter)
            .with(field_collector_layer)
            .with(fmt_layer)
            .with(ErrorLayer::default());
        tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");
    }
}

/// Extract service/package name from the tracing target
/// Maps crate names to short display names for the service column
fn extract_service_name(target: &str) -> &'static str {
    if target.starts_with("orchestrator_atlantic_service") {
        "ATLANTIC"
    } else if target.starts_with("orchestrator_gps_fact_checker") {
        "GPS_FACT_CHK"
    } else if target.starts_with("orchestrator_prover_client_interface") {
        "PROVER_IFACE"
    } else if target.starts_with("orchestrator_utils") {
        "UTILS"
    } else if target.starts_with("orchestrator") {
        "-"
    } else {
        "EXTERNAL"
    }
}

/// Function used by the logger to display queue with pretty formatting
pub fn queue_type_to_parts(queue_type: &str) -> (String, String) {
    // Special cases
    if queue_type == "job_handle_failure" {
        return ("JOB_HANDLE_FAILURE".into(), "-".into());
    }
    if queue_type == "worker_trigger" {
        return ("WORKER_TRIGGER".into(), "-".into());
    }
    // Split into prefix, suffix
    let (prefix, suffix) = match queue_type.rsplit_once('_') {
        Some(v) => v,
        None => return ("-".into(), "-".into()),
    };
    // Remove "_job" or "_JOB" if present
    let prefix = prefix.trim_end_matches("_job").trim_end_matches("_JOB");
    (prefix.to_uppercase(), suffix.to_uppercase())
}
