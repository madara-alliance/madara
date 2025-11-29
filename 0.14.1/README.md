# Starknet v0.14.1 Upgrade Documentation

This directory contains comprehensive documentation for upgrading Madara from v0.14.0 to v0.14.1 compatibility.

## Overview

Starknet v0.14.1 introduces several significant changes:

1. **SNIP-34: BLAKE Hash for CASM** - Migration from Poseidon to BLAKE hash functions for calculating compiled class hashes
2. **RPC 0.10.0** - New RPC specification with breaking changes
3. **DA Encryption** - Data availability encryption support in orchestrator
4. **Block Closing Optimization** - Faster block closing during low activity periods
5. **Backward Compatibility** - All changes must maintain compatibility with existing 0.14.0 data

## Timeline

- **Testnet**: November 19, 2025 (postponed from November 11)
- **Mainnet**: December 3, 2025 (postponed from November 25)

## Documentation Structure

- **[SNIP-34_CASM_Hash_Migration.md](./SNIP-34_CASM_Hash_Migration.md)** - Detailed guide for implementing BLAKE hash migration
- **[RPC_0.10.0_Changes.md](./RPC_0.10.0_Changes.md)** - Complete RPC specification updates
- **[Pathfinder_Implementation_Analysis.md](./Pathfinder_Implementation_Analysis.md)** - Analysis of Pathfinder's implementation approach
- **[Tasks.md](./Tasks.md)** - High-level task list organized by topic
- **[Implementation_Checklist.md](./Implementation_Checklist.md)** - Detailed step-by-step implementation guide
- **[Backward_Compatibility.md](./Backward_Compatibility.md)** - Backward compatibility considerations

## Key Resources

- [Starknet v0.14.1 Prerelease Notes](https://community.starknet.io/t/starknet-v0-14-1-prerelease-notes/116032)
- [SNIP-34 Specification](https://community.starknet.io/t/snip-34-more-efficient-casm-hashes/115979)
- [RPC 0.10.0 Spec Repository](https://github.com/starkware-libs/starknet-specs/tree/release/v0.10.0-rc.1)
- [Pathfinder PR #3068](https://github.com/eqlabs/pathfinder/pull/3068) - RPC 0.10.0 state-update changes
- [Pathfinder PR #3091](https://github.com/eqlabs/pathfinder/pull/3091) - Migrated classes support
- [Pathfinder PR #3094](https://github.com/eqlabs/pathfinder/pull/3094) - BLAKE CASM pre-computation

## Important Notes

- Pathfinder is a full-node implementation and does not have sequencer capabilities
- Madara has both full-node and sequencer capabilities, so implementation will differ
- All changes must maintain backward compatibility with 0.14.0 data
- Migration logic must handle existing CASM hashes calculated with Poseidon

## Implementation Priority

1. **High Priority**: SNIP-34 CASM hash migration (breaking change)
2. **High Priority**: RPC 0.10.0 specification updates
3. **Medium Priority**: Block closing optimizations
4. **Low Priority**: Performance improvements and optimizations
