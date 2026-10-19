# Adaptive resource budgeting

Status: proposal

## Summary

Madara should accept one operator policy:

```text
--foreground-headroom <factor>
MADARA_FOREGROUND_HEADROOM=<factor>
```

`1.0` reserves the measured foreground requirement, `1.5` reserves 50% above
it, and `2.0` reserves twice the measured requirement. Madara derives worker
counts, memory budgets, RPC execution concurrency, and the RocksDB compaction
rate from the resources visible to the process and a persisted workload
profile.

The policy is role aware:

- On a sequencer, foreground work is block execution, block finalization,
  Parallel Merkle, and the minimum RPC work needed by block production.
- On a full node, foreground work is synchronization and execution RPC.
- RocksDB flush and compaction are background work in both roles.

The resource governor must reject startup when the remaining background budget
cannot sustain the measured compaction arrival rate. It must not guess a safe
configuration and let compaction debt grow without bound.

When `--foreground-headroom` is absent, Madara retains its existing behavior.
This keeps the proposal opt-in while profiles are collected and validated.

## Why the current behavior bursts

RocksDB background parallelism currently follows
`std::thread::available_parallelism()`:

```rust
options.increase_parallelism(cores);
options.set_max_background_jobs(cores);
options.set_max_subcompactions(cores as _);
env.set_low_priority_background_threads(cores);
```

A larger pod CPU limit therefore allows more simultaneous compactions and
subcompactions. `bytes_per_sync` makes individual file syncs incremental, but
does not limit aggregate compaction throughput. Several column families can
become eligible together, consume the storage queue, delay foreground reads,
and then release a block-production catch-up wave.

The governor replaces "use every visible core" with one coordinated resource
plan and a bounded RocksDB rate. The same amount of compaction work is spread
over time, producing a plateau instead of a burst.

## A headroom factor needs facts, not guesses

A multiplier alone is insufficient. Madara also needs:

1. The process resource envelope.
2. A measured foreground and compaction baseline.
3. Current compaction debt.

CPU and memory limits can be discovered locally from cgroup v2 (`cpu.max` and
`memory.max`), with `available_parallelism()` and physical memory as bare-metal
fallbacks.

Provisioned EBS IOPS and throughput cannot be discovered from a Linux block
device or EC2 instance metadata. They are volume configuration, independent of
instance type. Madara must not infer them from an instance name or run a
destructive startup benchmark. The deployment controller must mount a
read-only resource envelope at a conventional path such as:

```json
{
  "version": 1,
  "storage": {
    "combined_iops": 16000,
    "combined_mib_per_second": 750,
    "safety_margin_percent": 10
  },
  "source": "kubernetes-volume-and-node-capacity"
}
```

The combined fields match how gp3 capacity is provisioned; read and write work
must not each be allowed to consume the full advertised limit. This file is
discovered automatically and is not another application tuning policy. The
operator or deployment system owns the hardware facts, while the Madara
operator supplies only `foreground_headroom`.

Strict automatic mode must reject startup if the storage envelope is absent.
Reporting a sustainability verdict without storage capacity would be false
confidence.

## Persisted workload profile

Madara records an atomic, versioned profile beside the database. A profile is
valid only for the same database format, chain role, and workload class. It
contains rolling representative values rather than the last, possibly idle,
window:

```json
{
  "version": 1,
  "role": "sequencer",
  "sample_window_seconds": 86400,
  "foreground": {
    "cpu_cores_p99": 3.1,
    "memory_mib_peak": 8120,
    "read_iops_p99": 2200,
    "write_iops_p99": 900,
    "read_mib_per_second_p99": 105,
    "write_mib_per_second_p99": 42
  },
  "compaction": {
    "input_mib_per_second_p99": 72,
    "output_mib_per_second_p99": 61,
    "total_io_per_output_byte": 2.18,
    "output_mib_per_cpu_second": 47
  }
}
```

The profile must distinguish foreground RocksDB reads/writes from compaction
reads/writes. Aggregate device throughput is not enough: it would count the
burst being controlled as foreground demand and reserve capacity for the
problem itself.

The first strict startup requires a profile produced by a representative
canary or a previous observation run. There is no honest way to prove future
compaction sustainability before observing a workload at least once. A fresh
database without a profile must stop with instructions to run an observation
deployment; it must not invent a baseline.

