# Madara Monitoring Stack

This directory contains configuration files for monitoring a Madara node using OpenTelemetry Collector, Prometheus, and Grafana.

## Architecture

```text
┌─────────────────┐     ┌─────────────────┐
│ Madara Sequencer│     │ Madara Fullnode │
│   (port 9464)   │     │   (port 9465)   │
└────────┬────────┘     └────────┬────────┘
         │                       │
         │  Prometheus metrics   │
         │                       │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Prometheus           │
         │  (port 9090)          │
         │  - Time series DB     │
         │  - Scrapes collector  │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Grafana              │
         │  (port 3000)          │
         │  - Dashboards         │
         │  - Visualization      │
         └───────────────────────┘
```

## Prerequisites

1. **Madara Node** running with Prometheus metrics enabled:

   ```bash
   cargo run --release -- --network <network> --analytics-prometheus-endpoint
   ```

   This exposes metrics at `http://localhost:9464/metrics`

2. **Docker & Docker Compose**

## Quick Start

```bash
# Start the monitoring stack
cd monitoring
docker-compose up -d

# Access the services
# Grafana: http://localhost:3000 (admin/admin)
# Prometheus: http://localhost:9090
# OTel Collector metrics: http://localhost:8888/metrics
# OTel Collector health: http://localhost:13133/health
```

## OpenTelemetry Collector

The OpenTelemetry Collector acts as a central hub for collecting, processing, and exporting telemetry data.

### Features

- **Receivers**: Accepts metrics via OTLP (gRPC/HTTP) and Prometheus scraping
- **Processors**: Batching, memory limiting, resource attribution
- **Exporters**: Prometheus format for Grafana dashboards
- **Extensions**: Health check, pprof profiling, zpages debugging

### Ports

| Port  | Protocol | Description                |
| ----- | -------- | -------------------------- |
| 4317  | gRPC     | OTLP receiver              |
| 4318  | HTTP     | OTLP receiver              |
| 8888  | HTTP     | Collector internal metrics |
| 8889  | HTTP     | Prometheus exporter        |
| 13133 | HTTP     | Health check endpoint      |
| 55679 | HTTP     | zpages debugging           |

### Environment Variables

| Variable                      | Default          | Description               |
| ----------------------------- | ---------------- | ------------------------- |
| `OTEL_ENVIRONMENT`            | `development`    | Deployment environment    |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `localhost:4317` | OTLP backend endpoint     |
| `OTEL_EXPORTER_OTLP_INSECURE` | `true`           | Disable TLS for OTLP      |

## Dashboards

The monitoring stack includes 3 consolidated dashboards with navigation links between them and instance selector for multi-node monitoring.

### 1. Madara Overview

**UID:** `madara-overview`

Comprehensive view of block production and storage:

- **Block Production**: Chain height, blocks produced, total transactions, TPS, mempool size
- **Database & Storage**: DB size, MemTable, block cache, column family sizes, growth rate

### 2. Madara Network

**UID:** `madara-network`

RPC and sync performance metrics:

- **RPC Overview**: Total calls, RPS, WebSocket sessions, latency percentiles (p50, p99), in-flight requests
- **RPC Success/Error Rate**: Success and error rate visualization over time
- **Top RPC Methods**: Breakdown of most called RPC methods
- **Sync Progress**: L2 block, transactions/events synced, L1 block, gas prices
- **WebSocket & Gas**: Active sessions, L1 gas prices (Wei/STRK)

### 3. Madara Cairo Native

**UID:** `madara-cairo-native`

Cairo Native execution metrics:

- **Cache Overview**: Cache size, memory/disk hits, evictions, VM fallbacks, active compilations
- **Cache Hit Rate**: Memory and disk hit rates, cache operations rate
- **Compilation**: Compilation rate, compilation time percentiles
- **Cache Errors & Disk I/O**: Cache error rates (timeouts, load errors, file errors), disk I/O latency

## Configuration

### Madara CLI Options

```bash
# Enable Prometheus endpoint (required)
--analytics-prometheus-endpoint

# Listen on external interface (0.0.0.0 instead of localhost)
--analytics-prometheus-endpoint-external

# Custom port (default: 9464)
--analytics-prometheus-endpoint-port <PORT>

# Service name for metrics (default: madara_analytics)
--analytics-service-name <NAME>
```

### Multi-Node Monitoring

To monitor multiple Madara nodes, update `otel-collector-config.yaml`:

```yaml
receivers:
  prometheus:
    config:
      scrape_configs:
        - job_name: 'madara-sequencer'
          static_configs:
            - targets: ['host.docker.internal:9464']
              labels:
                instance: 'sequencer'

        - job_name: 'madara-fullnode'
          static_configs:
            - targets: ['host.docker.internal:9465']
              labels:
                instance: 'fullnode'
```

## Directory Structure

