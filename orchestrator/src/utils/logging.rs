use chrono::Utc;
use color_eyre::config::{HookBuilder, Theme};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt as stdfmt;
use tracing::{
    field::{Field, Visit},
    Event, Level, Subscriber,
};
use tracing_error::ErrorLayer;
use tracing_subscriber::field::{MakeVisitor, VisitFmt, VisitOutput};
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
        // Timestamp (14 chars) | Level (5 chars) | Queue (24 chars) | Message and fields
        write!(writer, "{} ", now)?;
        write!(writer, "| ")?;
        write!(writer, "{:<5} ", *meta.level())?;
        write!(writer, "| ")?;
        write!(writer, "{:<20} ", queue.0)?;
        write!(writer, "| ")?;
        write!(writer, "{:<14} ", queue.1)?;
        write!(writer, "| ")?;

        // Add service/package as prefix to message
        let service = extract_service_name(meta.target());

        // Write the service prefix and main message
        if service != "-" {
            write!(writer, "{}: {}", service, visitor.message)?;
        } else {
            write!(writer, "{}", visitor.message)?;
        }

        // Write fields separately with proper formatting (excluding queue which is already shown)
        if !visitor.meta.is_empty() || !visitor.fields.is_empty() {
            write!(writer, " (")?;
            let mut first = true;

            // Write meta fields (id, etc.) but skip 'q' since it's in the queue column
            if !visitor.meta.is_empty() {
                write!(writer, "{}", visitor.meta)?;
                first = false;
            }
            if !visitor.fields.is_empty() {
                if !first {
                    write!(writer, ", ")?;
                }
                write!(writer, "{}", visitor.fields)?;
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
        if field.name() == "message" {
            self.message = format!("{:?}", value).trim_matches('"').to_string();
        } else if field.name() == "q" {
            // Skip 'q' field - it's displayed in the queue column
        } else {
            let formatted_value = format!("{:?}", value).trim_matches('"').to_string();
            let formatted_field = format!("{}={}", field.name(), formatted_value);

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

/// A field formatter that produces plain text without ANSI color codes.
/// Used for ErrorLayer to ensure spantraces don't contain escape sequences.
#[derive(Debug, Default)]
pub struct PlainFields;

impl PlainFields {
    pub fn new() -> Self {
        Self
    }
}

/// Visitor for PlainFields that formats fields without ANSI codes
pub struct PlainVisitor<'a> {
    writer: Writer<'a>,
    is_empty: bool,
    result: stdfmt::Result,
}

impl<'a> PlainVisitor<'a> {
    fn new(writer: Writer<'a>, is_empty: bool) -> Self {
        Self { writer, is_empty, result: Ok(()) }
    }

    fn maybe_pad(&mut self) {
        if self.is_empty {
            self.is_empty = false;
        } else {
            self.result = write!(self.writer, " ");
        }
    }
}

impl<'a> Visit for PlainVisitor<'a> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if self.result.is_err() {
            return;
        }
        if field.name() == "message" {
            self.record_debug(field, &format_args!("{}", value))
        } else {
            self.record_debug(field, &value)
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn stdfmt::Debug) {
        if self.result.is_err() {
            return;
        }
        self.maybe_pad();
        self.result = match field.name() {
            "message" => write!(self.writer, "{:?}", value),
            name if name.starts_with("r#") => write!(self.writer, "{}={:?}", &name[2..], value),
            name => write!(self.writer, "{}={:?}", name, value),
        };
    }
}

impl<'a> VisitOutput<stdfmt::Result> for PlainVisitor<'a> {
    fn finish(self) -> stdfmt::Result {
        self.result
    }
}

impl<'a> VisitFmt for PlainVisitor<'a> {
    fn writer(&mut self) -> &mut dyn stdfmt::Write {
        &mut self.writer
    }
}

impl<'a> MakeVisitor<Writer<'a>> for PlainFields {
    type Visitor = PlainVisitor<'a>;

    fn make_visitor(&self, target: Writer<'a>) -> Self::Visitor {
        PlainVisitor::new(target, true)
    }
}

/// Initialize the tracing subscriber with
/// - PrettyFormatter for console readability (when LOG_FORMAT != "json")
/// - JsonEventFormatter for json logging (when LOG_FORMAT = "json")
///
/// This will also install color_eyre to handle the panic in the application
pub fn init_logging() {
    // Install color_eyre with colors disabled for clean log files
    // Theme::new() creates a blank theme with no ANSI color codes (all Style::default())
    HookBuilder::blank()
        .capture_span_trace_by_default(true)
        .theme(Theme::new())
        .install()
        .expect("Unable to install color_eyre");

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
            .with(ErrorLayer::new(PlainFields::new()));
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
            .with(ErrorLayer::new(PlainFields::new()));
        tracing::subscriber::set_global_default(subscriber).expect("Failed to set global default subscriber");
    }
}

/// Extract service/package name from the tracing target
///
/// Maps crate names to short display names for service identification in logs.
///
/// # Classification
///
/// - **Orchestrator crates** → Specific service names (ATLANTIC, UTILS, GPS_FACT_CHK, etc.)
/// - **Generic orchestrator** → "-" (no specific service, core orchestrator code)
/// - **Third-party dependencies** → "EXTERNAL" (logs from Rust ecosystem crates)
///
/// Note: "EXTERNAL" refers to third-party Rust crates (tokio, hyper, reqwest, etc.),
/// NOT external services like Atlantic. Atlantic logs are tagged as "ATLANTIC".
///
/// # Examples
///
/// ```ignore
/// extract_service_name("orchestrator_atlantic_service::client") → "ATLANTIC"
/// extract_service_name("orchestrator_utils::logging")           → "UTILS"
/// extract_service_name("orchestrator::core::config")            → "-"
/// extract_service_name("tokio::runtime")                        → "EXTERNAL"
/// extract_service_name("hyper::client")                         → "EXTERNAL"
/// ```
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
