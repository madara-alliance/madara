# Madara Monitoring Stack

This directory contains configuration files for monitoring a Madara node using OpenTelemetry Collector, Prometheus, and Grafana.

## Architecture

```
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
         │  OpenTelemetry        │
         │  Collector            │
         │  - Receives metrics   │
         │  - Processes/batches  │
         │  - Exports to backends│
         │  (ports 4317/4318/    │
         │   8888/8889)          │
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

| Port  | Protocol | Description |
|-------|----------|-------------|
| 4317  | gRPC     | OTLP receiver |
| 4318  | HTTP     | OTLP receiver |
| 8888  | HTTP     | Collector internal metrics |
| 8889  | HTTP     | Prometheus exporter |
| 13133 | HTTP     | Health check endpoint |
| 55679 | HTTP     | zpages debugging |

### Configuration

The collector configuration (`otel-collector-config.yaml`) includes:

```yaml
receivers:
  otlp:           # OTLP protocol support
  prometheus:     # Scrape Madara metrics

processors:
  batch:          # Batches for efficiency
  memory_limiter: # Prevents OOM
  resource:       # Adds common attributes
  attributes:     # Metric transformations

exporters:
  prometheus:     # Expose for Prometheus scraping
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `OTEL_ENVIRONMENT` | `development` | Deployment environment label |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `localhost:4317` | OTLP backend endpoint |
| `OTEL_EXPORTER_OTLP_INSECURE` | `true` | Disable TLS for OTLP |

### Sending OTLP Metrics

To send metrics directly to the collector using OTLP:

```bash
# gRPC endpoint
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317

# HTTP endpoint
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

## Available Dashboards

### 1. Madara Overview
**UID:** `madara-overview`

Key metrics at a glance:
- L2 Block Number
- L1 Block Number
- Blocks Produced (sequencer)
- Database Size
- Request Rate
- L1 Gas Price
- Storage Usage

### 2. Sync Performance
**UID:** `madara-sync`

Full node sync metrics:
- Current L2 Block
- Transactions/Events Synced
- Block Sync Time (latest, average)
- Sync Throughput
- L1/L2 Gas Prices

### 3. Block Production
**UID:** `madara-block-production`

Sequencer metrics:
- Latest Block Number
- Blocks Produced Count
- Transaction Counter
- Mempool Size
- Block Production Rate
- Transaction Throughput

### 4. RPC Performance
**UID:** `madara-rpc`

RPC server metrics:
- Total Calls
- Requests Per Second
- Active WebSocket Sessions
- Latency Percentiles (p50, p90, p95, p99)
- In-Flight Requests

### 5. Storage & Resources
**UID:** `madara-storage`

Database and resource metrics:
- Total Database Size
- MemTable Memory Usage
- Block Cache Size
- Column Family Sizes
- Database Growth Rate

### 6. Cairo Native
**UID:** `madara-cairo-native`

Cairo Native execution metrics:
- Cache Size
- Cache Hit Rate (Memory/Disk)
- Compilation Rate (Success/Failure)
- Compilation Time Percentiles
- VM Fallbacks
- Cache Operation Latency

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

### Environment Variables

```bash
MADARA_ANALYTICS_PROMETHEUS_ENDPOINT=true
MADARA_ANALYTICS_PROMETHEUS_ENDPOINT_EXTERNAL=true
MADARA_ANALYTICS_PROMETHEUS_ENDPOINT_PORT=9464
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

**Running two Madara instances:**

```bash
# Terminal 1 - Sequencer (port 9464)
cargo run --release -- \
  --network devnet \
  --sequencer \
  --base-path /tmp/madara-sequencer \
  --analytics-prometheus-endpoint 0.0.0.0:9464

# Terminal 2 - Full Node (port 9465)
cargo run --release -- \
  --network sepolia \
  --base-path /tmp/madara-fullnode \
  --analytics-prometheus-endpoint 0.0.0.0:9465
```

## Directory Structure

```
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
        ├── madara-overview.json    # Overview dashboard
        ├── sync-performance.json   # Sync metrics dashboard
        ├── block-production.json   # Sequencer dashboard
        ├── rpc-performance.json    # RPC metrics dashboard
        ├── storage-resources.json  # Storage dashboard
        └── cairo-native.json       # Cairo Native dashboard
```

## Metrics Reference

### Sync Service
| Metric | Type | Description |
|--------|------|-------------|
| `l2_block_number` | Histogram | Current L2 block number |
| `l2_sync_time` | Histogram | Total sync time |
| `l2_latest_sync_time` | Histogram | Time to sync latest block |
| `l2_avg_sync_time` | Histogram | Average sync time per block |
| `transaction_count` | Counter | Total transactions synced |
| `event_count` | Counter | Total events synced |
| `l1_gas_price_wei` | Histogram | L1 gas price in Wei |
| `l1_gas_price_strk` | Histogram | L1 gas price in STRK |

