use console::{Style, StyledObject};
use std::{
    fmt,
    time::{Duration, SystemTime},
};
use time::{format_description, OffsetDateTime, UtcOffset};
use tracing::{field::Visit, Level, Subscriber};
use tracing_core::Field;
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

pub fn display_fn<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result>(f: F) -> impl fmt::Display {
    DisplayFromFn(f)
}

struct DisplayFromFn<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result>(F);

impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for DisplayFromFn<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(f)
    }
}

struct RpcCallEvent<'a> {
    pub method: &'a str,
    pub status: i64,
    pub res_len: u64,
    pub response_time: u128,
}

#[derive(Default)]
struct RpcCallEventVisitor {
    method: String,
    status: Option<i64>,
    res_len: Option<u64>,
    response_time: Option<u128>,
}

impl Visit for RpcCallEventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "method" {
            self.method.clear();
            self.method.push_str(value);
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if field.name() == "status" {
            self.status = Some(value)
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "res_len" {
            self.res_len = Some(value)
        }
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        if field.name() == "response_time" {
            self.response_time = Some(value)
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn fmt::Debug) {
        // ignored
    }
}

impl RpcCallEventVisitor {
    fn _clear(&mut self) {
        self.method.clear();
        self.status = None;
        self.res_len = None;
        self.response_time = None;
    }

    pub fn get(&self) -> Option<RpcCallEvent<'_>> {
        if self.method.is_empty() {
            return None;
        }
        Some(RpcCallEvent {
            method: &self.method,
            status: self.status?,
            res_len: self.res_len?,
            response_time: self.response_time?,
        })
    }
}

#[derive(Default)]
struct CairoNativeEventVisitor {
    message: String,
    class_hash: Option<String>,
    elapsed: Option<String>,
    elapsed_ms: Option<u64>,
    conversion_ms: Option<u64>,
    load_ms: Option<u64>,
    convert_ms: Option<u64>,
    error: Option<String>,
    native_enabled: Option<bool>,
}

impl Visit for CairoNativeEventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => {
                // Remove quotes from Debug formatting.
                let formatted = format!("{:?}", value);
                self.message = formatted.trim_matches('"').to_string();
            }
            "class_hash" => {
                let formatted = format!("{:?}", value);
                self.class_hash = Some(formatted.trim_matches('"').to_string());
            }
            "elapsed" | "duration" | "compile" | "load" | "convert" | "total" => {
                // Format Duration nicely by removing the surrounding quotes
                // while preserving the formatted duration string.
                let formatted = format!("{:?}", value);
                self.elapsed = Some(formatted.trim_matches('"').to_string());
            }
            "error" => {
                let formatted = format!("{:?}", value);
                self.error = Some(formatted.trim_matches('"').to_string());
            }
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => {
                self.message = value.to_string();
            }
            "class_hash" => {
                self.class_hash = Some(value.to_string());
            }
            "error" => {
                self.error = Some(value.to_string());
            }
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "elapsed_ms" => {
                self.elapsed_ms = Some(value);
            }
            "conversion_ms" => {
                self.conversion_ms = Some(value);
            }
            "load_ms" => {
                self.load_ms = Some(value);
            }
            "convert_ms" => {
                self.convert_ms = Some(value);
            }
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "native_enabled" {
            self.native_enabled = Some(value);
        }
    }
}

/// Visitor that collects all fields from a close_block event into a JSON
/// object.
#[derive(Default)]
struct CloseBlockEventVisitor {
    fields: Vec<(String, serde_json::Value)>,
    message: String,
}

impl Visit for CloseBlockEventVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.push((field.name().to_string(), serde_json::Value::Number(value.into())));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.push((field.name().to_string(), serde_json::Value::Number(value.into())));
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        // JSON does not support u128 natively, so convert large values to
        // strings when they do not fit in u64.
        if value <= u64::MAX as u128 {
            self.fields.push((field.name().to_string(), serde_json::Value::Number((value as u64).into())));
        } else {
            self.fields.push((field.name().to_string(), serde_json::Value::String(value.to_string())));
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.fields.push((field.name().to_string(), serde_json::Value::Number(n)));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push((field.name().to_string(), serde_json::Value::String(value.to_string())));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.push((field.name().to_string(), serde_json::Value::Bool(value)));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let formatted = format!("{:?}", value);
            self.message = formatted.trim_matches('"').to_string();
        } else {
            let formatted = format!("{:?}", value);
            self.fields
                .push((field.name().to_string(), serde_json::Value::String(formatted.trim_matches('"').to_string())));
        }
    }
}

