//! The **Feeder Gateway**, which you will sometimes see referred to as _fgw_, is an unspecified
//! endpoint of a Starknet node. It is used for synchronization during block sync from a centralized
//! source, predating Starknet's upgrade to p2p block sync.
//!
//! Despite this, the fgw has never been properly specified. Parity with mainnet implementation is
//! done on a best-effort basis and differences in edge cases are highly likely.
//!
//! This crate implements a local fgw. For ways to connect, request and deserialize information from
//! an existing fgw, check out the fgw client under `gateway/client`. Keep in mind that the fgw
//! server is disabled by default and has to be explicitly enabled via the cli on node startup.
//!
//! # Available methods
//!
//! Below is a list of methods made available to the fgw. Keep in mind that the actual
//! implementation is subject to subtle yet meaningful differences between nodes, and this includes
//! the official Starknet fgw endpoints.
//!
//! TODO

mod error;
mod handler;
mod helpers;
mod router;
pub mod service;
