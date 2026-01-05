# Madara Monitoring Stack

This directory contains configuration files for monitoring a Madara node using OpenTelemetry Collector, Prometheus, and Grafana.

## Architecture

```text
┌─────────────────┐     ┌─────────────────┐
│ Madara Sequencer│     │ Madara Fullnode │
│                 │     │                 │
└────────┬────────┘     └────────┬────────┘
         │                       │
         │      OTLP (gRPC)      │
         │                       │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  OTel Collector       │
         │  (port 4317 gRPC)     │
         │  - Receives OTLP      │
         │  - Exports Prometheus │
         └───────────┬───────────┘
                     │
                     │  Prometheus scrape
                     │  (port 8889)
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
cd observability
docker-compose up -d

# Access the services
# Grafana: http://localhost:3000 (admin/admin)
# Prometheus: http://localhost:9090
# OTel Collector Prometheus exporter: http://localhost:8889/metrics
```

## OpenTelemetry Collector

The OpenTelemetry Collector acts as a central hub for collecting and exporting telemetry data.

### Features

- **Receivers**: Accepts metrics via OTLP (gRPC and HTTP)
- **Exporters**: Prometheus format for Grafana dashboards

### Ports

| Port | Protocol | Description         |
| ---- | -------- | ------------------- |
| 4317 | gRPC     | OTLP receiver       |
| 4318 | HTTP     | OTLP receiver       |
| 8889 | HTTP     | Prometheus exporter |

## Dashboards

The monitoring stack includes 6 dashboards organized in two folders.

### Madara Dashboards

#### 1. Madara Overview

**UID:** `madara-overview`

Comprehensive view of block production and storage:

- **Block Production**: Chain height, blocks produced, total transactions, TPS, mempool size
- **Database & Storage**: DB size, MemTable, block cache, column family sizes, growth rate

#### 2. Madara Fullnode

**UID:** `madara-fullnode`

Fullnode-specific metrics and performance data.

### Orchestrator Dashboards

The stack also includes 4 Orchestrator-specific dashboards:

- `grafana_orchestrator_full_view.json` - Full orchestrator view
- `grafana_orchestrator_v1.json` - Orchestrator v1 dashboard
- `grafana_orchestrator_v2_fixed.json` - Orchestrator v2 dashboard
- `orchestrator_dashboard_v1.json` - Orchestrator dashboard v1

## Stack Components

### OpenTelemetry Collector Details

- **Image**: `otel/opentelemetry-collector:0.142.0`
- **Container**: `madara-otel-collector`
- Receives metrics via OTLP (gRPC and HTTP)
- Exports metrics in Prometheus format

### Prometheus Details

- **Image**: `prom/prometheus:v3.1.0`
- **Container**: `madara-prometheus`
- Scrapes metrics from OTel Collector's Prometheus exporter
- Includes self-monitoring job
- Data retention: 30 days (configurable)

### Grafana Details

- **Image**: `grafana/grafana:12.3.0`
- **Container**: `madara-grafana`
- Default credentials: `admin/admin`
- Anonymous read-only access enabled
- Embedding in iframes enabled
- Default home dashboard: Madara Overview

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

Multiple Madara nodes can send metrics to the same OTel Collector via OTLP. Configure each node with appropriate service name labels using the `--analytics-service-name` CLI option to distinguish them in dashboards.

### Prometheus Scrape Configuration

Prometheus scrapes metrics from:

- **OTel Collector**: `host.docker.internal:8889` (Prometheus exporter endpoint)
- **Prometheus self-monitoring**: `localhost:9090`

The scrape interval is set to 15 seconds for both targets.

## Directory Structure

```text
observability/
├── docker-compose.yml              # Docker Compose stack
├── otel-collector-config.yaml      # OpenTelemetry Collector configuration
├── prometheus.yml                  # Prometheus scrape configuration
├── README.md                       # This file
└── grafana/
    ├── provisioning/
    │   ├── datasources/
    │   │   └── datasource.yml      # Prometheus datasource config
    │   └── dashboards/
    │       └── dashboards.yml      # Dashboard provider config
    ├── dashboards/
    │   ├── Madara/
    │   │   ├── madara-overview.json    # Block production & storage
    │   │   └── madara-fullnode.json    # Fullnode metrics
    │   └── Orchestrator/
    │       ├── grafana_orchestrator_full_view.json
    │       ├── grafana_orchestrator_v1.json
    │       ├── grafana_orchestrator_v2_fixed.json
    │       └── orchestrator_dashboard_v1.json
    └── alerts/
        └── Orchestrator/
            ├── advanced_alerts.yaml
            ├── notification_channels.yaml
            ├── orchestrator_alerts.yaml
            └── README.md
```