## Startup planning

The resource governor runs before network-facing services or block production
are activated:

1. Discover cgroup CPU and memory limits.
2. Read the platform storage envelope.
3. Read and validate the persisted workload profile.
4. Calculate a preliminary plan.
5. Open RocksDB with the calculated low background limits.
6. Read current pending-compaction properties from every column family.
7. Add the required debt-drain rate and finalize the plan.
8. Log the full plan and sustainability verdict.
9. Start services only when the verdict is `sustainable`.

RocksDB must be opened with the preliminary rate and worker limits already in
place. Otherwise database open can itself start the unconstrained compaction
burst that this design is meant to prevent.

### Foreground reservation

For each resource `r`:

```text
foreground_reserved[r] = measured_foreground_p99[r] * headroom_factor
usable_capacity[r]      = detected_capacity[r] - platform_safety_margin[r]
background_capacity[r]  = usable_capacity[r] - foreground_reserved[r]
```

The platform safety margin covers the kernel, telemetry, and storage behavior
outside Madara. It is part of the platform resource envelope, not another
Madara tuning knob.

Memory uses the observed foreground peak instead of p99 because exceeding the
memory limit can terminate the process.

### Compaction sustainability

The required compaction service rate is:

```text
required_output_rate = measured_compaction_output_p99
                     + pending_compaction_bytes / debt_drain_window

required_storage_rate = required_output_rate * measured_total_io_per_output_byte
required_cpu_cores    = required_output_rate / measured_output_per_cpu_second
```

The debt drain window is selected internally from the workload profile window;
it is not exposed as another operator setting. With no debt, the requirement is
the measured p99 arrival rate. With debt, the plan must both keep up and drain
the backlog over time.

Startup is sustainable only when all of these are true:

```text
background_storage_mib_per_second >= required_storage_rate
background_storage_iops           >= required_compaction_iops
background_cpu_cores              >= required_cpu_cores
background_memory                 >= minimum_rocksdb_background_memory
```

Pending bytes alone are not a sustainability measurement. A database can have
little current debt while its steady-state arrival rate is greater than the
allowed service rate.

## Derived runtime settings

The plan controls all related parallelism from one source of truth.

### Block production and Parallel Merkle

Blockifier execution, block finalization, and Parallel Merkle consume a shared
foreground permit pool sized from `foreground_reserved.cpu`. Parallel Merkle
root workers may not exceed that pool.

Correctness settings still win. The governor must not enable optimistic
Blockifier concurrency when `disable_concurrency: true`. If concurrency is
enabled, its worker count is bounded by the foreground pool rather than raw
machine cores.

This avoids independently configuring Blockifier and Parallel Merkle to each
assume they own all CPUs.

### Execution RPC

Execution-heavy RPC uses the same foreground coordinator, with block
production or synchronization at higher priority. RPC may borrow idle
foreground permits but cannot consume the minimum permits required by the
role's primary work.

The RPC semaphore introduced by #1272 becomes the enforcement point. Its size
is derived from the plan rather than a separately tuned static count.
Lightweight metadata RPC remains outside this semaphore.

### RocksDB

The background CPU and storage plan derives:

- low-priority compaction thread count;
- high-priority flush thread count;
- `max_background_jobs`;
- `max_subcompactions`;
- a shared RocksDB rate limiter;
- write-buffer and block-cache memory ceilings.

The rate limiter must be created before opening the database. The current Rust
binding limits flush and compaction writes. The planner therefore converts the
combined storage budget into an output-write limit using the measured
compaction read/write amplification instead of treating the configured rate as
total device traffic.

Flush retains higher RocksDB priority than compaction. The plan always keeps at
least one flush worker and one compaction worker; a result that cannot afford
both is rejected.

## Startup output

The governor emits one structured event that can also be rendered as a readable
startup table:

```text
madara_resource_plan role=sequencer verdict=sustainable headroom_factor=2.0
  detected:
    cpu=12.0 cores memory=24576 MiB
    storage=16000 IOPS / 750 MiB/s source=kubernetes-volume-and-node-capacity
  measured foreground:
    cpu_p99=3.1 cores memory_peak=8120 MiB
    storage_p99=3100 IOPS / 147 MiB/s
  reserved foreground:
    cpu=6.2 cores memory=16240 MiB
    storage=6200 IOPS / 294 MiB/s
  available background:
    cpu=4.8 cores memory=6288 MiB
    storage=8200 IOPS / 381 MiB/s
  compaction required:
    cpu=2.4 cores storage=5100 IOPS / 226 MiB/s
    pending=1.90 GiB drain_eta=3h12m
  derived:
    blockifier_workers=1 parallel_merkle_workers=4 rpc_execution_limit=3
    compaction_threads=2 flush_threads=1 max_subcompactions=1
    compaction_output_limit=104 MiB/s
```

Every value includes its source: cgroup, resource envelope, persisted profile,
or live RocksDB property. This makes a derived setting reproducible rather than
magic.

An unsustainable plan exits before services start:

```text
madara_resource_plan role=sequencer verdict=unsustainable headroom_factor=2.0
  required compaction storage=226 MiB/s available=118 MiB/s
  required compaction cpu=2.4 cores available=1.1 cores

Automatic resource plan rejected: the remaining resources cannot sustain
RocksDB compaction. Reduce foreground headroom, reduce workload, or increase
the instance/storage capacity. Madara did not start block production or RPC.
```

The same format is used for full nodes, with `foreground` labelled
`sync_and_rpc` and no block-production worker fields.

## Runtime guardrail

Startup validation proves the plan against the available profile, not all
future workloads. The governor therefore checks the plan continuously:

- Compaction rate is capped at the startup maximum, preventing a throughput
  burst even when several column families become eligible together.
- If foreground latency or storage queue pressure rises, compaction is reduced
  gradually but never paused indefinitely.
- If compaction debt has a positive trend while the limiter is at its allowed
  maximum, Madara reports the plan as no longer sustainable.
- A sequencer stops accepting new transactions before debt reaches a RocksDB
  write-stop threshold. It does not abruptly kill an in-progress block.
- A full node sheds execution RPC before pausing synchronization.

Changing the worker count at runtime is optional for the first implementation.
A fixed worker count derived at startup plus an adjustable RocksDB byte rate is
enough to flatten the observed spikes. Dynamic worker-pool resizing should be
added only if measurements show the byte limiter is insufficient.

## Full-node behavior

Full nodes need the same compaction plateau even though they do not produce
blocks.

Their foreground profile includes:

- L2 synchronization and trie updates;
- execution-heavy RPC admitted by #1272;
- lightweight RPC memory and network overhead.

Synchronization has priority over execution RPC. When resources tighten, RPC
concurrency is reduced first. Compaction keeps its sustainable minimum so the
node does not trade an immediate RPC improvement for an eventual RocksDB stall.

An idle, caught-up window must not replace the representative sync profile with
near-zero demand. Profiles retain the most recent active-window distribution
for startup planning.

## Implementation sequence

1. Add resource-envelope discovery, profile parsing, pure planning formulas,
   startup logs, and fail-closed validation.
2. Apply the plan to RocksDB background pools and its shared rate limiter.
3. Integrate the foreground permit pool with Parallel Merkle and Blockifier.
4. Base #1272's execution-RPC semaphore on the same pool.
5. Persist foreground/compaction measurements and add runtime debt monitoring.
6. Add deployment-controller support for the storage resource envelope.

The first production canary should run in report-only mode against the same
database snapshot and resource envelope. Enforcement should be enabled only
after the derived plan matches observed CPU, memory, storage, and compaction
debt behavior over at least one full compaction cycle.

## Rejected alternatives

### Derive everything from the instance type

Rejected because EBS volume configuration is independent of instance type,
pod limits may be smaller than node capacity, and cloud instance tables do not
describe current workload demand.

### Benchmark the disk at every startup

Rejected because a useful write benchmark changes the production volume,
competes with recovery, and measures a transient maximum rather than the
provisioned sustainable budget.

### Configure only worker counts

Rejected because a small number of compaction jobs can still saturate storage.
Workers bound parallelism; the RocksDB rate limiter creates the throughput
plateau.

### Configure only a byte rate

Rejected because many simultaneous low-rate jobs can still create high random
I/O and CPU contention. The planner must bound both concurrency and throughput.

### Infer sustainability from pending compaction bytes at startup

Rejected because one snapshot does not reveal the rate at which new compaction
work arrives. Sustainability requires a measured arrival rate and a service
rate.
