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

// TODO: it sucks that console::Style allocates when styling a message, this is so dumb. It bothers me a lot.
pub struct CustomFormatter {
    local_offset: UtcOffset,
    dim_style: Style,
    open_bracket: StyledObject<&'static str>,
    closed_bracket: StyledObject<&'static str>,
    ts_format: Vec<format_description::BorrowedFormatItem<'static>>,
}

impl CustomFormatter {
    pub fn new() -> Self {
        let dim_style = Style::new().dim();
        Self {
            open_bracket: dim_style.apply_to("["),
            closed_bracket: dim_style.apply_to("]"),
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
                    write!(f, "{}{}{}", self.open_bracket, self.dim_style.apply_to(ts), self.closed_bracket,)
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
            writeln!(
                writer,
                "{} {}{}{} {:?}",
                self.timestamp_fmt(ts),
                self.open_bracket,
                level_style.apply_to(level),
                self.closed_bracket,
                message,
            )
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
                "{} {}{}{} {} {:?}",
                self.timestamp_fmt(ts),
                self.open_bracket,
                level_style.apply_to(level),
                self.closed_bracket,
                self.dim_style.apply_to(target),
                message,
            )
        })
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
            (&Level::INFO, "rpc_calls") => {
                // TODO: there is a String allocation in there that we could avoid with TLS or something, probably.
                // This is the reason there is a reset function in RpcCallEventVisitor, so that we can reuse that allocation.

                let mut visitor = RpcCallEventVisitor::default();
                event.record(&mut visitor);
                let Some(rpc_call_event) = visitor.get() else {
                    // Fallback to normal formatter.
                    return self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().blue());
                };

                let rpc_style = Style::new().magenta();

                let status_style = if rpc_call_event.status == 200 { Style::new().green() } else { Style::new().red() };

                let time_style =
                    if rpc_call_event.response_time <= 5000 { Style::new() } else { Style::new().yellow() };

                writeln!(
                    writer,
                    "{} {}{}{} {} {} {} bytes - {:.3?}",
                    self.timestamp_fmt(&ts),
                    self.open_bracket,
                    rpc_style.apply_to("HTTP"),
                    self.closed_bracket,
                    rpc_call_event.method,
                    status_style.apply_to(&rpc_call_event.status),
                    rpc_call_event.res_len,
                    // Convertion from micros u128 to u64 should be fine.
                    time_style.apply_to(&Duration::from_micros(rpc_call_event.response_time as u64)),
                )
            }
            (&Level::INFO, _) => self.format_without_target(&mut writer, event, &ts, level, &Style::new().green()),
            (&Level::WARN, _) => {
                self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().yellow())
            }
            // no need to display target for this particular error.
            (&Level::ERROR, "rpc_errors") => {
                self.format_without_target(&mut writer, event, &ts, level, &Style::new().red())
            }
            (&Level::ERROR, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().red()),
            (&Level::DEBUG, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().blue()),
            (&Level::TRACE, _) => self.format_with_target(&mut writer, event, target, &ts, level, &Style::new().cyan()),
        }
    }
}
