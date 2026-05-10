extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{FnArg, ItemFn, PatType, ReturnType, parse_macro_input};

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

/// Application entry-point macro.
///
/// Replaces the hand-written `__rust_main` and `MachineSoft` with a
/// declarative interface.  The user only writes the task-spawning body; all
/// platform initialisation, scheduler ISR wiring, and the idle loop are
/// generated automatically.
///
/// # What it generates
///
/// Given a priority count `N` (extracted from the `Spawner<N>` type argument),
/// the macro emits **three** top-level items:
///
/// 1. **`static mut __SPAWNER`** — a `MaybeUninit<Spawner<N>>` that lives for
///    the entire program lifetime.  It is pinned in place and shared between
///    `__rust_main` (initialisation) and `MachineSoft` (ISR).
///
/// 2. **`#[unsafe(no_mangle)] __rust_main() -> !`** — the bare-metal entry
///    point jumped to from the assembly `_start` stub.  It performs, in order:
///    - `platform::init()`  — logger / platform setup
///    - Create `Spawner::new()`, pin it, call `.init()`
///    - Execute the *user's function body* (the task-spawning code)
///    - `platform::start()` — enable MSI + global interrupts
///    - `loop { platform::idle() }` — WFI sleep
///
/// 3. **`#[unsafe(no_mangle)] MachineSoft(&mut TrapFrame)`** — the RISC-V
///    machine software-interrupt handler.  After clearing the MSI pending
///    flag, it checks [`platform::PEND_MARKER`] to distinguish the interrupt
///    source:
///
///    | `PEND_MARKER` | Source                    | Action                              |
///    |---------------|---------------------------|-------------------------------------|
///    | `true`        | `platform::pend()` (scheduler) | Run the priority-preemptive scheduler loop (`try_preempt` → `run` → `complete_executor`) |
///    | `false`       | External MSI (hardware / inter-core) | Call `__Inner_MachineSoft` — a user-defined hook see [`#[executor::interrupt]`][interrupt]) |
///
///    This design ensures the scheduler ISR is never accidentally bypassed,
///    while still giving the user full control over non-scheduler MSI events.
///
/// # Signature requirements
///
/// The decorated function **must** have exactly one parameter of type
/// `Pin<&'static Spawner<N>>` (or `Pin<&Spawner<N>>`).  The macro extracts
/// `N` from the const-generic argument at compile time.
///
/// # Constraints
///
/// - Must **not** be `async`.
/// - No generic parameters on the function.
/// - Exactly one `Spawner` parameter (see above).
///
/// # Example
///
/// ```ignore
/// #![no_std]
/// #![no_main]
///
/// #[executor::main]
/// fn main(spawner: core::pin::Pin<&'static executor::spawner::Spawner<4>>) {
///     spawner.spawn(Priority::new(0), blink(13).unwrap());
///     spawner.spawn(Priority::new(1), periodic().unwrap());
/// }
/// ```
///
/// # Interaction with `#[executor::interrupt]`
///
/// The `MachineSoft` symbol is always generated by this macro and must not
/// be defined elsewhere.  To handle *external* MSI events (i.e. MSI not
/// triggered by the scheduler's `pend()`), define a function named
/// `MachineSoft` with `#[executor::interrupt]`:
///
/// ```ignore
/// #[executor::interrupt]
/// fn MachineSoft(_tf: &mut TrapFrame) {
///     // This runs only for external MSI — the macro rewrites the
///     // symbol to `__Inner_MachineSoft` automatically.
/// }
/// ```
///
/// [`platform::PEND_MARKER`]: platform::PEND_MARKER
/// [interrupt]: executor::interrupt
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let f = parse_macro_input!(item as ItemFn);

    if f.sig.asyncness.is_some() {
        return syn::Error::new(
            f.sig.fn_token.span,
            "`#[main]` cannot be applied to async functions",
        )
        .to_compile_error()
        .into();
    }

    if !f.sig.generics.params.is_empty() {
        return syn::Error::new(
            f.sig.generics.span(),
            "`#[main]` does not support generic parameters",
        )
        .to_compile_error()
        .into();
    }

    // Extract exactly one parameter: the spawner
    let inputs = &f.sig.inputs;
    if inputs.len() != 1 {
        return syn::Error::new(
            f.sig.fn_token.span,
            "`#[main]` expects exactly one parameter: `Pin<&Spawner<N>>`",
        )
        .to_compile_error()
        .into();
    }

    let FnArg::Typed(PatType { pat, ty, .. }) = &inputs[0] else {
        return syn::Error::new(
            inputs[0].span(),
            "`#[main]` parameter must be `Pin<&Spawner<N>>`",
        )
        .to_compile_error()
        .into();
    };

    let n = match extract_spawner_n(ty) {
        Ok(n) => n,
        Err(e) => return e.to_compile_error().into(),
    };

    let spawner_pat = pat;
    let body = &f.block;
    let n_lit = syn::LitInt::new(&n.to_string(), proc_macro2::Span::call_site());

    let no_mangle_attr: proc_macro2::TokenStream = "#[unsafe(no_mangle)]".parse().unwrap();

    let expanded = quote! {
        static mut __SPAWNER: ::core::mem::MaybeUninit<executor::spawner::Spawner<#n_lit>> =
            ::core::mem::MaybeUninit::uninit();

        fn main(#spawner_pat: #ty) {
            #body
        }

        #no_mangle_attr
        pub unsafe extern "C" fn __rust_main() -> ! {
            unsafe {
                platform::init();

                let ptr = ::core::ptr::addr_of_mut!(__SPAWNER)
                    .cast::<executor::spawner::Spawner<#n_lit>>();
                ptr.write(executor::spawner::Spawner::new());
                ::core::pin::Pin::new_unchecked(&mut *ptr)
                    .init();

                let #spawner_pat = ::core::pin::Pin::new_unchecked(&*ptr);

                main(#spawner_pat);

                platform::start();
            }
            loop {
                platform::idle();
            }
        }

        #no_mangle_attr
        pub unsafe extern "C" fn MachineSoft(trap_frame: &mut platform::arch::TrapFrame) {
            unsafe {
                if platform::clear_pend() {
                    let #spawner_pat = ::core::pin::Pin::new_unchecked(
                        &*::core::ptr::addr_of!(__SPAWNER)
                            .cast::<executor::spawner::Spawner<#n_lit>>(),
                    );

                    while let Some(rt) = #spawner_pat.try_preempt() {
                        platform::enable_interrupts();
                        #spawner_pat.run(rt);
                        platform::disable_interrupts();
                        #spawner_pat.complete_executor();
                    }
                } else {
                    unsafe extern "C" {
                        fn __Inner_MachineSoft(trap_frame: &mut platform::arch::TrapFrame);
                    }
                    __Inner_MachineSoft(trap_frame);
                }
            }
        }
    };

    expanded.into()
}

