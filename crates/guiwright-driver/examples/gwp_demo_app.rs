//! The other-language-client demo app (§16 "Other-language clients", M3):
//! a mock-adapter GWP server over TCP — or, since M15, over WebSocket
//! with `--ws` — that a TypeScript client drives in the cross-language
//! e2e (`clients/typescript/scripts/e2e.ts`).
//!
//! ```text
//! cargo run -p guiwright-driver --example gwp_demo_app -- --port 0
//! # prints: GWP_TCP_ADDR=127.0.0.1:43210
//! cargo run -p guiwright-driver --example gwp_demo_app -- --port 0 --ws
//! # prints: GWP_WS_ADDR=127.0.0.1:43211
//! ```
//!
//! The app serves the buttons fixture (Save/Cancel), the virtual clock and
//! two state seams — exactly the surface the TS client covers: handshake →
//! resolve → act → clock → mock → bye.

use std::time::Duration;

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let port: u16 = args
        .windows(2)
        .find_map(|w| (w[0] == "--port").then(|| w[1].parse().ok()))
        .flatten()
        .unwrap_or(0);
    let ws = args.iter().any(|flag| flag == "--ws");

    #[derive(Debug, Default)]
    struct DemoState {
        rows: usize,
        fixture: Option<String>,
    }

    let mut seams = guiwright_driver::SeamRegistry::new();
    seams.register("reset_state", |ctx, scope| {
        let state = ctx
            .downcast_mut::<DemoState>()
            .ok_or("wrong runner context type")?;
        state.rows = 0;
        state.fixture = Some(scope.to_string());
        Ok(format!("state reset to {scope}"))
    });
    seams.register("seed_users", |ctx, _scope| {
        let state = ctx
            .downcast_mut::<DemoState>()
            .ok_or("wrong runner context type")?;
        state.rows = 3;
        Ok("3 users seeded".into())
    });

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    // The server stack runs on tokio (§7 transports); the demo hosts it in
    // a runtime it keeps alive for the process lifetime.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    let handle = runtime.block_on(async {
        let adapter = move || {
            // Scripted pixels give the adapter the §6.2 pixel row, so
            // the video domain works end to end (§10).
            MockAdapter::with_fixture(&Fixture::buttons())
                .with_frame_pixels(32, 16, vec![64u8; 32 * 16 * 4])
                .with_state(DemoState {
                    rows: 9,
                    fixture: None,
                })
        };
        if ws {
            GwpServerHandle::spawn_ws(addr, adapter, seams)
                .expect("demo app binds a WebSocket server")
        } else {
            GwpServerHandle::spawn_tcp(addr, adapter, seams).expect("demo app binds a TCP server")
        }
    });
    let (label, bound) = match handle.endpoint() {
        guiwright_driver::server::Endpoint::Ws(addr) => ("GWP_WS_ADDR", *addr),
        guiwright_driver::server::Endpoint::Tcp(addr) => ("GWP_TCP_ADDR", *addr),
        other => panic!("expected a TCP or WS endpoint, got {other:?}"),
    };
    println!("{label}={bound}");
    // Keep the process alive; the client closes the session with Bye.
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
