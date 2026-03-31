# CI/CD Workflows

## Overview

The CI system uses GitHub Actions with reusable workflows (`task-*.yml`) orchestrated by trigger workflows. Disabled features use `task-do-nothing-*.yml` stubs that exit immediately with success, keeping the dependency graph intact.

---

## Trigger Workflows

### 1. `pull-request-main-ci.yml` — PR Checks

**Trigger:** PR opened/synchronized/reopened/ready_for_review to `main` or `main-**`
**Skipped for:** Draft PRs

```
                    ┌──────────────────────┐     ┌──────────────────┐
                    │ update-version-db    │     │ update-version-  │
                    │ (label: db-migration)│     │ artifacts        │
                    └──────────┬───────────┘     │ (label:          │
                               │                 │ artifacts-update)│
                               │                 └────────┬─────────┘
                               │                          │
                               │                 ┌────────▼─────────┐
                               │                 │  build-artifacts │
                               │                 └────────┬─────────┘
                               │                          │
                               │                 ┌────────▼─────────┐
                               │                 │ publish-artifacts│
                               │                 └────────┬─────────┘
                               │                          │
                               ▼                          ▼
                    ┌─────────────────────────────────────────────┐
                    │              lint-rust                       │
                    │              lint-code-style                 │
                    │  (needs: update-version-db, publish-artifacts)│
                    └──────────────────────┬──────────────────────┘
                                           │
                          ┌────────────────┼────────────────┐
                          ▼                ▼                ▼
                  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
                  │ build-madara │ │check-orchestr.│ │check-bootstr.│
                  └──────┬───────┘ └──────┬───────┘ │  (no-op)     │
                         │                │         └──────────────┘
          ┌──────┬───────┼────────┐       │
          ▼      ▼       ▼        ▼       ▼
      ┌──────┐┌──────┐┌──────┐┌──────┐┌──────────────┐
      │test- ││test- ││test- ││test- ││test-         │
      │madara││migr. ││js    ││cli   ││orchestrator  │
      └──────┘└──────┘└──────┘└──────┘│(needs: build-│
                                      │madara + check│
                                      │-orchestrator)│
                                      └──────────────┘

      ┌───────────────────────────────────────────────┐
      │ test-end-to-end (independent, no dependencies) │
      │ Builds own binaries, only on base_ref == main  │
      └───────────────────────────────────────────────┘

      test-bootstrapper (no-op, v1 deprecated)
      build-nightly-and-publish-* (no-op placeholders)
```

**Artifact build conditional:** Only runs if PR has the `artifacts-update` label. If skipped, downstream jobs still run (GitHub treats skipped dependencies as satisfied) and `build.rs` pulls the existing artifact image from GHCR.

---

### 2. `nightly-run.yml` — Nightly Build, Test & Publish

**Trigger:** Daily at midnight UTC (cron), or manual dispatch
**Gate:** Only runs if commits were merged to `main` in the previous UTC day

```
      ┌─────────────────────┐
      │ check-merged-today  │
      └──────────┬──────────┘
                 │
    ┌────────────┼────────────┐
    ▼            ▼            ▼
┌──────────┐┌──────────┐┌──────────┐
│build-    ││build-    ││build-    │
│madara    ││orchestr. ││bootstr.  │
└────┬─────┘└────┬─────┘└────┬─────┘
     │           │           │
     └───────────┼───────────┘
                 ▼
      ┌───────────────────┐
      │ test-end-to-end   │
      └────────┬──────────┘
               │
    ┌──────────┼──────────┐
    ▼          ▼          ▼
┌────────┐┌────────┐┌────────┐
│publish ││publish ││publish │
│madara  ││orchest.││bootstr.│
│nightly ││nightly ││nightly │
└────────┘└────────┘└────────┘
               │
      ┌────────▼──────────┐
      │notify-on-failure  │
      │(SNS, runs always) │
      └───────────────────┘

    test-madara, test-orchestrator, test-bootstrapper,
    test-js, test-cli (all no-op stubs)
```

**Note:** Nightly images are only published from `main` branch. The e2e test in nightly still uses the old format (pre-built binary inputs).

---

### 3. `pull-request-merge.yml` — Merge Queue

**Trigger:** Merge group events (when PR enters merge queue)

All jobs are currently **no-op stubs**. This is a placeholder for future merge queue validation.

---

### 4. `release-publish.yml` — Release Publishing

**Trigger:** GitHub release published

```
    ┌─────────────────────────┐
    │ release event (publish) │
    └────────────┬────────────┘
                 │
    ┌────────────┼────────────┐
    ▼            ▼            ▼
┌────────┐ ┌────────┐ ┌────────┐
│publish │ │publish │ │publish │
│madara  │ │orchest.│ │bootstr.│
│release │ │release │ │release │
└────────┘ └────────┘ └────────┘
```

