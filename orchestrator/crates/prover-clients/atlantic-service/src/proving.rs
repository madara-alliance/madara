//! Proving layer abstraction for network-specific configuration
//!
//! This module provides the [`ProvingLayer`] trait which allows different
//! settlement layers (Ethereum, Starknet) to add their specific parameters
//! to proving requests.
//!
//! # Background
//!
//! Different blockchain networks have different requirements for proof verification.
//! For example, Starknet requires specific layout and result parameters that
//! Ethereum doesn't need. This module abstracts these differences behind a
//! common trait.
//!
//! # Components
//!
//! - [`ProvingLayer`]: Trait for adding network-specific proving parameters
//! - [`EthereumLayer`]: Implementation for Ethereum settlement (no extra params)
//! - [`StarknetLayer`]: Implementation for Starknet settlement (adds layout/result)
//! - [`ProvingParams`]: Parameters passed to the proving layer
//! - [`create_proving_layer`]: Factory function to create the appropriate layer
//!
//! # Example
//!
//! ```ignore
//! use crate::proving::{create_proving_layer, ProvingParams};
//!
//! // Create the appropriate layer based on configuration
//! let layer = create_proving_layer("starknet");
//!
//! // Add proving params to request
//! let request = layer.add_proving_params(request, ProvingParams {
//!     layout: LayoutName::dynamic,
//! });
//! ```

use cairo_vm::types::layout_name::LayoutName;
use orchestrator_utils::http_client::RequestBuilder;

use crate::types::AtlanticQueryStep;

/// Parameters for proving layer specific configuration
pub struct ProvingParams {
    /// Layout to be used for the proof
    pub layout: LayoutName,
}

/// Trait for network-specific proving layer configuration
///
/// Different settlement layers may require different parameters
/// to be added to proving requests. This trait provides a way
/// to abstract over these differences.
///
/// # Implementations
///
/// - `EthereumLayer`: No additional parameters needed
/// - `StarknetLayer`: Adds result and layout parameters
pub trait ProvingLayer: Send + Sync {
    /// Adds proving layer specific parameters to the request
    ///
    /// # Arguments
    /// * `request` - The request builder to add parameters to
    /// * `params` - The proving parameters (layout, etc.)
    ///
    /// # Returns
    /// The modified request builder
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, params: ProvingParams) -> RequestBuilder<'a>;
}

/// Ethereum proving layer
///
/// For Ethereum settlement, no additional parameters are needed
/// beyond the base request configuration.
pub struct EthereumLayer;

impl ProvingLayer for EthereumLayer {
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, _params: ProvingParams) -> RequestBuilder<'a> {
        // Ethereum doesn't need additional proving params
        request
    }
}

/// Starknet proving layer
///
/// For Starknet settlement, we need to add the result step
/// and layout parameters to the request.
pub struct StarknetLayer;

impl ProvingLayer for StarknetLayer {
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, params: ProvingParams) -> RequestBuilder<'a> {
        request
            .form_text("result", &AtlanticQueryStep::ProofGeneration.to_string())
            .form_text("layout", params.layout.to_str())
    }
}

/// Creates the appropriate proving layer based on the settlement layer name
///
/// # Arguments
/// * `settlement_layer` - The name of the settlement layer ("ethereum" or "starknet")
///
/// # Returns
/// A boxed proving layer implementation
///
/// # Panics
/// Panics if the settlement layer is not recognized
pub fn create_proving_layer(settlement_layer: &str) -> Box<dyn ProvingLayer> {
    match settlement_layer {
        "ethereum" => Box::new(EthereumLayer),
        "starknet" => Box::new(StarknetLayer),
        _ => panic!("Invalid settlement layer: {}", settlement_layer),
    }
}
