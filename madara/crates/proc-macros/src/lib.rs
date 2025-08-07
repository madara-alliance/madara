// Source:
// https://github.com/starkware-libs/papyrus/tree/main/crates/papyrus_proc_macros

//! This macro is a wrapper around the "rpc" macro supplied by the jsonrpsee
//! library that generate server and client traits from a given trait
//! definition.
//!
//! We use this macro to add versioning information to these traits and their
//! methods to more easily support multiple versions of the starknet rpc specs,
//! as well as madara-specific extensions.
//!
//! # Attributes
//!
//! ---
//!
//! `versionsed_rpc` **attribute**
//!
//! ---
//!
//! `versioned_rpc` attributes is applied to traits to decorate them with a
//! version and a namespace. The version and namespace are also added to each
//! of its methods.
//!
//! **Arguments:**
//! - `version`: valid sermver version string with capital `V` prepended
//! - `namespace`: rpc method namspace, added to all methods
//!
//! ---
//!
//! `method` and `subscription` **attribute**
//!
//! ---
//!
//! These attributes annotate trait methods as rpc http or websocket pub/sub
//! mehtods. When combined with `versioned_rpc`, extra versioning information
//! is added to them.
//!
//! **Arguments:**
//! - `name`: rpc method name, must not be a duplicate in the current namespace.
//! - `and_versions`: implementations of this method will also work for the
//!   supplied versions. Note that these versions must not already contain
//!   a method with the same name.
//!
//! # Example:
//!
//! Given this code:
//!
//! ```rust
//! # use m_proc_macros::versioned_rpc;
//! # use std::sync::Arc;
//! # use std::error::Error;
//! # use jsonrpsee::core::RpcResult;
//!
//! #[versioned_rpc("V0_7_1", "starknet")]
//! pub trait JsonRpc {
//!     #[method(name = "blockNumber", and_versions = ["V0_8_1"])]
//!     fn block_number(&self) -> RpcResult<u64>;
//! }
//! ```
//!
//! The macro will generate the following code:
//!
//! ```rust
//! # use m_proc_macros::versioned_rpc;
//! # use std::sync::Arc;
//! # use std::error::Error;
//! # use jsonrpsee::core::RpcResult;
//!
//! #[jsonrpsee::proc_macros::rpc(server, client, namespace = "starknet")]
//! pub trait JsonRpcV0_7_1 {
//!     #[method(name = "V0_7_1_blockNumber", aliases = ["starknet_V0_8_1blockNumber"])]
//!     fn block_number(&self) -> RpcResult<u64>;
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;

#[derive(Debug)]
struct VersionedRpcAttr {
    version: String,
    namespace: String,
}

fn validate_version(version: &str) -> Result<(), syn::Error> {
    if !version.starts_with('V') {
        return Err(syn::Error::new(Span::call_site(), "Version must start with 'V'"));
    }

    let parts: Vec<&str> = version[1..].split('_').collect();

    if parts.len() != 3 {
        return Err(syn::Error::new(Span::call_site(), "Version must have exactly three parts (VMAJOR_MINOR_PATCH)"));
    }

    for part in parts {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(syn::Error::new(Span::call_site(), "Each part of the version must be a non-empty number"));
        }
    }

    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), syn::Error> {
    if namespace.trim().is_empty() {
        Err(syn::Error::new(
            Span::call_site(),
            indoc::indoc!(
                r#"
                    Namespace cannot be empty.
                    Please provide a non-empty namespace string.

                    ex: #[versioned_rpc("V0_7_1", "starknet")]
                "#
            ),
        ))
    } else {
        Ok(())
    }
}

impl syn::parse::Parse for VersionedRpcAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let version = input.parse::<syn::LitStr>()?.value();
        input.parse::<syn::Token![,]>()?;
        let namespace = input.parse::<syn::LitStr>()?.value();

        validate_version(&version)?;
        validate_namespace(&namespace)?;

        Ok(VersionedRpcAttr { version, namespace })
    }
}

enum CallType {
    Method,
    Subscribe,
}

