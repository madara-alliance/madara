use opentelemetry::{InstrumentationScope, Key, Value};
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::{
    metrics::{
        data::{self, ResourceMetrics},
        reader::MetricReader,
        InstrumentKind, ManualReader, ManualReaderBuilder, Pipeline, Temporality,
    },
    Resource,
};
use prometheus::core::Collector as PrometheusCollector;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, Weak};

const TARGET_INFO_NAME: &str = "target_info";
const TARGET_INFO_DESCRIPTION: &str = "Target metadata";

const SCOPE_INFO_METRIC_NAME: &str = "otel_scope_info";
const SCOPE_INFO_DESCRIPTION: &str = "Instrumentation Scope metadata";

const SCOPE_INFO_KEYS: [&str; 2] = ["otel_scope_name", "otel_scope_version"];

const COUNTER_SUFFIX: &str = "_total";

#[derive(Default)]
pub struct ExporterBuilder {
    registry: Option<prometheus::Registry>,
}

impl fmt::Debug for ExporterBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExporterBuilder").field("registry", &self.registry).finish()
    }
}

pub fn exporter() -> ExporterBuilder {
    ExporterBuilder::default()
}

impl ExporterBuilder {
    pub fn with_registry(mut self, registry: prometheus::Registry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn build(self) -> Result<PrometheusExporter, OTelSdkError> {
        let reader = Arc::new(ManualReaderBuilder::new().build());
        let collector = Collector {
            reader: Arc::clone(&reader),
            create_target_info_once: OnceLock::new(),
            inner: Mutex::new(CollectorInner::default()),
        };

        let registry = self.registry.unwrap_or_default();
        registry.register(Box::new(collector)).map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?;

        Ok(PrometheusExporter { reader })
    }
}

#[derive(Debug)]
pub struct PrometheusExporter {
    reader: Arc<ManualReader>,
}

impl MetricReader for PrometheusExporter {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.reader.register_pipeline(pipeline)
    }

    fn collect(&self, rm: &mut ResourceMetrics) -> OTelSdkResult {
        self.reader.collect(rm)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.reader.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: std::time::Duration) -> OTelSdkResult {
        self.reader.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.reader.temporality(kind)
    }
}

#[derive(Debug)]
struct Collector {
    reader: Arc<ManualReader>,
    create_target_info_once: OnceLock<MetricFamily>,
    inner: Mutex<CollectorInner>,
}

#[derive(Debug, Default)]
struct CollectorInner {
    scope_infos: HashMap<InstrumentationScope, MetricFamily>,
    metric_families: HashMap<String, MetricFamily>,
}

impl Collector {
    fn metric_type_and_name(&self, metric: &data::Metric) -> Option<(MetricType, Cow<'static, str>)> {
        let mut name = self.get_name(metric);

        let result = match metric.data() {
            data::AggregatedMetrics::F64(metric_data) => match metric_data {
                data::MetricData::Histogram(_) => Some(MetricType::HISTOGRAM),
                data::MetricData::Gauge(_) => Some(MetricType::GAUGE),
                data::MetricData::Sum(sum) => {
                    if sum.is_monotonic() {
                        name = format!("{name}{COUNTER_SUFFIX}").into();
                        Some(MetricType::COUNTER)
                    } else {
                        Some(MetricType::GAUGE)
                    }
                }
                data::MetricData::ExponentialHistogram(_) => None,
            },
            data::AggregatedMetrics::I64(metric_data) => match metric_data {
                data::MetricData::Histogram(_) => Some(MetricType::HISTOGRAM),
                data::MetricData::Gauge(_) => Some(MetricType::GAUGE),
                data::MetricData::Sum(sum) => {
                    if sum.is_monotonic() {
                        name = format!("{name}{COUNTER_SUFFIX}").into();
                        Some(MetricType::COUNTER)
                    } else {
                        Some(MetricType::GAUGE)
                    }
                }
                data::MetricData::ExponentialHistogram(_) => None,
            },
            data::AggregatedMetrics::U64(metric_data) => match metric_data {
                data::MetricData::Histogram(_) => Some(MetricType::HISTOGRAM),
                data::MetricData::Gauge(_) => Some(MetricType::GAUGE),
                data::MetricData::Sum(sum) => {
                    if sum.is_monotonic() {
                        name = format!("{name}{COUNTER_SUFFIX}").into();
                        Some(MetricType::COUNTER)
                    } else {
                        Some(MetricType::GAUGE)
                    }
                }
                data::MetricData::ExponentialHistogram(_) => None,
            },
        };

        result.map(|metric_type| (metric_type, name))
    }