```text
monitoring/
├── docker-compose.yml              # Docker Compose stack
├── otel-collector-config.yaml      # OpenTelemetry Collector configuration
├── prometheus.yml                  # Prometheus scrape configuration
├── README.md                       # This file
└── grafana/
    ├── provisioning/
    │   ├── datasources/
    │   │   └── prometheus.yml      # Prometheus datasource config
    │   └── dashboards/
    │       └── dashboards.yml      # Dashboard provider config
    └── dashboards/
        ├── madara-overview.json    # Block production & storage
        ├── madara-network.json     # RPC & sync performance
        └── cairo-native.json       # Cairo Native execution
```

## Metrics Reference

### Block Production

| Metric                           | Type    | Description              |
| -------------------------------- | ------- | ------------------------ |
| `block_produced_no`              | Gauge   | Current block number     |
| `block_produced_count`           | Counter | Total blocks produced    |
| `transaction_counter`            | Counter | Total transactions       |
| `accepted_transaction_count`     | Counter | Mempool transactions     |

### Sync Service

| Metric                | Type      | Description             |
| --------------------- | --------- | ----------------------- |
| `l2_block_number`     | Histogram | Current L2 block number |
| `l2_sync_time`        | Histogram | Total sync time         |
| `l2_latest_sync_time` | Histogram | Latest block sync time  |
| `l2_avg_sync_time`    | Histogram | Average sync time       |
| `transaction_count`   | Counter   | Transactions synced     |
| `event_count`         | Counter   | Events synced           |

### RPC

| Metric               | Type      | Description              |
| -------------------- | --------- | ------------------------ |
| `calls_started`      | Counter   | RPC calls started        |
| `calls_finished`     | Counter   | RPC calls completed      |
| `calls_time`         | Histogram | RPC call duration (ms)   |
| `ws_sessions_opened` | Counter   | WebSocket sessions opened|
| `ws_sessions_closed` | Counter   | WebSocket sessions closed|

### Database

| Metric                     | Type  | Description             |
| -------------------------- | ----- | ----------------------- |
| `db_size`                  | Gauge | Total database size     |
| `column_sizes`             | Gauge | Per-column sizes        |
| `db_mem_table_total`       | Gauge | MemTable memory usage   |
| `db_mem_table_unflushed`   | Gauge | Unflushed MemTable size |
| `db_mem_table_readers_total`| Gauge | Table readers memory   |
| `db_cache_total`           | Gauge | Block cache size        |

### L1 Settlement

| Metric             | Type      | Description        |
| ------------------ | --------- | ------------------ |
| `l1_block_number`  | Gauge     | Current L1 block   |
| `l1_gas_price_wei` | Histogram | L1 gas price (Wei) |
| `l1_gas_price_strk`| Histogram | L1 gas price (STRK)|

### Cairo Native

| Metric                             | Type      | Description            |
| ---------------------------------- | --------- | ---------------------- |
| `cairo_native_cache_size`          | Gauge     | Classes in memory cache|
| `cairo_native_cache_hits_memory`   | Counter   | Memory cache hits      |
| `cairo_native_cache_hits_disk`     | Counter   | Disk cache hits        |
| `cairo_native_cache_evictions`     | Counter   | Cache evictions        |
| `cairo_native_compilations_started`| Counter   | Compilations started   |
| `cairo_native_compilations_succeeded`| Counter | Successful compilations|
| `cairo_native_compilations_failed` | Counter   | Failed compilations    |
| `cairo_native_compilation_time`    | Histogram | Compilation duration   |
| `cairo_native_vm_fallbacks`        | Counter   | VM fallback count      |

## Troubleshooting

### OTel Collector Issues

1. **Health check endpoint**: `curl http://localhost:13133/health`
2. **View internal metrics**: `curl http://localhost:8888/metrics`
3. **zpages debugging**: Open `http://localhost:55679/debug/tracez`
4. **Check logs**: `docker-compose logs otel-collector`

### Prometheus can't reach OTel Collector

1. Verify collector is running: `docker-compose ps otel-collector`
2. Check collector health: `curl http://localhost:13133/health`
3. Verify Prometheus targets: `http://localhost:9090/targets`
4. Check network connectivity between containers

### No data in Grafana dashboards

1. Check OTel Collector targets in collector logs
2. Check Prometheus targets: `http://localhost:9090/targets`
3. Verify the Prometheus datasource is configured correctly in Grafana
4. Check the time range in Grafana (metrics may not exist yet)

## Production Recommendations

1. **Change default credentials** - Update Grafana admin password
2. **Use persistent storage** - Docker volumes are configured by default
3. **Set resource limits** - Add resource constraints in docker-compose.yml
4. **Enable TLS** - Configure HTTPS for external access
5. **Set up alerts** - Add Prometheus alerting rules for critical conditions
6. **Retention policy** - Configure appropriate data retention (default: 30 days)