#[proc_macro_attribute]
pub fn versioned_rpc(attr: TokenStream, input: TokenStream) -> TokenStream {
    let VersionedRpcAttr { version, namespace } = syn::parse_macro_input!(attr as VersionedRpcAttr);
    let mut item_trait = syn::parse_macro_input!(input as syn::ItemTrait);

    let trait_name = &item_trait.ident;
    let train_name_with_version = syn::Ident::new(&format!("{trait_name}{version}"), trait_name.span());

    // This next section is reponsible for versioning the method name declared
    // with jsonrpsee
    let err = item_trait.items.iter_mut().try_fold((), |_, item| {
        let syn::TraitItem::Fn(method) = item else {
            return Err(syn::Error::new(
                item.span(),
                indoc::indoc! {r#"
                    Traits marked with `versioned_rpc` can only contain methods

                    ex:

                    #[versioned_rpc("V0_7_0", "starknet")]
                    trait MyTrait {
                        #[method(name = "foo", blocking)]
                        fn foo();
                    }
                "#},
            ));
        };

        method.attrs.iter_mut().try_fold((), |_, attr| {
            // We leave simple attribute parsing errors to be handled by
            // jsonrpsee
            let path = attr.path();
            let ident = if path.is_ident("method") {
                CallType::Method
            } else if path.is_ident("subscription") {
                CallType::Subscribe
            } else {
                return Ok(());
            };

            let syn::Meta::List(meta_list) = &attr.meta else {
                return Ok(());
            };

            let attr_args = meta_list
                .parse_args_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
                .map_err(|_| {
                    syn::Error::new(
                        meta_list.span(),
                        indoc::indoc! {r#"
                                The `method` and `subscription` attributes expect comma-separated values.

                                ex: `#[method(name = "foo", blocking)]`
                            "#},
                    )
                })?;

            // This convoluted section is just the way by which we traverse
            // the macro attribute list. We are looking for:
            //
            // - An assignment
            // - With lvalue a Path expression with literal value `name` or
            //  'unsubscribe'
            // - With rvalue a literal
            //
            // Any other attribute is skipped over and is not overwritten
            let mut method_name = None;
            let mut method_tokens = attr_args
                .iter()
                .filter_map(|expr| {
                    // There isn't really a more elegant way of doing this as
                    // `left` and `right` are boxed values and therefore cannot
                    // be pattern matched without being de-referenced
                    let syn::Expr::Assign(expr) = expr else { return Some(expr.clone()) };

                    let syn::Expr::Path(syn::ExprPath { path, .. }) = *expr.left.clone() else {
                        return Some(syn::Expr::Assign(expr.clone()));
                    };

                    // `and_versions` needs to be removed as it is not a valid
                    // jsonrpsee macro attribute
                    if path.is_ident("and_versions") {
                        return None;
                    }

                    let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(right), attrs }) = *expr.right.clone() else {
                        return Some(syn::Expr::Assign(expr.clone()));
                    };

                    if !path.is_ident("name") && !path.is_ident("unsubscribe") {
                        return Some(syn::Expr::Assign(expr.clone()));
                    }

                    method_name = Some(right.value());
                    let method_with_version = format!("{version}_{}", right.value());
                    let expr = syn::Expr::Assign(syn::ExprAssign {
                        right: Box::new(syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(syn::LitStr::new(&method_with_version, right.span())),
                            attrs,
                        })),
                        ..expr.clone()
                    });

                    Some(expr)
                })
                .collect::<Vec<_>>();

            // Method name is required by jsonrpsee anyways and this makes it
            // easier to work with
            let Some(method) = method_name else {
                return Err(syn::Error::new(
                    meta_list.span(),
                    indoc::indoc! {r#"
                            The 'method' and 'subscription' attributes expect a name.

                            ex: #[method(name = "foo")]
                        "#
                    },
                ));
            };

            // Same parsing logic as above, except `and_versions` is a nested
            // array and needs to be parsed for punctuations as well
            let aliases_tokens = attr_args.iter().cloned().try_fold(Vec::default(), |mut acc, expr| {
                let syn::Expr::Assign(expr) = expr else { return syn::Result::Ok(acc) };

                let syn::Expr::Path(syn::ExprPath { path, .. }) = *expr.left.clone() else {
                    return Ok(acc);
                };
                let syn::Expr::Array(syn::ExprArray { elems, .. }) = *expr.right.clone() else {
                    return Ok(acc);
                };

                if !path.is_ident("and_versions") {
                    return Ok(acc);
                }

                for elem in elems {
                    if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(version), attrs }) = elem {
                        let version_str = version.value();
                        validate_version(&version_str)?;
                        let method_with_version = format!("{namespace}_{}_{method}", version_str);

                        let lit = syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(syn::LitStr::new(&method_with_version, version.span())),
                            attrs,
                        });

                        acc.push(lit);
                    }
                }

                Ok(acc)
            })?;

            if !aliases_tokens.is_empty() {
                method_tokens.push(syn::parse_quote!(aliases = [#(#aliases_tokens),*]));
            }

            // This is the part where we actually replace the attribute with
            // its versioned alternative. Note that the syntax #(#foo),*
            // indicates a pattern repetition here, where all the elements in
            // attr_args are expanded into rust code
            attr.meta = match ident {
                CallType::Method => syn::parse_quote!(method(#(#method_tokens),*)),
                CallType::Subscribe => syn::parse_quote!(subscription(#(#method_tokens),*)),
            };

            Ok(())
        })
    });

    if let Err(e) = err {
        return e.into_compile_error().into();
    }

    let trait_with_version = syn::ItemTrait {
        attrs: vec![syn::parse_quote!(#[jsonrpsee::proc_macros::rpc(server, client, namespace = #namespace)])],
        ident: train_name_with_version,
        ..item_trait
    };

    quote! {
        #trait_with_version
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
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
                r#"
                Namespace cannot be empty.
                Please provide a non-empty namespace string.

                ex: #[versioned_rpc("V0_7_1", "starknet")]
            "#
            )
        );
    }
}