    fn get_name(&self, metric: &data::Metric) -> Cow<'static, str> {
        let name: Cow<'static, str> = Cow::Owned(metric.name().to_string());
        let name = sanitize_name(&name);
        match get_unit_suffixes(metric.unit()) {
            Some(suffix) => Cow::Owned(format!("{name}_{suffix}")),
            None => name,
        }
    }
}

impl PrometheusCollector for Collector {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        Vec::new()
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(err) => {
                tracing::error!(error = %err, "Metric scrape failed");
                return Vec::new();
            }
        };

        let mut metrics = ResourceMetrics::default();
        if let Err(err) = self.reader.collect(&mut metrics) {
            tracing::error!(error = %err, "Metric scrape failed");
            return vec![];
        }

        let mut res = Vec::with_capacity(metrics.scope_metrics().size_hint().0 + 1);

        let target_info = self
            .create_target_info_once
            .get_or_init(|| create_info_metric(TARGET_INFO_NAME, TARGET_INFO_DESCRIPTION, metrics.resource()));

        if !metrics.resource().is_empty() {
            res.push(target_info.clone());
        }

        for scope_metrics in metrics.scope_metrics() {
            let scope_labels = if scope_metrics.scope().attributes().count() > 0 {
                let scope_info =
                    inner.scope_infos.entry(scope_metrics.scope().clone()).or_insert_with_key(create_scope_info_metric);
                res.push(scope_info.clone());

                let mut labels = Vec::with_capacity(1 + scope_metrics.scope().version().is_some() as usize);
                let mut name = LabelPair::new();
                name.set_name(SCOPE_INFO_KEYS[0].into());
                name.set_value(scope_metrics.scope().name().to_string());
                labels.push(name);
                if let Some(version) = scope_metrics.scope().version() {
                    let mut version_label = LabelPair::new();
                    version_label.set_name(SCOPE_INFO_KEYS[1].into());
                    version_label.set_value(version.to_string());
                    labels.push(version_label);
                }
                labels
            } else {
                let mut labels = Vec::with_capacity(1 + scope_metrics.scope().version().is_some() as usize);
                let mut name = LabelPair::new();
                name.set_name(SCOPE_INFO_KEYS[0].into());
                name.set_value(scope_metrics.scope().name().to_string());
                labels.push(name);
                if let Some(version) = scope_metrics.scope().version() {
                    let mut version_label = LabelPair::new();
                    version_label.set_name(SCOPE_INFO_KEYS[1].into());
                    version_label.set_value(version.to_string());
                    labels.push(version_label);
                }
                labels
            };

            for metric in scope_metrics.metrics() {
                let (metric_type, name) = match self.metric_type_and_name(metric) {
                    Some((metric_type, name)) => (metric_type, name),
                    None => continue,
                };

                let (drop, help) =
                    validate_metrics(&name, metric.description(), metric_type, &mut inner.metric_families);
                if drop {
                    continue;
                }

                let description = help.unwrap_or_else(|| metric.description().into());

                match metric.data() {
                    data::AggregatedMetrics::F64(metric_data) => match metric_data {
                        data::MetricData::Histogram(hist) => {
                            add_histogram_metric(&mut res, hist, description, &scope_labels, name);
                        }
                        data::MetricData::Sum(sum) => {
                            add_sum_metric(&mut res, sum, description, &scope_labels, name);
                        }
                        data::MetricData::Gauge(gauge) => {
                            add_gauge_metric(&mut res, gauge, description, &scope_labels, name);
                        }
                        data::MetricData::ExponentialHistogram(_) => {}
                    },
                    data::AggregatedMetrics::I64(metric_data) => match metric_data {
                        data::MetricData::Histogram(hist) => {
                            add_histogram_metric(&mut res, hist, description, &scope_labels, name);
                        }
                        data::MetricData::Sum(sum) => {
                            add_sum_metric(&mut res, sum, description, &scope_labels, name);
                        }
                        data::MetricData::Gauge(gauge) => {
                            add_gauge_metric(&mut res, gauge, description, &scope_labels, name);
                        }
                        data::MetricData::ExponentialHistogram(_) => {}
                    },
                    data::AggregatedMetrics::U64(metric_data) => match metric_data {
                        data::MetricData::Histogram(hist) => {
                            add_histogram_metric(&mut res, hist, description, &scope_labels, name);
                        }
                        data::MetricData::Sum(sum) => {
                            add_sum_metric(&mut res, sum, description, &scope_labels, name);
                        }
                        data::MetricData::Gauge(gauge) => {
                            add_gauge_metric(&mut res, gauge, description, &scope_labels, name);
                        }
                        data::MetricData::ExponentialHistogram(_) => {}
                    },
                }
            }
        }

        res
    }
}