/// Extract the const generic `N` from `Pin<&Spawner<N>>` or `Pin<&'static Spawner<N>>`.
fn extract_spawner_n(ty: &syn::Type) -> syn::Result<usize> {
    // Outer: Pin<...>
    let syn::Type::Path(type_path) = ty else {
        return Err(syn::Error::new(ty.span(), "expected Pin<&Spawner<N>>"));
    };
    let pin_seg = type_path
        .path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new(ty.span(), "expected Pin<&Spawner<N>>"))?;
    if pin_seg.ident != "Pin" {
        return Err(syn::Error::new(
            pin_seg.ident.span(),
            "expected Pin<&Spawner<N>>",
        ));
    }
    let syn::PathArguments::AngleBracketed(args) = &pin_seg.arguments else {
        return Err(syn::Error::new(
            pin_seg.span(),
            "Pin missing generic arguments",
        ));
    };

    // Inner: &Spawner<N> or &'lifetime Spawner<N>
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
        return Err(syn::Error::new(
            args.span(),
            "Pin generic argument must be &Spawner<N>",
        ));
    };
    let syn::Type::Reference(ref_ty) = inner else {
        return Err(syn::Error::new(
            inner.span(),
            "Pin generic argument must be &Spawner<N>",
        ));
    };

    // Spawner<N>
    let syn::Type::Path(spawner_path) = &*ref_ty.elem else {
        return Err(syn::Error::new(ref_ty.elem.span(), "expected Spawner<N>"));
    };
    let spawner_seg = spawner_path
        .path
        .segments
        .last()
        .ok_or_else(|| syn::Error::new(spawner_path.span(), "expected Spawner<N>"))?;
    if spawner_seg.ident != "Spawner" {
        return Err(syn::Error::new(
            spawner_seg.ident.span(),
            "expected Spawner<N>",
        ));
    }
    let syn::PathArguments::AngleBracketed(spawner_args) = &spawner_seg.arguments else {
        return Err(syn::Error::new(
            spawner_seg.span(),
            "Spawner missing const generic N",
        ));
    };

    let Some(syn::GenericArgument::Const(expr)) = spawner_args.args.first() else {
        return Err(syn::Error::new(
            spawner_args.span(),
            "expected Spawner<N> with const generic",
        ));
    };
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Int(lit),
        ..
    }) = expr
    else {
        return Err(syn::Error::new(expr.span(), "N must be an integer literal"));
    };

    lit.base10_parse::<usize>()
}

