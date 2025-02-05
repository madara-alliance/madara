//! This crate is a fork of the original [starknet-types-rpc](https://github.com/starknet-io/types-rs) v0.7.1,
//! which was originally developed by the Starknet team under the MIT license.
//!
//! The original crate is no longer actively maintained, so this fork has been created
//! to ensure continued maintenance and compatibility with our project needs.
//!
//! Original authors:
//! - Pedro Fontana (@pefontana)
//! - Mario Rugiero (@Oppen)
//! - Lucas Levy (@LucasLvy)
//! - Shahar Papini (@spapinistarkware)
//! - Abdelhamid Bakhta (@abdelhamidbakhta)
//! - Dan Brownstein (@dan-starkware)
//! - Federico Carrone (@unbalancedparentheses)
//! - Jonathan Lei (@xJonathanLEI)
//! - Maciej Kamiński (@maciejka)
//!
//! Original repository: https://github.com/starknet-io/types-rs
//! Original version: 0.7.1
//! License: MIT

mod custom;
mod custom_serde;

pub mod v0_7_1;

pub use self::v0_7_1::*;