fn get_attrs(kvs: &mut dyn Iterator<Item = (&Key, &Value)>, extra: &[LabelPair]) -> Vec<LabelPair> {
    let mut keys_map = BTreeMap::<String, Vec<String>>::new();
    for (key, value) in kvs {
        let key = sanitize_prom_kv(key.as_str());
        keys_map
            .entry(key)
            .and_modify(|values| values.push(value.to_string()))
            .or_insert_with(|| vec![value.to_string()]);
    }

    let mut res = Vec::with_capacity(keys_map.len() + extra.len());
    for (key, mut values) in keys_map {
        values.sort_unstable();

        let mut label = LabelPair::new();
        label.set_name(key);
        label.set_value(values.join(";"));
        res.push(label);
    }

    res.extend(extra.iter().cloned());
    res
}

fn validate_metrics(
    name: &str,
    description: &str,
    metric_type: MetricType,
    mfs: &mut HashMap<String, MetricFamily>,
) -> (bool, Option<String>) {
    if let Some(existing) = mfs.get(name) {
        if existing.get_field_type() != metric_type {
            tracing::warn!(
                metric_name = name,
                existing_type = ?existing.get_field_type(),
                dropped_type = ?metric_type,
                "Instrument type conflict, using existing type definition"
            );
            return (true, None);
        }
        if existing.get_help() != description {
            tracing::warn!(
                metric_name = name,
                existing_help = existing.get_help(),
                dropped_help = description,
                "Instrument description conflict, using existing"
            );
            return (false, Some(existing.get_help().to_string()));
        }
        (false, None)
    } else {
        let mut mf = MetricFamily::default();
        mf.set_name(name.into());
        mf.set_help(description.to_string());
        mf.set_field_type(metric_type);
        mfs.insert(name.to_string(), mf);
        (false, None)
    }
}

fn add_histogram_metric<T: Numeric + Copy>(
    res: &mut Vec<MetricFamily>,
    histogram: &data::Histogram<T>,
    description: String,
    extra: &[LabelPair],
    name: Cow<'static, str>,
) {
    for dp in histogram.data_points() {
        let kvs = get_attrs(&mut dp.attributes().map(|kv| (&kv.key, &kv.value)), extra);
        let bounds: Vec<f64> = dp.bounds().collect();
        let bucket_counts: Vec<u64> = dp.bucket_counts().collect();
        let bounds_len = bounds.len();
        let (bucket, _) = bounds.iter().enumerate().fold(
            (Vec::with_capacity(bounds_len), 0),
            |(mut acc, mut count), (idx, bound)| {
                count += bucket_counts[idx];
                let mut b = prometheus::proto::Bucket::default();
                b.set_upper_bound(*bound);
                b.set_cumulative_count(count);
                acc.push(b);
                (acc, count)
            },
        );

        let mut histogram = prometheus::proto::Histogram::default();
        histogram.set_sample_sum(dp.sum().as_f64());
        histogram.set_sample_count(dp.count());
        histogram.set_bucket(bucket.into());

        let mut metric = prometheus::proto::Metric::default();
        metric.set_label(kvs.into());
        metric.set_histogram(histogram);

        let mut family = MetricFamily::default();
        family.set_name(name.to_string());
        family.set_help(description.clone());
        family.set_field_type(MetricType::HISTOGRAM);
        family.set_metric(vec![metric].into());
        res.push(family);
    }
}

