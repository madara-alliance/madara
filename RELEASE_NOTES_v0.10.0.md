# Madara v0.10.0 Release Notes

## Important

Before this merge, all 0.14.1 `sn-version`–related code was maintained on the `main-v0.14.1` branch, which contains the complete commit history for that version.

After this release, `main` will track the latest 0.14.1 code. Any bug fixes or improvements that are not specific to the 0.14.1 sn-version will be cherry-picked into `main-v0.14.0` as well, which is the only other version we are supporting at the moment.

---

## 🚀 Highlights

This release brings **Starknet v0.14.1 support** with full SNIP-34 (BLAKE compiled class hash) implementation, **RPC v0.10.0** specification support, and significant improvements to database migrations and class handling.

---

## ✨ New Features

### Starknet v0.14.1 Protocol Support

- Full support for Starknet protocol version v0.14.1
- Implementation of **SNIP-34**: BLAKE2s compiled class hash support
  - Classes declared on v0.14.1+ now use BLAKE2s hash instead of Poseidon
  - `SierraClassInfo` now supports dual hash storage (`compiled_class_hash` for Poseidon v1, `compiled_class_hash_v2` for BLAKE v2)
  - Added `compile_to_casm_with_hashes()` method that computes both hash variants
  - Proper handling of migrated classes in state diffs via `migrated_compiled_classes` field

### RPC v0.10.0 Specification Support

- Added complete RPC v0.10.0 implementation with all read, write, trace, and WebSocket methods
- New RPC endpoints:
  - `starknet_getBlockWithReceipts`
  - `starknet_getBlockWithTxHashes`
  - `starknet_getBlockWithTxs`
  - `starknet_getEvents`
  - `starknet_getStateUpdate`
  - `starknet_estimateMessageFee`

### Database Migration System

- New migration **revision 0009**: SierraClassInfo and StateDiff format changes for SNIP-34 support
- Database schema version bumped from 8 to 9
- Improved migration error handling and context propagation
- Batch processing with configurable batch size and progress logging

### Orchestrator Enhancements

- Added STWO prover backend support for Atlantic service (`--atlantic-sharp-prover` option)
- SNIP-34 `migrated_compiled_classes` support in state diff compression/squashing
- Improved handling of compiled class hashes in DA and SNOS jobs

---

## 🔧 Improvements

### Block Production

- Proper separation of declared classes vs migrated classes in state diffs
- Classes now correctly use v2 (BLAKE) or v1 (Poseidon) hash based on protocol version
- Improved compiled class hash handling in block closing logic

### Sync & Import

- Protocol version-aware class hash verification during sync
- Support for both Poseidon and BLAKE compiled class hashes during import
- Fixed Sierra class hash validation based on block protocol version

### Class Compilation

- New `CompiledClassHashes` struct containing both Poseidon and BLAKE hashes
- Added `compute_blake_compiled_class_hash()` using starknet_api's HashVersion::V2
- Improved class compilation error handling

### State Update Handling

- Added `from_blockifier()` method to StateDiff for proper migration class handling
- New `MigratedClassItem` type for tracking class hash migrations
- Improved conversion between internal and RPC state update formats

---

## 🐛 Bug Fixes

- Fixed Dockerfile sequencer requirements.txt lookup logic
- Fixed state diff handling for classes with optional compiled class hashes
- Improved error messages for migration failures

---

## 📦 Dependencies

- Bumped various Cargo dependencies
- Updated cairo requirements
- Upgraded orchestrator-atlantic-service with new types

---

## ⚠️ Breaking Changes

### Database Migration Required

- **Database version 8 → 9**: Existing databases will be automatically migrated
- `SierraClassInfo` format changed: `compiled_class_hash: Felt` → `Option<Felt>`, added `compiled_class_hash_v2: Option<Felt>`
- `StateDiff` now includes optional `migrated_compiled_classes` field

### Configuration Changes

- Devnet preset updated for v0.14.1 compatibility

---

## 📋 Upgrade Guide

1. **Backup your database** before upgrading (migration is automatic but backup recommended)
2. The database migration from v8 to v9 will run automatically on first start
3. No configuration changes required unless using custom class handling

---

## What's Changed

- Main v0.14.1 by @Mohiiit in https://github.com/madara-alliance/madara/pull/924

---

## 📚 Related Links

- [SNIP-34: BLAKE Compiled Class Hash](https://github.com/starknet-io/SNIPs/blob/main/SNIPS/snip-34.md)
- [Starknet RPC v0.10.0 Specification](https://github.com/starkware-libs/starknet-specs)

**Full Changelog**: https://github.com/madara-alliance/madara/compare/v0.9.1...v0.10.0
