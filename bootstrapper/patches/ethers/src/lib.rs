//! Patched `ethers` re-export crate for bootstrapper.
//!
//! This mirrors the upstream 2.0.14 crate while changing default TLS feature
//! selection in `Cargo.toml` from Rustls to OpenSSL.

#![warn(missing_debug_implementations, missing_docs, rust_2018_idioms, unreachable_pub)]
#![deny(rustdoc::broken_intra_doc_links)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[doc(inline)]
pub use ethers_addressbook as addressbook;
#[doc(inline)]
pub use ethers_contract as contract;
#[doc(inline)]
pub use ethers_core as core;
#[doc(inline)]
pub use ethers_middleware as middleware;
#[doc(inline)]
pub use ethers_providers as providers;
#[doc(inline)]
pub use ethers_signers as signers;

#[cfg(feature = "etherscan")]
#[doc(inline)]
pub use ethers_etherscan as etherscan;

#[cfg(feature = "solc")]
#[doc(inline)]
pub use ethers_solc as solc;

#[doc(no_inline)]
pub use ethers_core::{abi, types, utils};

/// Easy imports of frequently used type definitions and traits.
#[doc(hidden)]
#[allow(unknown_lints, ambiguous_glob_reexports)]
pub mod prelude {
    pub use super::addressbook::contract;
    pub use super::contract::*;
    pub use super::core::{types::*, *};
    pub use super::middleware::*;
    pub use super::providers::*;
    pub use super::signers::*;

    #[cfg(feature = "etherscan")]
    pub use super::etherscan::*;

    #[cfg(feature = "solc")]
    pub use super::solc::*;
}

#[doc(hidden)]
#[allow(unused_extern_crates)]
extern crate self as ethers;
