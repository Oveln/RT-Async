extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ItemFn, PatType, ReturnType};
use syn::spanned::Spanned;

/// Marks an async function as a statically-allocated task.
///
/// Transforms `async fn name(args)` into two items:
/// - `async fn __name(args) { ... }` — the original future body.
/// - `fn name(args) -> Result<SpawnToken<impl Future>, SpawnError>` — spawns
///   the future into a `TaskStorage` static and returns a token for
///   [`Spawner::spawn`].
///
/// # Constraints
///
/// - Must be applied to an `async fn`.
/// - The function must return `()` (explicitly or implicitly).
/// - No generic parameters allowed.
/// - No `self` parameter allowed.
///
/// # Example
///
/// ```ignore
/// #[executor::task]
/// async fn blink(led: usize) {
///     // ...
/// }
///
/// // Expands to:
/// // fn blink(led: usize) -> Result<SpawnToken<impl Future<Output = ()>>, SpawnError>
///
/// spawner.spawn(Priority::new(0), blink(13).unwrap());
/// ```
///
/// [`Spawner::spawn`]: executor::spawner::Spawner::spawn
#[proc_macro_attribute]
pub fn task(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let f = parse_macro_input!(item as ItemFn);

    if f.sig.asyncness.is_none() {
        return syn::Error::new(
            f.sig.fn_token.span,
            "`#[task]` can only be applied to async functions",
        )
        .to_compile_error()
        .into();
    }

    if !f.sig.generics.params.is_empty() {
        return syn::Error::new(
            f.sig.generics.span(),
            "`#[task]` does not support generic parameters",
        )
        .to_compile_error()
        .into();
    }

    match &f.sig.output {
        ReturnType::Default => {}
        ReturnType::Type(_, ty) => {
            if let syn::Type::Tuple(tup) = ty.as_ref() {
                if tup.elems.is_empty() {
                    // explicitly returns `()` — ok
                } else {
                    return syn::Error::new(ty.span(), "`#[task]` functions must return `()`")
                        .to_compile_error()
                        .into();
                }
            } else {
                return syn::Error::new(ty.span(), "`#[task]` functions must return `()`")
                    .to_compile_error()
                    .into();
            }
        }
    }

    for arg in &f.sig.inputs {
        if let FnArg::Receiver(_) = arg {
            return syn::Error::new(
                arg.span(),
                "`#[task]` functions cannot have `self` parameters",
            )
            .to_compile_error()
            .into();
        }
    }

    let vis = &f.vis;
    let name = &f.sig.ident;
    let inner_name = quote::format_ident!("__{}", name);
    let inputs = &f.sig.inputs;
    let body = &f.block;

    let arg_pats: Vec<_> = inputs
        .iter()
        .map(|arg| {
            let FnArg::Typed(PatType { pat, .. }) = arg else {
                unreachable!("self already rejected");
            };
            pat.clone()
        })
        .collect();

    let expanded = quote! {
        async fn #inner_name(#inputs) #body

        #vis fn #name(#inputs) -> ::core::result::Result<
            executor::spawner::SpawnToken<impl ::core::future::Future<Output = ()> + 'static>,
            executor::task::storage::SpawnError,
        > {
            trait __TaskTrait {
                type __Fut: ::core::future::Future<Output = ()> + 'static;
                fn __construct(#inputs) -> Self::__Fut;
            }
            impl __TaskTrait for () {
                type __Fut = impl ::core::future::Future<Output = ()> + 'static;
                fn __construct(#inputs) -> Self::__Fut {
                    #inner_name(#(#arg_pats),*)
                }
            }
            static __TASK: executor::task::storage::TaskStorage<<() as __TaskTrait>::__Fut> =
                executor::task::storage::TaskStorage::new();
            __TASK.spawn(move || <() as __TaskTrait>::__construct(#(#arg_pats),*))
        }
    };

    expanded.into()
}