impl CairoNativeEventVisitor {
    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_class_hash(&self) -> Option<&str> {
        self.class_hash.as_deref()
    }

    pub fn get_elapsed(&self) -> Option<&str> {
        self.elapsed.as_deref()
    }

    pub fn get_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn get_elapsed_ms(&self) -> Option<u64> {
        self.elapsed_ms
    }

    pub fn get_conversion_ms(&self) -> Option<u64> {
        self.conversion_ms
    }

    pub fn get_load_ms(&self) -> Option<u64> {
        self.load_ms
    }

    pub fn get_convert_ms(&self) -> Option<u64> {
        self.convert_ms
    }

    pub fn get_native_enabled(&self) -> Option<bool> {
        self.native_enabled
    }
}

pub fn visit_message(event: &tracing::Event<'_>, f: impl FnOnce(&dyn fmt::Debug) -> fmt::Result) -> fmt::Result {
    struct Visitor<F>(Option<F>, fmt::Result);

    impl<F: FnOnce(&dyn fmt::Debug) -> fmt::Result> Visit for Visitor<F> {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                if let Some(f) = self.0.take() {
                    self.1 = (f)(value);
                }
            }
        }
    }

    let mut visitor = Visitor(Some(f), Ok(()));
    event.record(&mut visitor);
    visitor.1
}

pub struct CustomFormatter {
    local_offset: UtcOffset,
    dim_style: Style,
    open_bracket_dim: StyledObject<&'static str>,
    closed_bracket_dim: StyledObject<&'static str>,
    ts_format: Vec<format_description::BorrowedFormatItem<'static>>,
}

