// Source:
// https://github.com/starkware-libs/papyrus/tree/main/crates/papyrus_proc_macros

//! This macro is a wrapper around the "rpc" macro supplied by the jsonrpsee library that generates
//! a server and client traits from a given trait definition. The wrapper gets a version id and
//! prepend the version id to the trait name and to every method name (note method name refers to
//! the name the API has for the function not the actual function name). We need this in order to be
//! able to merge multiple versions of jsonrpc APIs into one server and not have a clash in method
//! resolution.
//!
//! # Example:
//!
//! Given this code:
//! ```rust,ignore
//! #[versioned_starknet_rpc("V0_7_1")]
//! pub trait JsonRpc {
//!     #[method(name = "blockNumber")]
//!     fn block_number(&self) -> anyhow::Result<u64>;
//! }
//! ```
//!
//! The macro will generate this code:
//! ```rust,ignore
//! #[rpc(server, namespace = "starknet")]
//! pub trait JsonRpcV0_7_1 {
//!     #[method(name = "V0_7_1_blockNumber")]
//!     fn block_number(&self) -> anyhow::Result<u64>;
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse::Parse, parse_macro_input, Attribute, Ident, ItemTrait, LitStr, TraitItem};

#[derive(Debug)]
struct VersionedRpcAttr {
    version: String,
    namespace: String,
}

impl Parse for VersionedRpcAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let version = input.parse::<LitStr>()?.value();
        input.parse::<syn::Token![,]>()?;
        let namespace = input.parse::<LitStr>()?.value();

        if !version.starts_with('V') {
            return Err(syn::Error::new(Span::call_site(), "Version must start with 'V'"));
        }

        let parts: Vec<&str> = version[1..].split('_').collect();

        if parts.len() != 3 {
            return Err(syn::Error::new(
                Span::call_site(),
                "Version must have exactly three parts (VMAJOR_MINOR_PATCH)",
            ));
        }

        for part in parts {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return Err(syn::Error::new(Span::call_site(), "Each part of the version must be a non-empty number"));
            }
        }

        if namespace.trim().is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                indoc::indoc!(
                    "
                    Namespace cannot be empty.
                    Please provide a non-empty namespace string.
                    Example: #[versioned_rpc(\"V0_7_1\", \"starknet\")]
                "
                ),
            ));
        }

        Ok(VersionedRpcAttr { version, namespace })
    }
}

fn version_method_name(attr: &Attribute, version: &str) -> syn::Result<Attribute> {
    let mut new_attr = attr.clone();
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("name") {
            let value = meta.value()?;
            let method_name: LitStr = value.parse()?;
            let new_name = format!("{version}_{}", method_name.value());
            new_attr.meta = syn::parse_quote!(method(name = #new_name));
        }
        Ok(())
    })?;
    Ok(new_attr)
}

#[proc_macro_attribute]
pub fn versioned_rpc(attr: TokenStream, input: TokenStream) -> TokenStream {
    let VersionedRpcAttr { version, namespace } = parse_macro_input!(attr as VersionedRpcAttr);
    let mut item_trait = parse_macro_input!(input as ItemTrait);

    let trait_name = &item_trait.ident;
    let versioned_trait_name = Ident::new(&format!("{trait_name}{version}"), trait_name.span());

    for item in &mut item_trait.items {
        if let TraitItem::Fn(method) = item {
            method.attrs = method
                .attrs
                .iter()
                .filter_map(|attr| {
                    if attr.path().is_ident("method") {
                        version_method_name(attr, &version).ok()
                    } else {
                        Some(attr.clone())
                    }
                })
                .collect();
        }
    }

    let versioned_trait = ItemTrait {
        attrs: vec![syn::parse_quote!(#[rpc(server, namespace = #namespace)])],
        ident: versioned_trait_name,
        ..item_trait
    };

    quote! {
        #versioned_trait
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::{quote, ToTokens};
    use syn::parse_quote;

    #[test]
    fn test_versioned_rpc_attribute_parsing() {
        let attr: VersionedRpcAttr = parse_quote!("V0_7_1", "starknet");
        assert_eq!(attr.version, "V0_7_1");
        assert_eq!(attr.namespace, "starknet");
    }

    #[test]
    fn test_versioned_rpc_attribute_parsing_invalid_version() {
        let result: syn::Result<VersionedRpcAttr> = syn::parse2(quote!("0_7_1", "starknet"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Version must start with 'V'");
    }

    #[test]
    fn test_versioned_rpc_attribute_parsing_invalid_parts() {
        let result: syn::Result<VersionedRpcAttr> = syn::parse2(quote!("V0_7", "starknet"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Version must have exactly three parts (VMAJOR_MINOR_PATCH)");
    }

    #[test]
    fn test_versioned_rpc_attribute_parsing_empty_namespace() {
        let result: syn::Result<VersionedRpcAttr> = syn::parse2(quote!("V0_7_1", ""));
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            indoc::indoc!(
                "
                Namespace cannot be empty.
                Please provide a non-empty namespace string.
                Example: #[versioned_rpc(\"V0_7_1\", \"starknet\")]
            "
            )
        );
    }

    #[test]
    fn test_version_method_name() {
        let attr: Attribute = parse_quote!(#[method(name = "blockNumber")]);
        let result = version_method_name(&attr, "V0_7_1").unwrap();
        assert_eq!(result.to_token_stream().to_string(), "# [method (name = \"V0_7_1_blockNumber\")]");
    }
}
