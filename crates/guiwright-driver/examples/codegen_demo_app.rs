//! The §5.3 launch-mode demo app — the app half of the `guiwright
//! codegen` pairing (the m4 §4 deferred row): a mock-adapter GWP server
//! over the `GUIWRIGHT_DRIVER` door env, which the launcher injects.
//! The buttons fixture (Save/Cancel, the codegen E2E's tree) is what a
//! recording session sees.
//!
//! ```text
//! cargo run -p guiwright-cli -- codegen --duration-ms 10000 --out demo.rs -- \
//!     cargo run -p guiwright-driver --example codegen_demo_app
//! # drive the app (or let the window end); the launcher stops the
//! # recorder, prints (and writes) the generated kit code, and reaps the
//! # app. Standalone launch (§5.3) works too:
//! GUIWRIGHT_DRIVER=uds:///tmp/demo.gwp cargo run -p guiwright-driver \
//!     --example codegen_demo_app   # then: guiwright repl --uds /tmp/demo.gwp
//! ```

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::spawn_session_server;

fn main() {
    let door = match std::env::var("GUIWRIGHT_DRIVER") {
        Ok(door) => door,
        Err(_unset) => {
            eprintln!(
                "launch mode required: run `guiwright codegen -- cargo run -p \
                 guiwright-driver --example codegen_demo_app` (the launcher \
                 injects GUIWRIGHT_DRIVER, §5.3), or set \
                 GUIWRIGHT_DRIVER=uds:///tmp/demo.gwp"
            );
            std::process::exit(2);
        }
    };
    let Some(path) = door.strip_prefix("uds://") else {
        eprintln!(
            "the demo serves the uds launch form (GUIWRIGHT_DRIVER=uds://…), \
             got `{door}`"
        );
        std::process::exit(2);
    };
    // The server stack runs on tokio (§7 transports); the demo hosts it
    // in a runtime it keeps alive for the process lifetime.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let handle = spawn_session_server(std::path::PathBuf::from(path), || {
            MockAdapter::with_fixture(&Fixture::buttons())
        })
        .expect("demo app binds its door");
        eprintln!(
            "codegen demo app — buttons fixture serving on {:?}; \
             drive it before the capture window ends",
            handle.endpoint(),
        );
        // Serve until the launcher's reap (or Ctrl-C in standalone mode).
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}
