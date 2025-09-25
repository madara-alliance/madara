use chrono::Utc;
use serde_json::{Map, Value};
use std::collections::HashMap;
use tracing::{Event, Level, Subscriber, field::{Visit, Field}};
use tracing_error::ErrorLayer;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::{format::Writer, FormatEvent, FormatFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, EnvFilter, Registry, Layer};
use tracing_subscriber::layer::Context;

const FIELDS_TO_SKIP: &[&str] = &[
    "subject_id",
    "correlation_id",
    "trace_id",
    "span_type"
];

#[derive(Debug, Clone)]
pub struct CustomSpanFields {
    pub filtered_display: String,
    pub raw_fields: HashMap<String, String>,
}

impl CustomSpanFields {
    fn new() -> Self {
        Self {
            filtered_display: String::new(),
            raw_fields: HashMap::new(),
        }
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
        Self {
            fields: CustomSpanFields::new(),
        }
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
        let existing_fields = span.extensions().get::<CustomSpanFields>().cloned()
            // .map(|f| f.clone())
            .unwrap_or_else(CustomSpanFields::new);

        let mut collector = SpanFieldCollector::new();
        collector.fields = existing_fields;
        values.record(&mut collector);

        span.extensions_mut().insert(collector.fields);
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
        let msg_color = "\x1b[97m"; // Bright White
        let fixed_field_color = "\x1b[92m"; // Bright Green
        let reset = "\x1b[0m";
        let function_color = "\x1b[35m"; // Magenta

        // Format line
        write!(writer, "{}{}{} ", ts_color, now, reset)?;
        write!(writer, "{}{:<5}{} ", level_color, *meta.level(), reset)?;

        write!(
            writer,
            "{}[{}:{}]{} ",
            function_color,
            meta.module_path().unwrap_or("NaN"),
            meta.line().unwrap_or(0),
            reset
        )?;

        // Add filtered span fields if available
        if let Some(span) = ctx.lookup_current() {
            if let Some(custom_fields) = span.extensions().get::<CustomSpanFields>() {
                if !custom_fields.filtered_display.is_empty() {
                    write!(writer, "{}[{}]{} ", fixed_field_color, custom_fields.filtered_display, reset)?;
                }
            }
        }

        let mut visitor = FieldExtractor::default();
        event.record(&mut visitor);

        // Write the main message
        write!(writer, "{}{}{}", msg_color, visitor.message, reset)?;

        // Write fields separately with proper formatting
        if !visitor.fields.is_empty() || !visitor.meta.is_empty() {
            write!(writer, " (")?;
            let mut first = true;
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