fn add_sum_metric<T: Numeric + Copy>(
    res: &mut Vec<MetricFamily>,
    sum: &data::Sum<T>,
    description: String,
    extra: &[LabelPair],
    name: Cow<'static, str>,
) {
    let metric_type = if sum.is_monotonic() { MetricType::COUNTER } else { MetricType::GAUGE };

    for dp in sum.data_points() {
        let kvs = get_attrs(&mut dp.attributes().map(|kv| (&kv.key, &kv.value)), extra);

        let mut metric = prometheus::proto::Metric::default();
        metric.set_label(kvs.into());

        if sum.is_monotonic() {
            let mut counter = prometheus::proto::Counter::default();
            counter.set_value(dp.value().as_f64());
            metric.set_counter(counter);
        } else {
            let mut gauge = prometheus::proto::Gauge::default();
            gauge.set_value(dp.value().as_f64());
            metric.set_gauge(gauge);
        }

        let mut family = MetricFamily::default();
        family.set_name(name.to_string());
        family.set_help(description.clone());
        family.set_field_type(metric_type);
        family.set_metric(vec![metric].into());
        res.push(family);
    }
}

fn add_gauge_metric<T: Numeric + Copy>(
    res: &mut Vec<MetricFamily>,
    gauge: &data::Gauge<T>,
    description: String,
    extra: &[LabelPair],
    name: Cow<'static, str>,
) {
    for dp in gauge.data_points() {
        let kvs = get_attrs(&mut dp.attributes().map(|kv| (&kv.key, &kv.value)), extra);

        let mut metric = prometheus::proto::Metric::default();
        metric.set_label(kvs.into());

        let mut prom_gauge = prometheus::proto::Gauge::default();
        prom_gauge.set_value(dp.value().as_f64());
        metric.set_gauge(prom_gauge);

        let mut family = MetricFamily::default();
        family.set_name(name.to_string());
        family.set_help(description.to_string());
        family.set_field_type(MetricType::GAUGE);
        family.set_metric(vec![metric].into());
        res.push(family);
    }
}

fn create_info_metric(target_info_name: &str, target_info_description: &str, resource: &Resource) -> MetricFamily {
    let mut gauge = prometheus::proto::Gauge::default();
    gauge.set_value(1.0);

    let mut metric = prometheus::proto::Metric::default();
    metric.set_label(get_attrs(&mut resource.iter(), &[]).into());
    metric.set_gauge(gauge);

    let mut family = MetricFamily::default();
    family.set_name(target_info_name.into());
    family.set_help(target_info_description.into());
    family.set_field_type(MetricType::GAUGE);
    family.set_metric(vec![metric].into());
    family
}

fn create_scope_info_metric(scope: &InstrumentationScope) -> MetricFamily {
    let mut gauge = prometheus::proto::Gauge::default();
    gauge.set_value(1.0);

    let mut labels = Vec::with_capacity(1 + scope.version().is_some() as usize);
    let mut name = LabelPair::new();
    name.set_name(SCOPE_INFO_KEYS[0].into());
    name.set_value(scope.name().to_string());
    labels.push(name);
    if let Some(version) = scope.version() {
        let mut label = LabelPair::new();
        label.set_name(SCOPE_INFO_KEYS[1].into());
        label.set_value(version.to_string());
        labels.push(label);
    }

    let mut metric = prometheus::proto::Metric::default();
    metric.set_label(labels.into());
    metric.set_gauge(gauge);

    let mut family = MetricFamily::default();
    family.set_name(SCOPE_INFO_METRIC_NAME.into());
    family.set_help(SCOPE_INFO_DESCRIPTION.into());
    family.set_field_type(MetricType::GAUGE);
    family.set_metric(vec![metric].into());
    family
}

