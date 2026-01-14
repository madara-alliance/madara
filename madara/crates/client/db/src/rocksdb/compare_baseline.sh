#!/bin/bash
#
# RocksDB Baseline Comparison Script
# Compares current metrics against baseline values from BASELINE_METRICS.md
#
# Usage: ./compare_baseline.sh [--pod POD_NAME] [--namespace NAMESPACE] [--hours HOURS]
#
# Example:
#   ./compare_baseline.sh
#   ./compare_baseline.sh --pod paradex-mainnet-madara-dev-0 --namespace madara-full-node --hours 12
#

set -e

# ═══════════════════════════════════════════════════════════════════════════════
# CONFIGURATION
# ═══════════════════════════════════════════════════════════════════════════════

VM_SELECT_URL="${VM_SELECT_URL:-https://vmselect-central.karnot.xyz}"
POD_NAME="${POD_NAME:-paradex-mainnet-madara-dev-0}"
NAMESPACE="${NAMESPACE:-madara-full-node}"
HOURS="${HOURS:-12}"

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --pod) POD_NAME="$2"; shift 2 ;;
        --namespace) NAMESPACE="$2"; shift 2 ;;
        --hours) HOURS="$2"; shift 2 ;;
        --vm-url) VM_SELECT_URL="$2"; shift 2 ;;
        -h|--help)
            echo "Usage: $0 [--pod POD_NAME] [--namespace NAMESPACE] [--hours HOURS] [--vm-url URL]"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ═══════════════════════════════════════════════════════════════════════════════
# BASELINE VALUES (from BASELINE_METRICS.md - 2026-01-14)
# ═══════════════════════════════════════════════════════════════════════════════

# Write Performance
BASELINE_CLOSE_BLOCK_P50=5.02
BASELINE_CLOSE_BLOCK_P95=9.53
BASELINE_CLOSE_BLOCK_P99=9.93
BASELINE_CLOSE_BLOCK_AVG=2.04

# Block Operation Breakdown (milliseconds)
BASELINE_APPLY_TRIE_AVG=1844
BASELINE_APPLY_TRIE_P95=4044
BASELINE_CONTRACT_STORAGE_TRIE_AVG=467
BASELINE_DB_WRITE_PARTS_AVG=12

# RocksDB Internals
BASELINE_L0_FILES_AVG=29
BASELINE_L0_FILES_MAX=146
BASELINE_IMMUTABLE_MEMTABLES_AVG=3.24
BASELINE_IMMUTABLE_MEMTABLES_MAX=19
BASELINE_PENDING_COMPACTION_MAX=0

# Memory (GiB)
BASELINE_TABLE_READERS_AVG=1.89
BASELINE_CONTAINER_MEMORY_AVG=6.03

# Throughput
BASELINE_BLOCKS_PER_HOUR=217.75

# I/O (MiB/s)
BASELINE_DISK_WRITE_AVG=7.75
BASELINE_DISK_READ_AVG=9.87

# ═══════════════════════════════════════════════════════════════════════════════
# HELPER FUNCTIONS
# ═══════════════════════════════════════════════════════════════════════════════

query_instant() {
    local query="$1"
    curl -s --max-time 30 "${VM_SELECT_URL}/select/0/prometheus/api/v1/query" \
        --data-urlencode "query=${query}" | jq -r '.data.result[0].value[1] // "N/A"' 2>/dev/null
}

query_range_stat() {
    local query="$1"
    local stat="$2"  # min, max, avg, p50, p95
    local end=$(date +%s)
    local start=$((end - HOURS * 3600))

    local jq_expr
    case $stat in
        min) jq_expr='map(.[1] | tonumber) | min' ;;
        max) jq_expr='map(.[1] | tonumber) | max' ;;
        avg) jq_expr='map(.[1] | tonumber) | add/length' ;;
        p50) jq_expr='map(.[1] | tonumber) | sort | .[length/2 | floor]' ;;
        p95) jq_expr='map(.[1] | tonumber) | sort | .[(length*0.95) | floor]' ;;
        *) echo "Unknown stat: $stat"; return 1 ;;
    esac

    curl -s --max-time 30 "${VM_SELECT_URL}/select/0/prometheus/api/v1/query_range" \
        --data-urlencode "query=${query}" \
        --data-urlencode "start=${start}" \
        --data-urlencode "end=${end}" \
        --data-urlencode "step=300" | jq -r ".data.result[0].values | ${jq_expr}" 2>/dev/null
}

