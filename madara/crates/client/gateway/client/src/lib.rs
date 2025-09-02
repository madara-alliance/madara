//! The **Feeder Gateway**, which you will sometimes see referred to as _fgw_, is an unspecified
//! endpoint of a Starknet node. It is used for synchronization during block sync from a centralized
//! source, predating Starknet's upgrade to p2p block sync.
//!
//! Despite this, the fgw has never been properly specified. Parity with mainnet implementation is
//! done on a best-effort basis and differences in edge cases are highly likely.
//!
//! This crate implements the methods and deserialization logic to connect to a fgw endpoint during
//! block sync. For an local implementation of the fgw, check out the fgw server under
//! `gateway/server`.

mod builder;
mod methods;
mod request_builder;
mod submit_tx;

pub use builder::GatewayProvider;
