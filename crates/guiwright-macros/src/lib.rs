//! Proc macros for Guiwright (§5.2 `guiwright-kit`: `#[guiwright::test]`,
//! `#[guiwright_kit::seam]`, fixtures).
//!
//! `#[test]` turns a plain function taking `&mut App` into a `#[test]`
//! that launches the embedded harness (§5.1 "embedded mode: guiwright-kit
//! calls the adapter directly in the test's process"), runs the body with
//! per-test artifacts wired (trace file, screenshot baselines, nextest
//! naming, §11 process-per-test) and reports the outcome:
//!
//! ```ignore
//! #[guiwright_kit::test]
//! fn login_flow(app: &mut App) {
//!     app.get_by_role(Role::Button, "Save").click().unwrap();
//! }
//!
//! #[guiwright_kit::test]
//! fn checkout(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
//!     app.get_by_label("Email").fill("ada@example.dev")?;
//!     Ok(())
//! }
//! ```
//!
//! Both unit and `Result` bodies work (a failing `Result` fails the test
//! with the error message). Extra attributes (`#[ignore]`,
//! `#[should_panic]`) are carried over to the generated test.
//!
//! `#[seam]` (§14 "State seams") marks an app function as a reset/init
//! hook for the mock domain. The original function stays untouched; the
//! macro additionally generates `pub fn __gw_seam_<name>(registry)` that
//! installs a hook compiling the function into a `run_in` task — the
//! adapter's runner context is downcast to the state type, so **state
//! writes run in the runner's context** (§14):
//!
//! ```ignore
//! #[guiwright_kit::seam]
//! fn reset_db(state: &mut AppState) -> Result<(), AppError> {
//!     app_state::reset_to_fixture(state)
//! }
//!
//! // app boot: hand the generated registration to the session
//! guiwright_kit::register_seams![reset_db];
//! // test side:
//! app.mock().reset_state("seed-42").unwrap();
//! ```
//!
//! Accepted shapes: `fn name()`, `fn name(state: &mut S)` and
//! `fn name(state: &mut S, scope: &str)`. The return must be
//! `Result<(), E: Display>` (detail `"ok"`) or `Result<String, E>`
//! (detail = the string).

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemFn, ReturnType, parse_macro_input};

/// The `#[test]` attribute (used as `#[guiwright_kit::test]`).
#[proc_macro_attribute]
pub fn test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let name = &input.sig.ident;
    let vis = &input.vis;
    let attrs = &input.attrs;
    let output = &input.sig.output;
    let body = &input.block;
    let Some(app_arg) = input.sig.inputs.first() else {
        return syn::Error::new_spanned(
            &input.sig,
            "#[guiwright_kit::test] functions must take exactly one argument: `app: &mut guiwright_kit::App`",
        )
        .to_compile_error()
        .into();
    };

    // Unit bodies report `()`; Result bodies propagate the error message
    // (any `Result<(), E: Display>` shape works — the standard one).
    let outcome = match &input.sig.output {
        ReturnType::Default => quote! {
            let __gw_outcome = __gw_body(&mut __gw_app);
            guiwright_kit::TestOutcome::report(__gw_outcome, __GW_NAME);
        },
        ReturnType::Type(_, _) => quote! {
            let __gw_outcome = __gw_body(&mut __gw_app);
            match __gw_outcome {
                Ok(()) => guiwright_kit::TestOutcome::report(Ok::<(), String>(()), __GW_NAME),
                Err(__gw_err) => guiwright_kit::TestOutcome::report(
                    Err::<(), String>(__gw_err.to_string()),
                    __GW_NAME,
                ),
            }
        },
    };

    let expanded = quote! {
        #(#attrs)*
        #vis fn #name() {
            const __GW_NAME: &str = stringify!(#name);
            fn __gw_body(#app_arg) #output #body
            let mut __gw_app = guiwright_kit::App::launch_fixture(__GW_NAME);
            #outcome
            __gw_app.finish();
        }
    };
    expanded.into()
}

/// The `#[seam]` attribute (§14 state seams; used as `#[guiwright_kit::seam]`).
#[proc_macro_attribute]
pub fn seam(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    if let Err(err) = validate_seam(&input) {
        return err.to_compile_error().into();
    }

    let name = &input.sig.ident;
    let register_name = quote::format_ident!("__gw_seam_{}", name);
    let id_literal = name.to_string();
    let (call, ok_map) = seam_call(&input);

    let expanded = quote! {
        #input

        /// Registers `#id_literal` into a [`::guiwright_kit::SeamRegistry`]
        /// (§14). The hook compiles into a `run_in` task: the adapter's
        /// runner context is downcast to the state type, so the seam writes
        /// app state **inside the runner's context**.
        pub fn #register_name(registry: &mut ::guiwright_kit::SeamRegistry) {
            registry.register(#id_literal, |
                __gw_ctx: &mut dyn ::std::any::Any,
                __gw_scope: &str,
            | {
                #call
                #ok_map
            });
        }
    };
    expanded.into()
}

