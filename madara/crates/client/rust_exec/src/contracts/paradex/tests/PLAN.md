# Paradex rust-exec test plan

## Location choice

Tests will live next to the Paradex rust-exec modules in:
`/Users/heemankverma/Work/Karnot/RvsC/madara/madara/crates/client/rust-exec/src/contracts/paradex/tests`

Rationale:

- Direct access to module internals without extra pub re-exports.
- Keeps contract logic and tests tightly coupled.

Alternative (not chosen): crate-level integration tests under
`/Users/heemankverma/Work/Karnot/RvsC/madara/madara/crates/client/rust-exec/tests`.

## Planned file layout

- `mod.rs` (test module root)
- `fixtures.rs` (MockStateReader helpers, storage setters, TradeRequestV3 builders)
- `paraclear_selectors.rs`
- `paraclear_decode.rs`
- `paraclear_delegate.rs`
- `paraclear_asset_kind.rs`
- `paraclear_settlement_token.rs`
- `paraclear_max_assets.rs`
- `paraclear_spot.rs`
- `paraclear_perp.rs`
- `paraclear_fees.rs`
- `paraclear_balances.rs`
- `paraclear_risk.rs`
- `paraclear_storage_keys.rs`
- `assets_manager.rs`
- `oracle.rs`
- `precomputed_sn_keccak.rs`
- `TESTS.md` (tracking added vs skipped tests)

## Concrete steps

1. Add test scaffolding (module + tracking docs)
   - Ensure `paradex/mod.rs` includes `#[cfg(test)] mod tests;`
   - Create `tests/mod.rs`, `PLAN.md`, `TESTS.md`
2. Build shared fixtures in `fixtures.rs`
   - `MockStateReader` wrappers for storage/class hash/nonce
   - Helpers for setting storage via layout keys
   - Builders for `TradeRequestV3`, `OrderV3`, `AccountStateLite`
3. Implement unit tests for:
   - `assets_manager.rs`
   - `oracle.rs`
   - `precomputed_sn_keccak.rs`
4. Implement Paraclear unit tests:
   - decode/encode helpers
   - key derivations / storage layout helpers
   - fee math, referral logic, risk math
5. Implement Paraclear flow tests:
   - spot settle success/fail paths
   - perpetual settle success/fail paths
   - event emission and state diff expectations
6. Update `TESTS.md` as tests are added or explicitly skipped

## Notes

- Use existing `crate::state::mock::MockStateReader` where possible.
- Prefer table-driven tests to cover multiple cases in a single test function.
- Keep tests deterministic: avoid env dependencies unless explicitly scoped.