/// Marks a function as a RISC-V interrupt handler.
///
/// Transforms the decorated function into a `#[unsafe(no_mangle)] pub unsafe
/// extern "C"` handler whose *symbol name* equals the function name, matching
/// the RISC-V interrupt trap dispatch table.
///
/// # Symbol name mapping
///
/// | Function name       | Output symbol          | Notes |
/// |---------------------|------------------------|-------|
/// | `MachineTimer`      | `MachineTimer`         | Direct mapping |
/// | `MachineExternal`   | `MachineExternal`      | Direct mapping |
/// | `MachineSoft`       | `__Inner_MachineSoft`  | **Special** — see below |
/// | *(any other)*       | *(same as function name)* | Direct mapping |
///
/// ## `MachineSoft` special case
///
/// The `MachineSoft` symbol is reserved by [`#[executor::main]`][main] for the
/// scheduler ISR.  When the user writes a function named `MachineSoft`, this
/// macro automatically rewrites the output symbol to `__Inner_MachineSoft`.
///
/// At runtime the generated `MachineSoft` ISR checks [`platform::PEND_MARKER`]:
///
/// - **System pend** (`PEND_MARKER == true`) → runs the scheduler.
/// - **External MSI** (`PEND_MARKER == false`) → calls the user's
///   `__Inner_MachineSoft`.
///
/// If the user does **not** define a `MachineSoft` with this macro, the linker
/// provides a weak default (`DefaultHandler`) for `__Inner_MachineSoft`, which
/// will abort — consistent with any other unhandled interrupt.
///
/// # Signature
///
/// The function must take a single `&mut TrapFrame` parameter and return `()`.
/// The macro adds `#[unsafe(no_mangle)]` and the `unsafe extern "C"` wrapper.
///
/// # Constraints
///
/// - Must **not** be `async`.
/// - Must take exactly one `&mut TrapFrame` parameter.
///
/// # Examples
///
/// Timer interrupt:
///
/// ```ignore
/// #[executor::interrupt]
/// fn MachineTimer(_tf: &mut TrapFrame) {
///     // clear timer interrupt, update tick, etc.
/// }
/// ```
///
/// External MSI handler (only called for non-scheduler MSI):
///
/// ```ignore
/// #[executor::interrupt]
/// fn MachineSoft(_tf: &mut TrapFrame) {
///     // Symbol rewritten to `__Inner_MachineSoft`.
///     // Runs only when MSI is triggered externally (not by pend()).
/// }
/// ```
///
/// [main]: executor::main
/// [`platform::PEND_MARKER`]: platform::PEND_MARKER
#[proc_macro_attribute]
pub fn interrupt(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let f = parse_macro_input!(item as ItemFn);

    if f.sig.asyncness.is_some() {
        return syn::Error::new(
            f.sig.fn_token.span,
            "`#[interrupt]` cannot be applied to async functions",
        )
        .to_compile_error()
        .into();
    }

    let name = &f.sig.ident;
    let inputs = &f.sig.inputs;
    let body = &f.block;

    let no_mangle_attr: proc_macro2::TokenStream = "#[unsafe(no_mangle)]".parse().unwrap();

    // MachineSoft → __Inner_MachineSoft (external MSI hook)
    let sym = if name == "MachineSoft" {
        quote::format_ident!("__Inner_MachineSoft")
    } else {
        name.clone()
    };

    let expanded = quote! {
        #no_mangle_attr
        pub unsafe extern "C" fn #sym(#inputs) {
            #body
        }
    };

    expanded.into()
}
