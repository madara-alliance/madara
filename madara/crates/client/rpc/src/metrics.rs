use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};

pub const SUBSCRIBE_NEW_HEADS: &str = "subscribeNewHeads";
pub const SUBSCRIBE_EVENTS: &str = "subscribeEvents";
pub const SUBSCRIBE_TRANSACTION_STATUS: &str = "subscribeTransactionStatus";
pub const SUBSCRIBE_NEW_TRANSACTIONS: &str = "subscribeNewTransactions";
pub const SUBSCRIBE_NEW_TRANSACTION_RECEIPTS: &str = "subscribeNewTransactionReceipts";

pub const SOURCE_REORG: &str = "reorg";
pub const SOURCE_NEW_TRANSACTION: &str = "new_transaction";

#[derive(Debug)]
pub struct WsMetrics {
    active_subscriptions: Gauge<u64>,
    active_subscriptions_by_method: Gauge<u64>,
    subscriptions_opened: Counter<u64>,
    subscriptions_closed: Counter<u64>,
    notifications_sent: Counter<u64>,
    notification_send_failures: Counter<u64>,
    reorg_notifications_sent: Counter<u64>,
    lagged_notifications: Counter<u64>,
    subscription_duration: Histogram<f64>,
}

impl WsMetrics {
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.rpc.websocket.opentelemetry")
                .with_attributes([KeyValue::new("crate", "rpc")])
                .build(),
        );

        Self {
            active_subscriptions: register_gauge_metric_instrument(
                &meter,
                "ws_active_subscriptions".to_string(),
                "Current websocket subscriptions".to_string(),
                "subscription".to_string(),
            ),
            active_subscriptions_by_method: register_gauge_metric_instrument(
                &meter,
                "ws_active_subscriptions_by_method".to_string(),
                "Current websocket subscriptions by method".to_string(),
                "subscription".to_string(),
            ),
            subscriptions_opened: register_counter_metric_instrument(
                &meter,
                "ws_subscriptions_opened".to_string(),
                "Websocket subscriptions opened".to_string(),
                "subscription".to_string(),
            ),
            subscriptions_closed: register_counter_metric_instrument(
                &meter,
                "ws_subscriptions_closed".to_string(),
                "Websocket subscriptions closed".to_string(),
                "subscription".to_string(),
            ),
            notifications_sent: register_counter_metric_instrument(
                &meter,
                "ws_subscription_notifications_sent".to_string(),
                "Websocket subscription notifications sent".to_string(),
                "notification".to_string(),
            ),
            notification_send_failures: register_counter_metric_instrument(
                &meter,
                "ws_subscription_notification_send_failures".to_string(),
                "Websocket subscription notification send failures".to_string(),
                "notification".to_string(),
            ),
            reorg_notifications_sent: register_counter_metric_instrument(
                &meter,
                "ws_subscription_reorg_notifications_sent".to_string(),
                "Websocket reorg notifications sent".to_string(),
                "notification".to_string(),
            ),
            lagged_notifications: register_counter_metric_instrument(
                &meter,
                "ws_subscription_lagged_notifications".to_string(),
                "Websocket subscription notifications missed because a receiver lagged".to_string(),
                "notification".to_string(),
            ),
            subscription_duration: register_histogram_metric_instrument(
                &meter,
                "ws_subscription_duration_seconds".to_string(),
                "Websocket subscription lifetime".to_string(),
                "s".to_string(),
            ),
        }
    }

    pub fn record_subscription_opened(&self, method: &'static str) {
        self.subscriptions_opened.add(1, &[method_label(method)]);
    }

    pub fn record_subscription_closed(&self, method: &'static str) {
        self.subscriptions_closed.add(1, &[method_label(method)]);
    }

    pub fn record_active_subscriptions(&self, count: u64) {
        self.active_subscriptions.record(count, &[]);
    }

    pub fn record_active_subscriptions_for_method(&self, method: &'static str, count: u64) {
        self.active_subscriptions_by_method.record(count, &[method_label(method)]);
    }

    pub fn record_notification_sent(&self, method: &'static str) {
        self.notifications_sent.add(1, &[method_label(method)]);
    }

    pub fn record_notification_send_failure(&self, method: &'static str) {
        self.notification_send_failures.add(1, &[method_label(method)]);
    }

    pub fn record_reorg_notification_sent(&self) {
        self.reorg_notifications_sent.add(1, &[]);
    }

    pub fn record_lagged_notification(&self, method: &'static str, source: &'static str) {
        self.lagged_notifications.add(1, &[method_label(method), KeyValue::new("source", source)]);
    }

    pub fn record_subscription_duration(&self, method: &'static str, duration_secs: f64) {
        self.subscription_duration.record(duration_secs, &[method_label(method)]);
    }
}

fn method_label(method: &'static str) -> KeyValue {
    KeyValue::new("method", method)
}

static WS_METRICS: std::sync::LazyLock<WsMetrics> = std::sync::LazyLock::new(WsMetrics::register);

pub fn ws_metrics() -> &'static WsMetrics {
    &WS_METRICS
}

pub fn record_lagged_reorg(method: &'static str) {
    ws_metrics().record_lagged_notification(method, SOURCE_REORG);
}

pub fn record_lagged_new_transaction(method: &'static str) {
    ws_metrics().record_lagged_notification(method, SOURCE_NEW_TRANSACTION);
}