query_histogram_quantile() {
    local metric="$1"
    local quantile="$2"
    query_instant "histogram_quantile(${quantile}, rate(${metric}_bucket{pod=\"${POD_NAME}\"}[${HOURS}h]))"
}

compare_values() {
    local name="$1"
    local current="$2"
    local baseline="$3"
    local lower_is_better="${4:-true}"
    local tolerance="${5:-0.1}"  # 10% tolerance

    if [[ "$current" == "N/A" || -z "$current" ]]; then
        printf "| %-35s | %12s | %12s | ⚠️  No data |\n" "$name" "N/A" "$baseline"
        return
    fi

    # Calculate percentage change
    local pct_change=$(echo "scale=2; (($current - $baseline) / $baseline) * 100" | bc 2>/dev/null || echo "0")

    # Determine status
    local status
    local threshold=$(echo "scale=2; $tolerance * 100" | bc)

    if [[ "$lower_is_better" == "true" ]]; then
        if (( $(echo "$pct_change < -$threshold" | bc -l) )); then
            status="✅ Improved"
        elif (( $(echo "$pct_change > $threshold" | bc -l) )); then
            status="❌ Regressed"
        else
            status="➖ Similar"
        fi
    else
        if (( $(echo "$pct_change > $threshold" | bc -l) )); then
            status="✅ Improved"
        elif (( $(echo "$pct_change < -$threshold" | bc -l) )); then
            status="❌ Regressed"
        else
            status="➖ Similar"
        fi
    fi

    printf "| %-35s | %12s | %12s | %+.1f%% %s |\n" "$name" "$current" "$baseline" "$pct_change" "$status"
}

format_seconds() {
    printf "%.2f" "$1"
}

format_ms() {
    local val="$1"
    if [[ "$val" != "N/A" && -n "$val" ]]; then
        printf "%.0f" "$(echo "$val * 1000" | bc)"
    else
        echo "N/A"
    fi
}

format_gib() {
    local val="$1"
    if [[ "$val" != "N/A" && -n "$val" ]]; then
        printf "%.2f" "$(echo "$val / 1073741824" | bc -l)"
    else
        echo "N/A"
    fi
}

format_mib() {
    local val="$1"
    if [[ "$val" != "N/A" && -n "$val" ]]; then
        printf "%.0f" "$(echo "$val / 1048576" | bc -l)"
    else
        echo "N/A"
    fi
}

# ═══════════════════════════════════════════════════════════════════════════════
# MAIN SCRIPT
# ═══════════════════════════════════════════════════════════════════════════════

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  RocksDB Performance Comparison Report"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
echo "  Pod:        ${POD_NAME}"
echo "  Namespace:  ${NAMESPACE}"
echo "  Time Range: Last ${HOURS} hours"
echo "  Generated:  $(date)"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 1. WRITE STALL CHECK (Critical)
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  1. WRITE STALL CHECK (Critical)"
echo "═══════════════════════════════════════════════════════════════════════════════"

write_stopped_max=$(query_range_stat "db_is_write_stopped{pod=\"${POD_NAME}\"}" "max")
if [[ "$write_stopped_max" == "0" || "$write_stopped_max" == "null" ]]; then
    echo "  ✅ No write stalls detected (db_is_write_stopped = 0)"
else
    echo "  ❌ WRITE STALLS DETECTED! (db_is_write_stopped max = ${write_stopped_max})"
fi
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 2. WRITE PERFORMANCE
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  2. WRITE PERFORMANCE (Close Block Duration)"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

close_p50=$(query_histogram_quantile "close_block_total_duration_seconds" "0.50")
close_p95=$(query_histogram_quantile "close_block_total_duration_seconds" "0.95")
close_p99=$(query_histogram_quantile "close_block_total_duration_seconds" "0.99")
close_avg=$(query_instant "rate(close_block_total_duration_seconds_sum{pod=\"${POD_NAME}\"}[${HOURS}h]) / rate(close_block_total_duration_seconds_count{pod=\"${POD_NAME}\"}[${HOURS}h])")