### Block Production
| Metric | Type | Description |
|--------|------|-------------|
| `block_produced_no` | Gauge | Current block number |
| `block_produced_count` | Counter | Total blocks produced |
| `transaction_counter` | Counter | Total transactions included |

### Mempool
| Metric | Type | Description |
|--------|------|-------------|
| `accepted_transaction_count` | Counter | Transactions in mempool |

### RPC
| Metric | Type | Description |
|--------|------|-------------|
| `calls_started` | Counter | RPC calls started |
| `calls_finished` | Counter | RPC calls completed |
| `calls_time` | Histogram | RPC call duration (ms) |
| `ws_sessions_opened` | Counter | WebSocket sessions opened |
| `ws_sessions_closed` | Counter | WebSocket sessions closed |
| `ws_sessions_time` | Histogram | WebSocket session duration |

### Database
| Metric | Type | Description |
|--------|------|-------------|
| `db_size` | Gauge | Total database size (bytes) |
| `column_sizes` | Gauge | Per-column sizes (bytes) |
| `db_mem_table_total` | Gauge | MemTable memory usage |
| `db_mem_table_unflushed` | Gauge | Unflushed MemTable size |
| `db_mem_table_readers_total` | Gauge | Table readers memory |
| `db_cache_total` | Gauge | Block cache size |

### L1 Settlement
| Metric | Type | Description |
|--------|------|-------------|
| `l1_block_number` | Gauge | Current L1 block |
| `l1_gas_price_wei` | Gauge | L1 gas price (Wei) |
| `l1_gas_price_strk` | Gauge | L1 gas price (STRK) |

### Cairo Native
| Metric | Type | Description |
|--------|------|-------------|
| `cairo_native_cache_size` | Gauge | Classes in memory cache |
| `cairo_native_cache_hits_memory` | Counter | Memory cache hits |
| `cairo_native_cache_hits_disk` | Counter | Disk cache hits |
| `cairo_native_cache_memory_miss` | Counter | Memory cache misses |
| `cairo_native_cache_disk_miss` | Counter | Disk cache misses |
| `cairo_native_cache_evictions` | Counter | Cache evictions |
| `cairo_native_compilations_started` | Counter | Compilations started |
| `cairo_native_compilations_succeeded` | Counter | Successful compilations |
| `cairo_native_compilations_failed` | Counter | Failed compilations |
| `cairo_native_compilations_timeout` | Counter | Compilation timeouts |
| `cairo_native_compilation_time` | Histogram | Compilation duration (ms) |
| `cairo_native_current_compilations` | Gauge | Active compilations |
| `cairo_native_vm_fallbacks` | Counter | VM fallback count |

### OpenTelemetry Collector
| Metric | Type | Description |
|--------|------|-------------|
| `otelcol_receiver_accepted_metric_points` | Counter | Metric points accepted |
| `otelcol_receiver_refused_metric_points` | Counter | Metric points refused |
| `otelcol_exporter_sent_metric_points` | Counter | Metric points exported |
| `otelcol_processor_batch_batch_send_size` | Histogram | Batch sizes |
| `otelcol_process_memory_rss` | Gauge | Collector memory usage |

## Troubleshooting

### OTel Collector Issues

1. **Health check endpoint**: `curl http://localhost:13133/health`
2. **View internal metrics**: `curl http://localhost:8888/metrics`
3. **zpages debugging**: Open `http://localhost:55679/debug/tracez`
4. **Check logs**: `docker-compose logs otel-collector`

### Prometheus can't reach OTel Collector

1. Verify collector is running: `docker-compose ps otel-collector`
2. Check collector health: `curl http://localhost:13133/health`
3. Verify Prometheus targets: http://localhost:9090/targets
4. Check network connectivity between containers

### OTel Collector can't reach Madara

1. Verify Madara is running with `--analytics-prometheus-endpoint`
2. Check if metrics are accessible: `curl http://localhost:9464/metrics`
3. For Docker on macOS/Windows, use `host.docker.internal:9464`
4. For Docker on Linux, use `--network=host` or the host's IP address

### No data in Grafana dashboards

1. Check OTel Collector targets in collector logs
2. Check Prometheus targets: http://localhost:9090/targets
3. Verify the Prometheus datasource is configured correctly in Grafana
4. Check the time range in Grafana (metrics may not exist yet)

### Dashboard not loading

1. Ensure dashboard JSON files are properly mounted
2. Check Grafana logs: `docker-compose logs grafana`
3. Verify provisioning configs are correct

## Production Recommendations

1. **Change default credentials** - Update Grafana admin password
2. **Use persistent storage** - Docker volumes are configured by default
3. **Set resource limits** - Add resource constraints in docker-compose.yml
4. **Enable TLS** - Configure HTTPS for external access
5. **Set up alerts** - Add Prometheus alerting rules for critical conditions
6. **Retention policy** - Configure appropriate data retention (default: 30 days)
7. **Scale OTel Collector** - Use multiple replicas with a load balancer for high availability
8. **Enable OTLP export** - Configure additional backends (Jaeger, Tempo, etc.) for traces
