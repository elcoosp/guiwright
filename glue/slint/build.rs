//! Build script for the slint glue workspace: compiles the real
//! `counter_form.slint` consumer app with element debug info, so the
//! testing backend's introspection (`ElementHandle::id` / `type_name`)
//! sees element names — the §4.4/§11 debug-info flag. Apps written
//! outside this workspace do the same through the
//! `guiwright-adapter-slint::build` helper (`TestingBuild` +
//! `cargo_instructions`) or `SLINT_EMIT_DEBUG_INFO=1` in their build
//! environment.

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_debug_info(true);
    slint_build::compile_with_config("examples/counter_form.slint", config)
        .expect("compile the counter_form example app with element debug info");
}