impl CustomFormatter {
    pub fn new() -> Self {
        let dim_style = Style::new().dim();
        Self {
            open_bracket_dim: dim_style.apply_to("["),
            closed_bracket_dim: dim_style.apply_to("]"),
            local_offset: UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC),
            dim_style,
            ts_format: format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]:[subsecond digits:3]")
                .expect("Invalid date format constant"),
        }
    }

    fn timestamp_fmt<'a>(&'a self, ts: &'a SystemTime) -> impl fmt::Display + 'a {
        display_fn(|f| {
            let datetime: OffsetDateTime = (*ts).into();
            let local_datetime = datetime.to_offset(self.local_offset);
            // This allocates a String as part of the time formatting.
            match local_datetime.format(&self.ts_format) {
                Ok(ts) => {
                    write!(f, "{}{}{}", self.open_bracket_dim, self.dim_style.apply_to(ts), self.closed_bracket_dim)
                }
                Err(_) => {
                    write!(f, "<error>")
                }
            }
        })
    }

    fn format_without_target(
        &self,
        writer: &mut Writer<'_>,
        event: &tracing::Event<'_>,
        ts: &SystemTime,
        level: &Level,
        level_style: &Style,
    ) -> fmt::Result {
        visit_message(event, |message| {
            writeln!(writer, "{} {} {:?}", self.timestamp_fmt(ts), level_style.apply_to(level), message,)
        })
    }

    fn format_with_target(
        &self,
        writer: &mut Writer<'_>,
        event: &tracing::Event<'_>,
        target: &str,
        ts: &SystemTime,
        level: &Level,
        level_style: &Style,
    ) -> fmt::Result {
        visit_message(event, |message| {
            writeln!(
                writer,
                "{} {} {} {:?}",
                self.timestamp_fmt(ts),
                level_style.apply_to(level),
                self.dim_style.apply_to(target),
                message,
            )
        })
    }

    fn format_cairo_native(
        &self,
        writer: &mut Writer<'_>,
        event: &tracing::Event<'_>,
        ts: &SystemTime,
        level: &Level,
    ) -> fmt::Result {
        let mut visitor = CairoNativeEventVisitor::default();
        event.record(&mut visitor);

        let message = visitor.get_message();
        let class_hash = visitor.get_class_hash();
        let elapsed = visitor.get_elapsed();
        let elapsed_ms = visitor.get_elapsed_ms();
        let conversion_ms = visitor.get_conversion_ms();
        let load_ms = visitor.get_load_ms();
        let convert_ms = visitor.get_convert_ms();
        let error = visitor.get_error();
        let native_enabled = visitor.get_native_enabled();

        // Darker cyan for the CAIRO_NATIVE prefix to keep it visible but
        // understated.
        let cairo_native_prefix = Style::new().cyan().dim().apply_to("CAIRO_NATIVE");
        // Muted styling keeps class_hash metadata less prominent than the main
        // prefix and message.
        let class_hash_style = Style::new().dim();
        let error_style = Style::new().red();

        // Format: timestamp + CAIRO_NATIVE + optional level prefix + message +
        // class_hash + native_enabled + optional error + optional timing.
        write!(writer, "{} {}", self.timestamp_fmt(ts), cairo_native_prefix)?;

        // Level prefix is shown for WARN and ERROR only.
        match *level {
            Level::WARN => {
                write!(writer, " {}", Style::new().yellow().apply_to("WARN"))?;
            }
            Level::ERROR => {
                write!(writer, " {}", Style::new().red().apply_to("ERROR"))?;
            }
            _ => {} // Other levels do not show an explicit prefix.
        }

        // Message formatting improves clarity by capitalizing and replacing
        // underscores.
        let formatted_message = Self::format_message(message);
        write!(writer, " {}", formatted_message)?;

        if let Some(hash) = class_hash {
            // class_hash is displayed in full, but with less prominent styling.
            // Quotes are removed if present from Debug formatting.
            let hash_clean = hash.trim_matches('"');
            write!(writer, " {}", class_hash_style.apply_to(format!("class_hash={}", hash_clean)))?;
        }

        // Display native_enabled when present.
        if let Some(enabled) = native_enabled {
            write!(writer, " {}", class_hash_style.apply_to(format!("native_enabled={}", enabled)))?;
        }

        if let Some(err) = error {
            write!(writer, " {}", error_style.apply_to(format!("error={}", err)))?;
        }

        // Prefer elapsed_ms when available, otherwise fall back to the
        // formatted Duration string.
        let timing_str = Self::format_timing(elapsed_ms, conversion_ms, load_ms, convert_ms, elapsed);
        if let Some(timing) = timing_str {
            let elapsed_style = Self::get_timing_style(&timing);
            write!(writer, " {}", elapsed_style.apply_to(format!("- {}", timing)))?;
        }

        writeln!(writer)
    }

    /// Format timing information from the available metric fields.
    /// Prefers elapsed_ms and includes a breakdown when component timings are
    /// present.
    fn format_timing(
        elapsed_ms: Option<u64>,
        conversion_ms: Option<u64>,
        load_ms: Option<u64>,
        convert_ms: Option<u64>,
        elapsed: Option<&str>,
    ) -> Option<String> {
        // If elapsed_ms is present, use it as the primary timing value.
        if let Some(total_ms) = elapsed_ms {
            let mut parts = Vec::new();

            // Format the total elapsed time first.
            let total_str =
                if total_ms >= 1000 { format!("{:.3}s", total_ms as f64 / 1000.0) } else { format!("{}ms", total_ms) };
            parts.push(total_str);

            // Add a breakdown when individual component timings are available.
            let mut breakdown = Vec::new();
            if let Some(load) = load_ms {
                breakdown.push(format!("load: {}ms", load));
            }
            if let Some(convert) = convert_ms {
                breakdown.push(format!("convert: {}ms", convert));
            }
            if let Some(conv) = conversion_ms {
                breakdown.push(format!("conversion: {}ms", conv));
            }

            if !breakdown.is_empty() {
                parts.push(format!("({})", breakdown.join(", ")));
            }

            return Some(parts.join(" "));
        }

        // Fall back to the formatted Duration string when no elapsed_ms value
        // is available.
        elapsed.map(|s| s.to_string())
    }

    /// Format messages for readability.
    fn format_message(message: &str) -> String {
        // Remove quotes when the message came through Debug formatting.
        let cleaned = message.trim_matches('"');
        // Replace underscores with spaces for readability.
        let spaced = cleaned.replace('_', " ");

        // Capitalize the first letter for display.
        if let Some(first) = spaced.chars().next() {
            format!("{}{}", first.to_uppercase(), &spaced[1..])
        } else {
            spaced
        }
    }

    /// Pick a display style based on the duration value.
    /// Duration Debug format examples: "15.833µs", "1.732s", "234ms", "123ns".
    fn get_timing_style(timing_str: &str) -> Style {
        // Parse the timing string to infer the unit and relative severity.
        let timing_clean = timing_str.trim();

        // Handle longer suffixes before the generic seconds suffix.
        if timing_clean.ends_with("ns") || timing_clean.ends_with("µs") || timing_clean.ends_with("us") {
            // Nanoseconds and microseconds are always treated as fast.
            return Style::new().dim();
        } else if timing_clean.ends_with("ms") {
            // Millisecond timings become yellow/red when they cross the
            // relevant thresholds.
            if let Ok(val) = timing_clean.trim_end_matches("ms").trim().parse::<f64>() {
                if val > 1000.0 {
                    return Style::new().red(); // Very slow (>1s).
                } else if val > 100.0 {
                    return Style::new().yellow(); // Slow (>100ms).
                }
            }
            return Style::new().dim();
        } else if timing_clean.ends_with('s') {
            // Multi-second timings become yellow/red for slower operations.
            if let Ok(val) = timing_clean.trim_end_matches('s').trim().parse::<f64>() {
                if val > 5.0 {
                    return Style::new().red(); // Very slow (>5s).
                } else if val > 1.0 {
                    return Style::new().yellow(); // Slow (>1s).
                }
            }
            return Style::new().dim();
        }

        // Default to dim styling for unknown formats.
        Style::new().dim()
    }

    fn format_http_call(
        &self,
        writer: &mut Writer<'_>,
        event: &tracing::Event<'_>,
        target: &str,
        ts: &SystemTime,
        level: &Level,
    ) -> fmt::Result {
        let mut visitor = RpcCallEventVisitor::default();
        event.record(&mut visitor);
        let Some(rpc_call_event) = visitor.get() else {
            // Fall back to the normal formatter when the RPC-specific fields
            // are not present.
            return self.format_with_target(writer, event, target, ts, level, &Style::new().blue());
        };

        let status_style = if (400..=600).contains(&rpc_call_event.status) || rpc_call_event.status < 100 {
            Style::new().red()
        } else {
            Style::new().green()
        };
        let time_style = if rpc_call_event.response_time <= 5000 { Style::new() } else { Style::new().yellow() };

        if target == "gateway_calls" {
            writeln!(
                writer,
                "{} {} {} {} {} bytes - {:.3?}",
                self.timestamp_fmt(ts),
                Style::new().blue().apply_to("GATEWAY"),
                rpc_call_event.method,
                status_style.apply_to(&rpc_call_event.status),
                rpc_call_event.res_len,
                // Conversion from micros u128 to u64 should be safe here.
                time_style.apply_to(&Duration::from_micros(rpc_call_event.response_time as u64)),
            )
        } else {
            writeln!(
                writer,
                "{} {} {} {} {} bytes - {:.3?}",
                self.timestamp_fmt(ts),
                Style::new().magenta().apply_to("RPC"),
                rpc_call_event.method,
                status_style.apply_to(&rpc_call_event.status),
                rpc_call_event.res_len,
                // Conversion from micros u128 to u64 should be safe here.
                time_style.apply_to(&Duration::from_micros(rpc_call_event.response_time as u64)),
            )
        }
    }

    /// Format close_block events as JSON for Loki ingestion.
    fn format_close_block(
        &self,
        writer: &mut Writer<'_>,
        event: &tracing::Event<'_>,
        ts: &SystemTime,
        level: &Level,
        target: &str,
    ) -> fmt::Result {
        let mut visitor = CloseBlockEventVisitor::default();
        event.record(&mut visitor);

        // Build a JSON object with timestamp, level, target, and all captured
        // fields.
        let datetime: OffsetDateTime = (*ts).into();
        let local_datetime = datetime.to_offset(self.local_offset);
        let timestamp_str = local_datetime
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".to_string());

        let mut json_obj = serde_json::Map::new();
        json_obj.insert("timestamp".to_string(), serde_json::Value::String(timestamp_str));
        json_obj.insert("level".to_string(), serde_json::Value::String(level.to_string()));
        json_obj.insert("target".to_string(), serde_json::Value::String(target.to_string()));
        json_obj.insert("message".to_string(), serde_json::Value::String(visitor.message.clone()));

        // Add all captured event fields.
        for (key, value) in visitor.fields {
            json_obj.insert(key, value);
        }

        let json_str = serde_json::to_string(&serde_json::Value::Object(json_obj)).unwrap_or_else(|_| "{}".to_string());
        writeln!(writer, "{}", json_str)
    }
}

impl<S, N> FormatEvent<S, N> for CustomFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let ts = SystemTime::now();
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        match (level, target) {
            (&Level::INFO, "rpc_calls" | "gateway_calls") => {
                self.format_http_call(&mut writer, event, target, &ts, level)
            }
            (_, "madara_cairo_native") => self.format_cairo_native(&mut writer, event, &ts, level),
            (_, "close_block") => self.format_close_block(&mut writer, event, &ts, level, target),
            (&Level::INFO, _) => self.format_without_target(&mut writer, event, &ts, level, &Style::new().green()),
            (&Level::WARN, _) => {
                self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().yellow())
            }
            (&Level::ERROR, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().red()),
            (&Level::DEBUG, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().blue()),
            (&Level::TRACE, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().cyan()),
        }
    }
}