## Metrics Reference

### Block Production

| Metric                       | Type    | Description           |
| ---------------------------- | ------- | --------------------- |
| `block_produced_no`          | Gauge   | Current block number  |
| `block_produced_count`       | Counter | Total blocks produced |
| `transaction_counter`        | Counter | Total transactions    |
| `accepted_transaction_count` | Counter | Mempool transactions  |

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

| Metric               | Type      | Description               |
| -------------------- | --------- | ------------------------- |
| `calls_started`      | Counter   | RPC calls started         |
| `calls_finished`     | Counter   | RPC calls completed       |
| `calls_time`         | Histogram | RPC call duration (ms)    |
| `ws_sessions_opened` | Counter   | WebSocket sessions opened |
| `ws_sessions_closed` | Counter   | WebSocket sessions closed |

### Database

| Metric                       | Type  | Description             |
| ---------------------------- | ----- | ----------------------- |
| `db_size`                    | Gauge | Total database size     |
| `column_sizes`               | Gauge | Per-column sizes        |
| `db_mem_table_total`         | Gauge | MemTable memory usage   |
| `db_mem_table_unflushed`     | Gauge | Unflushed MemTable size |
| `db_mem_table_readers_total` | Gauge | Table readers memory    |
| `db_cache_total`             | Gauge | Block cache size        |

### L1 Settlement

| Metric              | Type      | Description         |
| ------------------- | --------- | ------------------- |
| `l1_block_number`   | Gauge     | Current L1 block    |
| `l1_gas_price_wei`  | Histogram | L1 gas price (Wei)  |
| `l1_gas_price_strk` | Histogram | L1 gas price (STRK) |

### Cairo Native

| Metric                                | Type      | Description             |
| ------------------------------------- | --------- | ----------------------- |
| `cairo_native_cache_size`             | Gauge     | Classes in memory cache |
| `cairo_native_cache_hits_memory`      | Counter   | Memory cache hits       |
| `cairo_native_cache_hits_disk`        | Counter   | Disk cache hits         |
| `cairo_native_cache_evictions`        | Counter   | Cache evictions         |
| `cairo_native_compilations_started`   | Counter   | Compilations started    |
| `cairo_native_compilations_succeeded` | Counter   | Successful compilations |
| `cairo_native_compilations_failed`    | Counter   | Failed compilations     |
| `cairo_native_compilation_time`       | Histogram | Compilation duration    |
| `cairo_native_vm_fallbacks`           | Counter   | VM fallback count       |

## Troubleshooting

### OTel Collector Issues

1. **Check logs**: `docker-compose logs otel-collector`
2. **Verify Prometheus exporter**: `curl http://localhost:8889/metrics`

### Prometheus can't reach OTel Collector

1. Verify collector is running: `docker-compose ps otel-collector`
2. Verify Prometheus targets: `http://localhost:9090/targets`
3. Check network connectivity between containers
4. Note: Prometheus scrapes from `host.docker.internal:8889` - ensure the collector's Prometheus exporter is accessible at this address

### No data in Grafana dashboards

1. Check OTel Collector targets in collector logs
2. Check Prometheus targets: `http://localhost:9090/targets`
3. Verify the Prometheus datasource is configured correctly in Grafana
4. Check the time range in Grafana (metrics may not exist yet)

## Production Recommendations

1. **Change default credentials** - Update Grafana admin password (currently `admin/admin`)
2. **Use persistent storage** - Docker volumes (`prometheus-data` and `grafana-data`) are configured by default
3. **Set resource limits** - Add resource constraints in docker-compose.yml
4. **Enable TLS** - Configure HTTPS for external access
5. **Set up alerts** - Add Prometheus alerting rules for critical conditions (see `grafana/alerts/` directory for Orchestrator examples)
6. **Retention policy** - Configure appropriate data retention (default: 30 days, set via `--storage.tsdb.retention.time`)
7. **Disable anonymous access** - If not needed, remove `GF_AUTH_ANONYMOUS_ENABLED` from Grafana environment variables
8. **Review embedding settings** - Disable `GF_SECURITY_ALLOW_EMBEDDING` if not required
