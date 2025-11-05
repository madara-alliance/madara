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
    error: Option<String>,
}

impl Visit for CairoNativeEventVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => {
                // Remove quotes from Debug formatting
                let formatted = format!("{:?}", value);
                self.message = formatted.trim_matches('"').to_string();
            }
            "class_hash" => {
                let formatted = format!("{:?}", value);
                self.class_hash = Some(formatted.trim_matches('"').to_string());
            }
            "elapsed" | "duration" | "compile" | "load" | "convert" | "total" => {
                // Format Duration nicely (remove quotes, keep the formatted duration)
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
            // This allocates a String also :/
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
        let error = visitor.get_error();

        // Darker cyan for CAIRO_NATIVE prefix (more subtle)
        let cairo_native_prefix = Style::new().cyan().dim().apply_to("CAIRO_NATIVE");
        // More muted color for class_hash (less bright than CAIRO_NATIVE) - use dim white/gray
        let class_hash_style = Style::new().dim();
        let error_style = Style::new().red();

        // Format: timestamp + CAIRO_NATIVE + [WARN/ERROR] + message + class_hash + (optional error) + (optional timing)
        write!(writer, "{} {}", self.timestamp_fmt(ts), cairo_native_prefix)?;

        // Add level prefix for WARN and ERROR only
        match level {
            &Level::WARN => {
                write!(writer, " {}", Style::new().yellow().apply_to("WARN"))?;
            }
            &Level::ERROR => {
                write!(writer, " {}", Style::new().red().apply_to("ERROR"))?;
            }
            _ => {} // INFO and DEBUG don't show level prefix
        }

        // Improve message clarity - add context if needed
        let formatted_message = Self::format_message(message);
        write!(writer, " {}", formatted_message)?;

        if let Some(hash) = class_hash {
            // Format class_hash - keep full hash but make it less prominent
            // Remove quotes if present from Debug formatting
            let hash_clean = hash.trim_matches('"');
            write!(writer, " {}", class_hash_style.apply_to(format!("class_hash={}", hash_clean)))?;
        }

        if let Some(err) = error {
            write!(writer, " {}", error_style.apply_to(format!("error={}", err)))?;
        }

        // Color-code timing based on duration
        if let Some(timing) = elapsed {
            let elapsed_style = Self::get_timing_style(&timing);
            write!(writer, " {}", elapsed_style.apply_to(format!("- {}", timing)))?;
        }

        writeln!(writer)
    }

    /// Format message for better clarity
    fn format_message(message: &str) -> String {
        // Remove quotes if present from Debug formatting
        let cleaned = message.trim_matches('"');

        // Replace underscores with spaces for readability
        let spaced = cleaned.replace("_", " ");

        // Capitalize first letter
        if let Some(first) = spaced.chars().next() {
            format!("{}{}", first.to_uppercase(), &spaced[1..])
        } else {
            spaced
        }
    }

    /// Get style for timing based on duration value
    /// Duration Debug format: "15.833µs", "1.732s", "234ms", "123ns"
    fn get_timing_style(timing_str: &str) -> Style {
        // Parse timing string to extract numeric value and unit
        let timing_clean = timing_str.trim();

        // Handle different time units (check longer units first)
        if timing_clean.ends_with("ns") {
            // Nanoseconds - always very fast, use dim
            return Style::new().dim();
        } else if timing_clean.ends_with("µs") || timing_clean.ends_with("us") {
            // Microseconds - always fast, use dim
            return Style::new().dim();
        } else if timing_clean.ends_with("ms") {
            // Milliseconds
            if let Ok(val) = timing_clean.trim_end_matches("ms").trim().parse::<f64>() {
                if val > 1000.0 {
                    return Style::new().red(); // Very slow (>1s)
                } else if val > 100.0 {
                    return Style::new().yellow(); // Slow (>100ms)
                }
            }
            return Style::new().dim(); // <100ms, normal
        } else if timing_clean.ends_with("s") {
            // Seconds - yellow/red for slower operations
            if let Ok(val) = timing_clean.trim_end_matches("s").trim().parse::<f64>() {
                if val > 5.0 {
                    return Style::new().red(); // Very slow (>5s)
                } else if val > 1.0 {
                    return Style::new().yellow(); // Slow (>1s)
                }
            }
            return Style::new().dim(); // <1s, normal
        }

        // Default: dim for unknown format
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
            // Fallback to normal formatter.
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
                // Conversion from micros u128 to u64 should be fine.
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
                // Convertion from micros u128 to u64 should be fine.
                time_style.apply_to(&Duration::from_micros(rpc_call_event.response_time as u64)),
            )
        }
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
            (_, "madara.cairo_native") => self.format_cairo_native(&mut writer, event, &ts, level),
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