/// The `register_seams!` function-like macro (§14 app boot): expands to an
/// expression that builds a [`::guiwright_kit::SeamRegistry`] with the
/// `#[seam]`-generated registration functions:
///
/// ```ignore
/// app.register_seams(&guiwright_kit::register_seams![reset_db, seed_users]);
/// ```
#[proc_macro]
pub fn register_seams(input: TokenStream) -> TokenStream {
    use syn::parse::Parser as _;
    let names = match syn::punctuated::Punctuated::<syn::Ident, syn::token::Comma>::parse_terminated
        .parse(input)
    {
        Ok(p) => p.into_iter().collect::<Vec<_>>(),
        Err(e) => return e.to_compile_error().into(),
    };
    let calls = names.iter().map(|n| {
        let reg_fn = quote::format_ident!("__gw_seam_{}", n);
        quote! { #reg_fn(&mut __gw_registry); }
    });
    // `mut` only when seams are registered — an empty `register_seams![]`
    // must not emit unused-mut warnings into the app's crate.
    let registry = if names.is_empty() {
        quote! { let __gw_registry = ::guiwright_kit::SeamRegistry::new(); }
    } else {
        quote! { let mut __gw_registry = ::guiwright_kit::SeamRegistry::new(); }
    };
    quote! {{
        #registry
        #(#calls)*
        __gw_registry
    }}
    .into()
}

/// Shape validation: return must be `Result<_, E>`; params must be none,
/// `&mut S` or `&mut S, &str`.
fn validate_seam(input: &ItemFn) -> Result<(), syn::Error> {
    let mut params = input.sig.inputs.iter();
    let first = params.next();
    let second = params.next();
    if params.next().is_some() {
        return Err(syn::Error::new_spanned(
            &input.sig,
            "#[seam] takes at most (state: &mut S, scope: &str)",
        ));
    }
    if let Some(arg) = first {
        if !is_mut_ref(fn_arg_type(arg)) {
            return Err(syn::Error::new_spanned(
                arg,
                "#[seam] state parameter must be `&mut S` (the runner context)",
            ));
        }
    }
    if let Some(arg) = second {
        if !is_str_ref(fn_arg_type(arg)) {
            return Err(syn::Error::new_spanned(
                arg,
                "#[seam] scope parameter must be `&str`",
            ));
        }
    }
    match &input.sig.output {
        ReturnType::Default => Err(syn::Error::new_spanned(
            &input.sig,
            "#[seam] must return Result<_, E: Display>",
        )),
        ReturnType::Type(_, ty) if result_ok_type(ty).is_some() => Ok(()),
        ReturnType::Type(_, ty) => Err(syn::Error::new_spanned(
            ty,
            "#[seam] must return Result<_, E: Display>",
        )),
    }
}

fn seam_takes_scope(input: &ItemFn) -> bool {
    input.sig.inputs.len() == 2
}

/// Build the call expression for the registered hook, adapting the
/// arguments (state downcast + optional scope pass-through).
fn seam_call(input: &ItemFn) -> (TokenStream2, TokenStream2) {
    let name = &input.sig.ident;
    let state_ty = input
        .sig
        .inputs
        .first()
        .map(|arg| inner_of_mut_ref(fn_arg_type(arg)));
    let has_scope = seam_takes_scope(input);

    let call = match (&state_ty, has_scope) {
        (None, false) => quote! { let __gw_result = #name(); },
        // Both validated-shape holes: `#[seam]` rejects these combos.
        (None, true) | (Some(None), _) => unreachable!("validated"),
        (Some(Some(ty)), false) => quote! {
            let __gw_state = __gw_ctx
                .downcast_mut::<#ty>()
                .ok_or("wrong runner context type")?;
            let __gw_result = #name(__gw_state);
        },
        (Some(Some(ty)), true) => quote! {
            let __gw_state = __gw_ctx
                .downcast_mut::<#ty>()
                .ok_or("wrong runner context type")?;
            let __gw_result = #name(__gw_state, __gw_scope);
        },
    };

    // `Result<(), E>` → detail "ok"; `Result<String, E>` → the string.
    let ok_ty = match &input.sig.output {
        ReturnType::Type(_, ty) => result_ok_type(ty).expect("validated as Result"),
        ReturnType::Default => unreachable!("validated"),
    };
    let ok_map = if is_unit(ok_ty) {
        quote! { __gw_result.map(|()| ::std::string::String::from("ok")) }
    } else {
        quote! { __gw_result.map(|__gw_detail| ::std::string::ToString::to_string(&__gw_detail)) }
    };
    (call, ok_map)
}

// --- small syn helpers ------------------------------------------------------

fn fn_arg_type(arg: &syn::FnArg) -> &syn::Type {
    match arg {
        syn::FnArg::Typed(pat) => &pat.ty,
        syn::FnArg::Receiver(_) => {
            // A `self` receiver fails the is_mut_ref/is_str_ref checks and
            // produces a spanned error; seam functions never take self.
            unreachable!("seam functions do not take receivers")
        }
    }
}

fn is_unit(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Tuple(t) if t.elems.is_empty())
}

fn is_mut_ref(ty: &syn::Type) -> bool {
    matches!(
        ty,
        syn::Type::Reference(r) if r.mutability.is_some()
    )
}

fn is_str_ref(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Reference(r)
        if r.mutability.is_none()
            && matches!(&*r.elem, syn::Type::Path(p)
                if p.path.is_ident("str")))
}

/// `&mut S` → `S` (None when `ty` is not a `&mut` reference).
fn inner_of_mut_ref(ty: &syn::Type) -> Option<&syn::Type> {
    match ty {
        syn::Type::Reference(r) if r.mutability.is_some() => Some(&r.elem),
        _ => None,
    }
}

/// `Result<Ok, E>` → `Ok` type (None when `ty` is not a `Result`).
fn result_ok_type(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(p) = ty else {
        return None;
    };
    let last = p.path.segments.last()?;
    if last.ident != "Result" {
        return None;
    }
    match &last.arguments {
        syn::PathArguments::AngleBracketed(args) => args.args.first().and_then(|a| match a {
            syn::GenericArgument::Type(t) => Some(t),
            _ => None,
        }),
        _ => None,
    }
}
