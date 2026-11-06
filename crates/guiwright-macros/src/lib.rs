//! Proc macros for Guiwright (§5.2 `guiwright-kit`: "#[guiwright::test],
//! fixtures").
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

use proc_macro::TokenStream;
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
    let app_arg = match input.sig.inputs.first() {
        Some(first) => first,
        None => {
            return syn::Error::new_spanned(
                &input.sig,
                "#[guiwright_kit::test] functions must take exactly one argument: `app: &mut guiwright_kit::App`",
            )
            .to_compile_error()
            .into();
        }
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
