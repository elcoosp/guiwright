//! The clock seam row (m51): [`ClientConn::clock`] carries the §14
//! virtual-clock domain as typed sugar — one call taking the wire's own
//! [`ClockV1`] enum and answering the [`ClockState`] carrier (virtual time,
//! paused flag, frame-sized steps), the same shape the TypeScript client
//! has spoken since §14 (`clock()` / `fastForward()`) and the REPL since
//! M4 (`clock install` / `ff` / `state`). No wire change: the command
//! rides the `Clock` domain byte-for-byte as §14 blessed it; every
//! [`ClientConn`] impl inherits the row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_clock
//! # prints the state after install, a fast-forward (with its frame-sized
//! # steps), the pause/resume flag flips and the set-time + probe chain
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::ClockV1;

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
            .expect("in-process client connects");
        client
            .handshake("wire-client-clock-demo/0.1")
            .await
            .expect("handshake");

        // Install the virtual clock: the adapter's time source swaps (§14
        // "clean injection" per framework), everything virtual from here.
        let state = client
            .clock(ClockV1::InstallVirtual)
            .await
            .expect("install");
        println!(
            "install → virtual {:?} ms, paused {}, steps {}",
            state.virtual_time_ms, state.paused, state.steps
        );

        // Fast-forward: the jump advances in frame-sized steps (each step
        // pumps a frame so timers settle), and the reply reports both.
        let state = client
            .clock(ClockV1::FastForward { ms: 100 })
            .await
            .expect("fast-forward");
        println!(
            "ff 100  → virtual {:?} ms in {} step(s)",
            state.virtual_time_ms, state.steps
        );

        // Pause / resume: the flag rides every reply, one call each.
        let state = client.clock(ClockV1::Pause).await.expect("pause");
        println!("pause   → paused {}", state.paused);
        let state = client.clock(ClockV1::Resume).await.expect("resume");
        println!("resume  → paused {}", state.paused);

        // The chaining contract: every reply carries the full state, so the
        // next command chains off the state just produced — set lands
        // exactly, the probe (a read, never a mutation) echoes it.
        let state = client
            .clock(ClockV1::SetTime { ms: 120_000 })
            .await
            .expect("set-time");
        println!("set     → virtual {:?} ms", state.virtual_time_ms);
        let state = client.clock(ClockV1::State).await.expect("probe");
        println!(
            "state   → virtual {:?} ms, paused {}, steps {}",
            state.virtual_time_ms, state.paused, state.steps
        );

        handle.shutdown();
    });
}