compare_values "Close Block p50 (sec)" "$(format_seconds $close_p50)" "$BASELINE_CLOSE_BLOCK_P50" "true"
compare_values "Close Block p95 (sec)" "$(format_seconds $close_p95)" "$BASELINE_CLOSE_BLOCK_P95" "true"
compare_values "Close Block p99 (sec)" "$(format_seconds $close_p99)" "$BASELINE_CLOSE_BLOCK_P99" "true"
compare_values "Close Block avg (sec)" "$(format_seconds $close_avg)" "$BASELINE_CLOSE_BLOCK_AVG" "true"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 3. BLOCK OPERATION BREAKDOWN
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  3. BLOCK OPERATION BREAKDOWN"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

apply_trie_avg=$(query_range_stat "apply_to_global_trie_last_seconds{pod=\"${POD_NAME}\"}" "avg")
apply_trie_p95=$(query_range_stat "apply_to_global_trie_last_seconds{pod=\"${POD_NAME}\"}" "p95")
contract_storage_avg=$(query_range_stat "contract_storage_trie_commit_last_seconds{pod=\"${POD_NAME}\"}" "avg")
db_write_avg=$(query_range_stat "db_write_block_parts_last_seconds{pod=\"${POD_NAME}\"}" "avg")

compare_values "apply_to_global_trie avg (ms)" "$(format_ms $apply_trie_avg)" "$BASELINE_APPLY_TRIE_AVG" "true"
compare_values "apply_to_global_trie p95 (ms)" "$(format_ms $apply_trie_p95)" "$BASELINE_APPLY_TRIE_P95" "true"
compare_values "contract_storage_trie avg (ms)" "$(format_ms $contract_storage_avg)" "$BASELINE_CONTRACT_STORAGE_TRIE_AVG" "true"
compare_values "db_write_block_parts avg (ms)" "$(format_ms $db_write_avg)" "$BASELINE_DB_WRITE_PARTS_AVG" "true"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 4. ROCKSDB INTERNALS
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  4. ROCKSDB INTERNALS"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

l0_avg=$(query_range_stat "db_level_files_count{pod=\"${POD_NAME}\",level=\"L0\"}" "avg")
l0_max=$(query_range_stat "db_level_files_count{pod=\"${POD_NAME}\",level=\"L0\"}" "max")
immut_avg=$(query_range_stat "db_num_immutable_memtables{pod=\"${POD_NAME}\"}" "avg")
immut_max=$(query_range_stat "db_num_immutable_memtables{pod=\"${POD_NAME}\"}" "max")
pending_max=$(query_range_stat "db_pending_compaction_bytes{pod=\"${POD_NAME}\"}" "max")

compare_values "L0 Files (aggregated) avg" "$(printf '%.0f' $l0_avg)" "$BASELINE_L0_FILES_AVG" "true"
compare_values "L0 Files (aggregated) max" "$(printf '%.0f' $l0_max)" "$BASELINE_L0_FILES_MAX" "true"
compare_values "Immutable Memtables avg" "$(printf '%.2f' $immut_avg)" "$BASELINE_IMMUTABLE_MEMTABLES_AVG" "true"
compare_values "Immutable Memtables max" "$(printf '%.0f' $immut_max)" "$BASELINE_IMMUTABLE_MEMTABLES_MAX" "true"
compare_values "Pending Compaction max (GiB)" "$(format_gib $pending_max)" "$BASELINE_PENDING_COMPACTION_MAX" "true"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 5. MEMORY USAGE
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  5. MEMORY USAGE"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

table_readers_avg=$(query_range_stat "db_mem_table_readers_total{pod=\"${POD_NAME}\"}" "avg")
container_mem_avg=$(query_range_stat "container_memory_working_set_bytes{pod=\"${POD_NAME}\",container=\"madara\"}" "avg")

compare_values "Table Readers avg (GiB)" "$(format_gib $table_readers_avg)" "$BASELINE_TABLE_READERS_AVG" "true" "0.2"
compare_values "Container Memory avg (GiB)" "$(format_gib $container_mem_avg)" "$BASELINE_CONTAINER_MEMORY_AVG" "true" "0.2"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 6. THROUGHPUT
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  6. THROUGHPUT"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