trait Numeric: fmt::Debug {
    fn as_f64(&self) -> f64;
}

impl Numeric for u64 {
    fn as_f64(&self) -> f64 {
        *self as f64
    }
}

impl Numeric for i64 {
    fn as_f64(&self) -> f64 {
        *self as f64
    }
}

impl Numeric for f64 {
    fn as_f64(&self) -> f64 {
        *self
    }
}

fn get_unit_suffixes(unit: &str) -> Option<String> {
    if unit.is_empty() {
        return None;
    }

    if let Some(matched) = get_prom_units(unit) {
        return Some(matched.to_string());
    }

    if let Some((first, second)) = unit.split_once('/') {
        return match (NON_APPLICABLE_ON_PER_UNIT.contains(&first), get_prom_units(first), get_prom_per_unit(second)) {
            (true, _, Some(second_part)) | (false, None, Some(second_part)) => Some(format!("per_{second_part}")),
            (false, Some(first_part), Some(second_part)) => Some(format!("{first_part}_per_{second_part}")),
            _ => None,
        };
    }

    None
}

const NON_APPLICABLE_ON_PER_UNIT: [&str; 8] = ["1", "d", "h", "min", "s", "ms", "us", "ns"];

fn get_prom_units(unit: &str) -> Option<&'static str> {
    match unit {
        "d" => Some("days"),
        "h" => Some("hours"),
        "min" => Some("minutes"),
        "s" => Some("seconds"),
        "ms" => Some("milliseconds"),
        "us" => Some("microseconds"),
        "ns" => Some("nanoseconds"),
        "By" => Some("bytes"),
        "KiBy" => Some("kibibytes"),
        "MiBy" => Some("mebibytes"),
        "GiBy" => Some("gibibytes"),
        "TiBy" => Some("tibibytes"),
        "KBy" => Some("kilobytes"),
        "MBy" => Some("megabytes"),
        "GBy" => Some("gigabytes"),
        "TBy" => Some("terabytes"),
        "B" => Some("bytes"),
        "KB" => Some("kilobytes"),
        "MB" => Some("megabytes"),
        "GB" => Some("gigabytes"),
        "TB" => Some("terabytes"),
        "m" => Some("meters"),
        "V" => Some("volts"),
        "A" => Some("amperes"),
        "J" => Some("joules"),
        "W" => Some("watts"),
        "g" => Some("grams"),
        "Cel" => Some("celsius"),
        "Hz" => Some("hertz"),
        "1" => Some("ratio"),
        "%" => Some("percent"),
        _ => None,
    }
}

fn get_prom_per_unit(unit: &str) -> Option<&'static str> {
    match unit {
        "s" => Some("second"),
        "m" => Some("minute"),
        "h" => Some("hour"),
        "d" => Some("day"),
        "w" => Some("week"),
        "mo" => Some("month"),
        "y" => Some("year"),
        _ => None,
    }
}

fn sanitize_name(s: &Cow<'static, str>) -> Cow<'static, str> {
    let mut prefix = "";
    if let Some((replace_idx, _)) = s.char_indices().find(|(i, c)| {
        if *i == 0 && c.is_ascii_digit() {
            prefix = "_";
            true
        } else {
            !c.is_alphanumeric() && *c != '_' && *c != ':'
        }
    }) {
        let (valid, rest) = s.split_at(replace_idx);
        Cow::Owned(
            prefix
                .chars()
                .chain(valid.chars())
                .chain(rest.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == ':' { c } else { '_' }))
                .collect(),
        )
    } else {
        s.clone()
    }
}

fn sanitize_prom_kv(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == ':' { c } else { '_' }).collect()
}
