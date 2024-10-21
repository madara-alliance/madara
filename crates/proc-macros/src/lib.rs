// Source:
// https://github.com/starkware-libs/papyrus/tree/main/crates/papyrus_proc_macros

//! This macro is a wrapper around the "rpc" macro supplied by the jsonrpsee library that generates
//! a server and client traits from a given trait definition. The wrapper gets a version id and
//! prepends the version id to the trait name and to every method name (note method name refers to
//! the name the API has for the function not the actual function name). We need this in order to be
//! able to merge multiple versions of jsonrpc APIs into one server and not have a clash in method
//! resolution.
//!
//! # Example:
//!
//! Given this code:
//! ```rust,ignore
//! #[versioned_rpc("V0_7_1", "starknet")]
//! pub trait JsonRpc {
//!     #[method(name = "blockNumber", aliases = ["block_number"])]
//!     fn block_number(&self) -> anyhow::Result<u64>;
//! }
//! ```
//!
//! The macro will generate this code:
//! ```rust,ignore
//! #[rpc(server, namespace = "starknet")]
//! pub trait JsonRpcV0_7_1 {
//!     #[method(name = "V0_7_1_blockNumber", aliases = ["block_number"])]
//!     fn block_number(&self) -> anyhow::Result<u64>;
//! }
//! ```
//!
//! > [!NOTE]
//! > This macro _will not_ override any other jsonrpsee attribute, meaning
//! > it does not currently support renaming `aliases` or `unsubscribe_aliases`

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;

#[derive(Debug)]
struct VersionedRpcAttr {
    version: String,
    namespace: String,
}

impl syn::parse::Parse for VersionedRpcAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let version = input.parse::<syn::LitStr>()?.value();
        input.parse::<syn::Token![,]>()?;
        let namespace = input.parse::<syn::LitStr>()?.value();

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
                    r#"
                    Namespace cannot be empty.
                    Please provide a non-empty namespace string.

                    ex: #[versioned_rpc("V0_7_1", "starknet")]
                "#
                ),
            ));
        }

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
            // We leave these errors to be handled by jsonrpsee
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

            // This convoluted section is just the way by which we traverse
            // the macro attribute list. We are looking for:
            //
            // - An assignment
            // - With lvalue a Path expression with literal value `name` or
            //  'unsubscribe'
            // - With rvalue a literal
            //
            // Any other attribute is skipped over and is not overwritten
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
                })?
                .into_iter()
                .map(|expr| {
                    // There isn't really a more elegant way of doing this as
                    // `left` and `right` are boxed values and therefore cannot
                    // be pattern matched without being de-referenced
                    let syn::Expr::Assign(expr) = expr else { return expr };

                    let syn::Expr::Path(syn::ExprPath { path, .. }) = *expr.left.clone() else {
                        return syn::Expr::Assign(expr);
                    };
                    let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(right), attrs }) = *expr.right.clone() else {
                        return syn::Expr::Assign(expr);
                    };

                    if !path.is_ident("name") && !path.is_ident("unsubscribe") {
                        return syn::Expr::Assign(expr);
                    }

                    let method_with_version = format!("{version}_{}", right.value());
                    syn::Expr::Assign(syn::ExprAssign {
                        right: Box::new(syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(syn::LitStr::new(&method_with_version, right.span())),
                            attrs,
                        })),
                        ..expr
                    })
                })
                .collect::<Vec<_>>();

            // This is the part where we actually replace the attribute with
            // its versioned alternative. Note that the syntax #(#foo),*
            // indicates a pattern repetition here, where all the elements in
            // attr_args are expanded into rust code
            attr.meta = match ident {
                CallType::Method => syn::parse_quote!(method(#(#attr_args),*)),
                CallType::Subscribe => syn::parse_quote!(subscription(#(#attr_args),*)),
            };

            Ok(())
        })
    });

    if let Err(e) = err {
        return e.into_compile_error().into();
    }

    let trait_with_version = syn::ItemTrait {
        attrs: vec![syn::parse_quote!(#[jsonrpsee::proc_macros::rpc(server, namespace = #namespace)])],
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