blocks_per_hour=$(query_instant "rate(close_block_total_duration_seconds_count{pod=\"${POD_NAME}\"}[${HOURS}h]) * 3600")

compare_values "Blocks per Hour" "$(printf '%.2f' $blocks_per_hour)" "$BASELINE_BLOCKS_PER_HOUR" "false"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 7. DISK I/O
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  7. DISK I/O"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-35s | %12s | %12s | %15s |\n" "Metric" "Current" "Baseline" "Change"
echo "|-------------------------------------|--------------|--------------|-----------------|"

disk_write=$(query_instant "rate(container_fs_writes_bytes_total{pod=\"${POD_NAME}\",container=\"madara\"}[${HOURS}h])")
disk_read=$(query_instant "rate(container_fs_reads_bytes_total{pod=\"${POD_NAME}\",container=\"madara\"}[${HOURS}h])")

disk_write_mib=$(echo "scale=2; $disk_write / 1048576" | bc 2>/dev/null || echo "N/A")
disk_read_mib=$(echo "scale=2; $disk_read / 1048576" | bc 2>/dev/null || echo "N/A")

compare_values "Disk Write (MiB/s)" "$disk_write_mib" "$BASELINE_DISK_WRITE_AVG" "true" "0.2"
compare_values "Disk Read (MiB/s)" "$disk_read_mib" "$BASELINE_DISK_READ_AVG" "true" "0.2"
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# 8. CURRENT LSM TREE STRUCTURE
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  8. CURRENT LSM TREE STRUCTURE"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
printf "| %-10s | %12s |\n" "Level" "Files"
echo "|------------|--------------|"

for level in L0 L1 L2 L3 L4 L5 L6; do
    count=$(query_instant "db_level_files_count{pod=\"${POD_NAME}\",level=\"${level}\"}")
    printf "| %-10s | %12s |\n" "$level" "${count:-N/A}"
done
echo ""

# ─────────────────────────────────────────────────────────────────────────────────
# SUMMARY
# ─────────────────────────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════════════════════════════════"
echo "  SUMMARY"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""
echo "  Legend:"
echo "    ✅ Improved  - Metric improved by more than 10%"
echo "    ➖ Similar   - Metric within 10% of baseline"
echo "    ❌ Regressed - Metric worsened by more than 10%"
echo "    ⚠️  No data   - Metric not available"
echo ""
echo "  Key checks:"

# Write stall check
if [[ "$write_stopped_max" == "0" || "$write_stopped_max" == "null" || -z "$write_stopped_max" ]]; then
    echo "    ✅ No write stalls"
else
    echo "    ❌ Write stalls detected!"
fi

# Throughput check
if [[ -n "$blocks_per_hour" && "$blocks_per_hour" != "N/A" ]]; then
    throughput_pct=$(echo "scale=2; (($blocks_per_hour - $BASELINE_BLOCKS_PER_HOUR) / $BASELINE_BLOCKS_PER_HOUR) * 100" | bc 2>/dev/null || echo "0")
    if (( $(echo "$throughput_pct >= -5" | bc -l) )); then
        echo "    ✅ Throughput maintained (${blocks_per_hour} blocks/hr)"
    else
        echo "    ❌ Throughput decreased (${blocks_per_hour} blocks/hr, ${throughput_pct}%)"
    fi
fi

# Close block check
if [[ -n "$close_p95" && "$close_p95" != "N/A" ]]; then
    close_pct=$(echo "scale=2; (($close_p95 - $BASELINE_CLOSE_BLOCK_P95) / $BASELINE_CLOSE_BLOCK_P95) * 100" | bc 2>/dev/null || echo "0")
    if (( $(echo "$close_pct <= 10" | bc -l) )); then
        echo "    ✅ Block close time acceptable (p95: ${close_p95}s)"
    else
        echo "    ⚠️  Block close time increased (p95: ${close_p95}s, +${close_pct}%)"
    fi
fi

echo ""
echo "═══════════════════════════════════════════════════════════════════════════════"