---

### 5. `pull-request-close.yml` — PR Cleanup

**Trigger:** PR closed

Deletes Docker images associated with the closed PR from GHCR.

---

### 6. `schedule-daily-maintenance-issues.yml` — Daily Maintenance

**Trigger:** Daily at 8:00 UTC

Creates maintenance tracking issues.

---

### 7. `schedule-daily-security-audit.yml` — Security Audit

**Trigger:** Daily at 6:00 UTC

Runs `cargo audit` for dependency vulnerability scanning.

---

### 8. `claude.yml` / `claude-code-review.yml` — AI Review

**Trigger:** Issue comments / PR events

Claude Code integration for automated PR reviews.

---

### 9. `manual-build-docker-images.yml` — Manual Docker Build

**Trigger:** Manual dispatch only

Builds Docker images with custom parameters (branch, image name, etc.).

---

## Reusable Task Workflows

### Build Tasks

| Workflow                             | Purpose                                                                    | Used By           |
| ------------------------------------ | -------------------------------------------------------------------------- | ----------------- |
| `task-build-madara.yml`              | Build Madara binary, upload as artifact                                    | PR CI, Nightly    |
| `task-build-orchestrator.yml`        | Build Orchestrator binary, upload as artifact                              | Nightly           |
| `task-build-bootstrapper.yml`        | Build Bootstrapper v1 binary (deprecated)                                  | Nightly           |
| `task-build-artifacts.yml`           | Build contract artifacts via Docker (`build.docker`), create archive image | PR CI             |
| `task-build-binary.yml`              | Generic reusable binary builder                                            | Other build tasks |
| `task-build-nightly-and-publish.yml` | Build and publish nightly Docker image to GHCR                             | Nightly           |
| `task-build-and-publish-release.yml` | Build and publish release Docker image to GHCR                             | Release           |
| `task-build-manual-and-publish.yml`  | Build and publish with manual params                                       | Manual            |

### Test Tasks

| Workflow                     | Purpose                                                           | Used By        |
| ---------------------------- | ----------------------------------------------------------------- | -------------- |
| `task-test-end-to-end.yml`   | Full E2E test — builds all binaries, runs bridge deposit/withdraw | PR CI, Nightly |
| `task-test-madara.yml`       | Madara unit/integration tests with coverage                       | PR CI          |
| `task-test-orchestrator.yml` | Orchestrator unit/integration tests with coverage                 | PR CI          |
| `task-test-bootstrapper.yml` | Bootstrapper v1 tests (deprecated)                                | —              |
| `task-test-js.yml`           | JavaScript RPC tests against Madara binary                        | PR CI          |
| `task-test-cli.yml`          | CLI tests                                                         | PR CI          |
| `task-test-migration.yml`    | Database migration tests                                          | PR CI          |
| `task-test-hive.yml`         | Ethereum Hive compatibility tests                                 | —              |
| `task-e2e-orchestrator.yml`  | Orchestrator-specific E2E tests (disabled)                        | —              |

### Lint & Check Tasks

| Workflow                      | Purpose                                              | Used By |
| ----------------------------- | ---------------------------------------------------- | ------- |
| `task-lint-cargo.yml`         | Rust linting — clippy, fmt, workspace check          | PR CI   |
| `task-lint-code-style.yml`    | Code style — prettier, markdown-lint, TOML lint      | PR CI   |
| `task-check-orchestrator.yml` | Orchestrator `cargo check` (compile without linking) | PR CI   |
| `task-check-bootstrapper.yml` | Bootstrapper check (no-op, v1 deprecated)            | PR CI   |

### Infrastructure Tasks

| Workflow                   | Purpose                                            | Used By        |
| -------------------------- | -------------------------------------------------- | -------------- |
| `task-ci-version-file.yml` | Auto-bump version files when PR has specific label | PR CI          |
| `task-publish-image.yml`   | Push Docker image to GHCR with tags                | PR CI, Nightly |
| `task-publish-stable.yml`  | Tag existing image as stable                       | —              |

### No-Op Stubs (`task-do-nothing-*.yml`)

These immediately exit with success. Used as placeholders when a feature is disabled, allowing dependent jobs in the dependency graph to still run.

