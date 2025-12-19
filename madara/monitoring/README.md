# Madara Monitoring Stack

This directory contains configuration files for monitoring a Madara node using Prometheus and Grafana.

## Prerequisites

1. **Madara Node** running with Prometheus metrics enabled:
   ```bash
   cargo run --release -- --network <network> --analytics-prometheus-endpoint
   ```
   This exposes metrics at `http://localhost:9464/metrics`

2. **Docker & Docker Compose** (for local development)

3. **kubectl** (for Kubernetes deployment)

## Quick Start (Docker Compose)

```bash
# Start the monitoring stack
cd monitoring
docker-compose up -d

# Access the dashboards
# Grafana: http://localhost:3000 (admin/admin)
# Prometheus: http://localhost:9090
```

## Quick Start (Kubernetes)

```bash
cd monitoring/kubernetes

# Create namespace and deploy
kubectl apply -f namespace.yaml
kubectl apply -f prometheus-config.yaml
kubectl apply -f prometheus-deployment.yaml
kubectl apply -f grafana-config.yaml
kubectl apply -f grafana-deployment.yaml

# Port forward to access locally
kubectl port-forward -n madara-monitoring svc/grafana 3000:3000
kubectl port-forward -n madara-monitoring svc/prometheus 9090:9090
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

To monitor multiple Madara nodes, update `prometheus.yml`:

```yaml
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
├── docker-compose.yml              # Local development stack
├── prometheus.yml                  # Prometheus scrape configuration
├── README.md                       # This file
├── grafana/
│   ├── provisioning/
│   │   ├── datasources/
│   │   │   └── prometheus.yml      # Prometheus datasource config
│   │   └── dashboards/
│   │       └── dashboards.yml      # Dashboard provider config
│   └── dashboards/
│       ├── madara-overview.json    # Overview dashboard
│       ├── sync-performance.json   # Sync metrics dashboard
│       ├── block-production.json   # Sequencer dashboard
│       ├── rpc-performance.json    # RPC metrics dashboard
│       ├── storage-resources.json  # Storage dashboard
│       └── cairo-native.json       # Cairo Native dashboard
└── kubernetes/
    ├── namespace.yaml              # madara-monitoring namespace
    ├── prometheus-config.yaml      # Prometheus ConfigMap
    ├── prometheus-deployment.yaml  # Prometheus Deployment + Service + PVC
    ├── grafana-config.yaml         # Grafana ConfigMaps
    └── grafana-deployment.yaml     # Grafana Deployment + Service + PVC
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

## Troubleshooting

### Prometheus can't reach Madara

1. Verify Madara is running with `--analytics-prometheus-endpoint`
2. Check if metrics are accessible: `curl http://localhost:9464/metrics`
3. For Docker on macOS/Windows, use `host.docker.internal:9464` in prometheus.yml
4. For Docker on Linux, use `--network=host` or the host's IP address

### No data in Grafana dashboards

1. Check Prometheus targets: http://localhost:9090/targets
2. Verify the Prometheus datasource is configured correctly in Grafana
3. Check the time range in Grafana (metrics may not exist yet)

### Dashboard not loading

1. Ensure dashboard JSON files are properly mounted
2. Check Grafana logs: `docker-compose logs grafana`
3. Verify provisioning configs are correct

## Production Recommendations

1. **Change default credentials** - Update Grafana admin password
2. **Use persistent storage** - Configure proper PVCs in Kubernetes
3. **Set resource limits** - Adjust CPU/memory based on your cluster
4. **Enable TLS** - Configure HTTPS for external access
5. **Set up alerts** - Add Prometheus alerting rules for critical conditions
6. **Retention policy** - Configure appropriate data retention (default: 30 days)
