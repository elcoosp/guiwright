//! The clock wire domain (§14): the virtual clock a remote client
//! drives — `Clock::InstallVirtual | FastForward | RunFor | Pause |
//! Resume | SetTime | State` over GWP, the commands the REPL's `clock`
//! family and the TypeScript `clock()`/`fastForward()` helpers speak.
//! The reply always carries the full `ClockState` (virtual time, paused
//! flag, frame-sized steps the servicing command applied), so a client
//! can chain commands off the state it just produced; the mock adapter's
//! clock is the §14 reference semantics (fast-forward advances virtual
//! time in frame-sized steps until drained).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_clock
//! # prints the clock state after install, a fast-forward, the state
//! # probe, and the pause/resume pair
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::schema::{ClockV1, RequestV1, ResponseV1};

fn render(state: &ResponseV1) -> String {
    match state {
        ResponseV1::ClockState {
            virtual_time_ms,
            paused,
            steps,
        } => format!(
            "virtual {:?} paused {paused} steps {steps}",
            virtual_time_ms
        ),
        other => format!("unexpected reply: {other:?}"),
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            || MockAdapter::with_fixture(&Fixture::buttons()),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client
            .handshake("wire-clock-demo/0.1")
            .await
            .expect("handshake");

        // Install the virtual clock: the adapter's time source is swapped
        // (§14 "clean injection" per framework), everything virtual from
        // here.
        let state = client
            .request(RequestV1::Clock(ClockV1::InstallVirtual))
            .await
            .expect("clock installed");
        println!("install        → {}", render(&state));

        // Fast-forward: virtual time advances in frame-sized steps until
        // timers + animations settle (the `steps` count is the drain).
        let state = client
            .request(RequestV1::Clock(ClockV1::FastForward { ms: 500 }))
            .await
            .expect("fast forward");
        println!("ff 500 ms      → {}", render(&state));

        // The state probe reads without mutating (the REPL `clock state`
        // row).
        let state = client
            .request(RequestV1::Clock(ClockV1::State))
            .await
            .expect("clock probed");
        println!("state          → {}", render(&state));

        // Pause / resume: real-time pacing is refused while paused, so a
        // suite can freeze mid-flow and let go deterministically.
        let state = client
            .request(RequestV1::Clock(ClockV1::Pause))
            .await
            .expect("clock paused");
        println!("pause          → {}", render(&state));
        let state = client
            .request(RequestV1::Clock(ClockV1::Resume))
            .await
            .expect("clock resumed");
        println!("resume         → {}", render(&state));

        // Absolute set (§7 `SetTime(t)`) — the trace's `clock.bin` ledger
        // replays these jumps.
        let state = client
            .request(RequestV1::Clock(ClockV1::SetTime { ms: 1_000 }))
            .await
            .expect("clock set");
        println!("set 1000 ms    → {}", render(&state));

        handle.shutdown();
    });
}