| Stub                                            | Replaces                             |
| ----------------------------------------------- | ------------------------------------ |
| `task-do-nothing-build-nightly-and-publish.yml` | `task-build-nightly-and-publish.yml` |
| `task-do-nothing-build-nightly.yml`             | `task-build-nightly-and-publish.yml` |
| `task-do-nothing-check-bootstrapper.yml`        | `task-check-bootstrapper.yml`        |
| `task-do-nothing-e2e-orchestrator.yml`          | `task-e2e-orchestrator.yml`          |
| `task-do-nothing-publish-image.yml`             | `task-publish-image.yml`             |
| `task-do-nothing-test-bootstrapper.yml`         | `task-test-bootstrapper.yml`         |
| `task-do-nothing-test-cli.yml`                  | `task-test-cli.yml`                  |
| `task-do-nothing-test-end-to-end.yml`           | `task-test-end-to-end.yml`           |
| `task-do-nothing-test-hive.yml`                 | `task-test-hive.yml`                 |
| `task-do-nothing-test-js.yml`                   | `task-test-js.yml`                   |
| `task-do-nothing-test-madara.yml`               | `task-test-madara.yml`               |
| `task-do-nothing-test-migration.yml`            | `task-test-migration.yml`            |
| `task-do-nothing-test-orchestrator.yml`         | `task-test-orchestrator.yml`         |

---

## Artifact Versioning

Contract artifacts (compiled Solidity/Cairo) are versioned and distributed as Docker images on GHCR.

### Version File: `.artifact-versions.yml`

```yaml
current_version: 9
versions:
  - version: 9
    pr: 1013
  - version: 8
    pr: 801
  ...
```

### Flow

```
1. Developer adds `artifacts-update` label to PR
                    │
2. task-ci-version-file.yml
   ├─ Checks label exists
   ├─ Checks PR number not already in history
   ├─ Bumps current_version (8 → 9)
   ├─ Adds {version: 9, pr: 1013} to history
   └─ Commits and pushes to PR branch
                    │
3. task-build-artifacts.yml
   ├─ Reads current_version (9)
   ├─ Tags as VERSION+1 = 10 (for CI internal use)
   ├─ Runs: make artifacts (Docker build via build.docker)
   ├─ Compresses to artifacts.tar.gz
   └─ Builds archive image (archive.docker)
                    │
4. task-publish-image.yml
   ├─ Pushes ghcr.io/madara-alliance/artifacts (latest)
   └─ Pushes ghcr.io/madara-alliance/artifacts:10
                    │
5. At cargo build time (any crate with build.rs)
   ├─ build_version::get_or_compile_artifacts()
   ├─ Reads current_version from .artifact-versions.yml
   ├─ Pulls ghcr.io/madara-alliance/artifacts:{current_version}
   ├─ Extracts artifacts.tar.gz from container
   └─ Unpacks to build-artifacts/
```

**Note:** There is a version offset — CI tags as `current_version + 1` but `build.rs` pulls `current_version`. When manually pushing artifacts, push **both** tags to avoid mismatches.

### Database Versioning

Follows the same pattern with `.db-versions.yml` and the `db-migration` label.

---

## E2E Test

The E2E test (`task-test-end-to-end.yml`) is self-contained in the PR CI — it builds all binaries (Madara, Orchestrator, Bootstrapper V2) and runs the full bridge deposit/withdraw test.

**Local equivalent:** `./scripts/run-e2e.sh`

### Services Started During Test

| Service         | Purpose                            | Port (dynamic)           |
| --------------- | ---------------------------------- | ------------------------ |
| Anvil           | L1 Ethereum chain                  | random                   |
| MongoDB         | Orchestrator database              | random                   |
| LocalStack      | AWS S3, SQS, SNS, EventBridge mock | random                   |
| Madara          | L2 Starknet sequencer              | random                   |
| Pathfinder      | Starknet full node                 | random                   |
| Orchestrator    | Proof pipeline coordinator         | random                   |
| Bootstrapper V2 | L1/L2 contract deployment          | N/A (runs to completion) |

### Test Phases

**Phase 1 — Setup:**

1. Start infrastructure (MongoDB, LocalStack)
2. Setup L1 (Anvil → mock verifier → bootstrapper base)
3. Setup L2 (Madara → bootstrapper madara)
4. Sync full node (Pathfinder catches up to Madara)
5. Stop block production
6. Run orchestrator, wait for batches, close open batches, wait for state settlement
7. Dump databases for reuse

**Phase 2 — Test:**

1. Restore databases, start all services
2. ETH deposit (L1 → L2)
3. ERC20 deposit (L1 → L2)
4. ETH withdrawal (L2 → L1)
5. ERC20 withdrawal (L2 → L1)
6. Wait for withdrawal finality (AcceptedOnL1)

---

## Quick Reference

| Action                     | How                                                               |
| -------------------------- | ----------------------------------------------------------------- |
| Run E2E locally            | `./scripts/run-e2e.sh`                                            |
| Rebuild artifacts          | Add `artifacts-update` label to PR, push commit                   |
| Rebuild artifacts manually | `make artifacts` then push Docker image                           |
| Trigger CI                 | Push a commit (or empty: `git commit --allow-empty -m "trigger"`) |
| Re-run failed job          | GitHub UI → "Re-run failed jobs"                                  |
| Skip CI on draft           | PR must be marked "Ready for review"                              |
