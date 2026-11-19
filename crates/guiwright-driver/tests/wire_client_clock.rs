//! The client seam's clock rows over the wire (m51): [`ClientConn::clock`]
//! carries the §14 virtual-clock domain as typed seam sugar — one call
//! answering the [`ClockState`] carrier (virtual time, paused flag, frame-
//! sized steps), the same shape the TypeScript client has spoken since §14
//! (`clock()` / `fastForward()`) and the REPL since M4 (`clock install` /
//! `ff` / `pause` / `resume` / `set` / `state`). **No wire change**: the
//! sugar rides the `Clock` domain byte-for-byte as §14 blessed it; every
//! [`ClientConn`] impl inherits the row. Driven here over the in-process
//! door — the same reality `wire_clock.rs` pins at the raw-request level.

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::ClockV1;

async fn connected(connect: &DuplexConnect) -> DuplexClient {
    let mut client = DuplexClient::connect(connect)
        .await
        .expect("client connects");
    client
        .handshake("wire-client-clock/0.1")
        .await
        .expect("handshake");
    client
}

/// The probe is total (a read, never a mutation): before the virtual
/// clock exists, the state reads `None` at real time — the same honest
/// shape the REPL's `clock state` row prints.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_probe_reads_the_pre_install_state() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let state = client.clock(ClockV1::State).await.expect("the probe");
    assert_eq!(
        state.virtual_time_ms, None,
        "no virtual clock installed yet"
    );
    assert!(!state.paused, "real time is not paused");
    assert_eq!(state.steps, 0, "a probe applies no jumps");

    handle.shutdown();
}

/// Install + fast-forward: the adapter's time source swaps to virtual
/// (§14 "clean injection"), the jump advances in frame-sized steps, and
/// the reply reports both — the same state the REPL's `clock ff` row
/// renders and the TypeScript `fastForward()` helper chains off.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn install_and_fast_forward_report_the_steps() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let installed = client
        .clock(ClockV1::InstallVirtual)
        .await
        .expect("the install");
    assert_eq!(installed.virtual_time_ms, Some(0), "virtual time at zero");
    assert!(!installed.paused);
    assert_eq!(installed.steps, 0, "the install applies no jumps");

    let jumped = client
        .clock(ClockV1::FastForward { ms: 100 })
        .await
        .expect("the jump");
    assert_eq!(
        jumped.virtual_time_ms,
        Some(100),
        "the whole budget landed on the virtual clock"
    );
    assert!(
        jumped.steps >= 1,
        "the jump reports its frame-sized steps: {}",
        jumped.steps
    );

    handle.shutdown();
}

/// Pause and resume flip the flag in the echoed state — the round-trip
/// affordance the REPL's `clock pause` / `clock resume` rows render, one
/// seam call each.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_and_resume_echo_the_flag() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    client
        .clock(ClockV1::InstallVirtual)
        .await
        .expect("the install");

    let paused = client.clock(ClockV1::Pause).await.expect("the pause");
    assert!(paused.paused, "the state echo carries the flag");
    let probe = client.clock(ClockV1::State).await.expect("the probe");
    assert!(probe.paused, "the pause persisted");

    let resumed = client.clock(ClockV1::Resume).await.expect("the resume");
    assert!(!resumed.paused, "the state echo carries the flag");

    handle.shutdown();
}

/// The chaining contract the wireClock doc records: every reply carries
/// the full state, so a client chains the next command off the state it
/// just produced — set-time lands exactly, the probe echoes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_time_lands_and_the_probe_echoes_it() {
    let (handle, connect) = GwpServerHandle::spawn_inprocess(
        || MockAdapter::with_fixture(&Fixture::buttons()),
        guiwright_driver::SeamRegistry::new(),
    )
    .expect("in-process server starts");

    let mut client = connected(&connect).await;
    let state = client
        .clock(ClockV1::SetTime { ms: 120_000 })
        .await
        .expect("the set");
    assert_eq!(state.virtual_time_ms, Some(120_000));

    let probe = client.clock(ClockV1::State).await.expect("the probe");
    assert_eq!(
        probe.virtual_time_ms,
        Some(120_000),
        "the state persists across the round trip"
    );

    handle.shutdown();
}
