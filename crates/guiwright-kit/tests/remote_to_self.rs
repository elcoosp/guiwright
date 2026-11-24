//! The kit-side "remote-to-self" handle (§7's embedded door at kit
//! width — the m16 record's remaining-gaps list, restated unchanged by
//! the m18 record's honest list: *"A kit-level embedded handle — the
//! kit drives sessions through its own adapter surface; wiring
//! `spawn_inprocess` into a kit-side 'remote-to-self' handle is a
//! follow-on, deliberately not squeezed into this milestone"*).
//!
//! The acceptance this pins: **one kit call brings the whole remote
//! stack up in-process** — the session thread, the GWP bridge and a
//! typed [`ClientConn`] client joined to it over the duplex door — no
//! socket, no filesystem, the conversation byte-identical to the UDS,
//! TCP and WS doors (the m16 contract). The kit's embedded `App` calls
//! the adapter directly; this handle is the missing other half: the
//! same process, the full wire path.

use guiwright_core::selector::{Selector, TextMatch, TextPattern};
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::Endpoint;
use guiwright_kit::KitError;
use guiwright_kit::remote::RemoteToSelf;
use guiwright_proto::schema::{ErrorCode, NodeKeyV1, SelectorV1, SemActionV1};

/// A Role selector by exact name, in the kit's own vocabulary — the
/// `From<Selector> for SelectorV1` bridge the handle rides (the kit
/// speaks core selectors; the door speaks the wire vocabulary).
fn role_named(role: accesskit::Role, name: &str) -> Selector {
    Selector::Role {
        role,
        name: Some(TextMatch {
            pattern: TextPattern::Str(name.into()),
            exact: true,
            ignore_case: false,
        }),
    }
}

/// The same Role selector in the wire's vocabulary, name-free — the
/// role-wide set walk (the M39 domain's non-strict read).
fn wire_role(role: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: None,
    }
}

/// The m16 acceptance: one call brings the stack up in-process — the
/// endpoint reports [`Endpoint::InProcess`], the Welcome names the
/// fixture adapter, and the door's facts are the handle's facts.
#[test]
fn launch_fixture_brings_the_stack_up_in_process() {
    let rts = RemoteToSelf::launch_fixture();
    assert_eq!(rts.endpoint(), &Endpoint::InProcess);
    let adapter = rts.adapter();
    assert_eq!(adapter.name, "mock");
    assert_eq!(adapter.framework, "fixture");
    assert!(rts.agent().contains("guiwright-kit"), "agent names the kit");
    rts.finish().expect("the door shuts down cleanly");
}

/// The typed seam answers over the door: resolve (kit selector in,
/// wire handle out), describe (the M36 identity row), act (the m47
/// aim-and-act pair) — the §5.1 rule, remote sessions behave like the
/// embedded surface, now exercised from the kit without a socket.
#[test]
fn typed_seam_rows_answer_over_the_door() {
    let mut rts = RemoteToSelf::launch_fixture();
    let resolved = rts
        .resolve(role_named(accesskit::Role::Button, "Save"), 2_000)
        .expect("the strict resolve crosses the door");
    let NodeKeyV1::AccessKit { node, .. } = resolved.node else {
        panic!("the mock fixture speaks AccessKit handles");
    };
    assert_eq!(node, 2, "the fixture's Save button");

    let row = rts
        .describe(resolved.node.clone(), resolved.generation)
        .expect("the M36 describe crosses the door");
    assert_eq!(row.role, "Button");
    assert_eq!(row.name.as_deref(), Some("Save"));

    let receipt = rts
        .act(
            resolved.node,
            resolved.generation,
            SemActionV1::Click,
            false,
            2_000,
        )
        .expect("the m47 act crosses the door");
    assert_eq!(receipt.detail, "clicked");
    rts.finish().expect("shutdown joins");
}

/// The §8.3 actionability gate rides the door unchanged: a non-force
/// act on the fixture's hidden button refuses with the typed `Timeout`,
/// naming the stuck check — the same honesty a remote suite meets.
#[test]
fn the_actionability_gate_rides_the_door() {
    let mut rts = RemoteToSelf::launch_fixture();
    let hidden = rts
        .resolve(role_named(accesskit::Role::Button, "Invisible"), 2_000)
        .expect("the hidden node resolves (it exists, it is just hidden)");
    let err = rts
        .act(
            hidden.node,
            hidden.generation,
            SemActionV1::Click,
            false,
            300,
        )
        .expect_err("a hidden node is never clicked");
    let KitError::Wire(guiwright_proto::GwpError::Rejected { code, message }) = err else {
        panic!("the typed wire refusal crosses the door: {err:?}");
    };
    assert_eq!(code, ErrorCode::Timeout);
    assert!(
        message.contains("stuck on visible"),
        "the failing check is named: {message}"
    );
    rts.finish().expect("shutdown joins");
}

/// Set reads over the door: `describe_all` walks the fixture's buttons
/// set in one round trip (the M39 domain), rows carrying the identity
/// facts — the kit's bulk readers' wire twin, reachable from kit tests.
#[test]
fn set_reads_walk_the_fixture_over_the_door() {
    let mut rts = RemoteToSelf::launch_fixture();
    let rows = rts
        .describe_all(wire_role("Button"))
        .expect("the M39 walk crosses the door");
    let names: Vec<_> = rows.iter().filter_map(|r| r.name.as_deref()).collect();
    assert!(
        names.contains(&"Save"),
        "the fixture buttons ride: {names:?}"
    );
    assert!(
        names.contains(&"Cancel"),
        "the fixture buttons ride: {names:?}"
    );
    rts.finish().expect("shutdown joins");
}

/// The connect handle's honesty survives `finish`: after the door
/// shuts down, a later client connect fails with the typed
/// "listener is gone" error instead of queueing into the void (the
/// duplex door's documented contract). The later connect rides its own
/// runtime — the sync test thread never nests runtimes (the handle's
/// documented boundary: bring the door up on a sync thread, not inside
/// an async context).
#[test]
fn finish_shuts_the_door_and_later_connects_fail_honestly() {
    // The handle hands out the connect half so hosts can attach more
    // clients (the kit's live capture rides exactly this); the clone
    // escapes the block through the tuple.
    let (report, connect) = {
        let mut rts = RemoteToSelf::launch_fixture();
        let connect = rts.connect_half().clone();
        rts.resolve(role_named(accesskit::Role::Button, "Save"), 2_000)
            .expect("the door is live");
        (rts.finish().expect("shutdown joins"), connect)
    };
    assert_eq!(report.endpoint, Endpoint::InProcess);
    assert_eq!(report.adapter, "mock");
    assert_eq!(report.framework, "fixture");
    assert!(report.agent.contains("guiwright-kit"));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the probe runtime");
    let err = runtime
        .block_on(guiwright_proto::DuplexClient::connect(&connect))
        .expect_err("the finished door refuses later connects");
    assert!(
        err.to_string().contains("listener is gone"),
        "the honest connect refusal: {err}"
    );
}

/// `launch` takes any adapter factory (the m16 wording: "the kit drives
/// sessions through its own adapter surface") — the adapter is built on
/// the session thread, exactly the `spawn_inprocess` shape.
#[test]
fn launch_takes_any_adapter_factory() {
    let mut rts = RemoteToSelf::launch(|| {
        let mut adapter = MockAdapter::with_fixture(&fixture());
        adapter.push_frame(1);
        adapter.push_frame(1);
        adapter
    })
    .expect("the door comes up");
    let rows = rts
        .describe_all(wire_role("TextInput"))
        .expect("the walk answers");
    assert!(
        rows.iter().any(|r| r.name.as_deref() == Some("Email")),
        "the factory's fixture answers: {:?}",
        rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()
    );
    rts.finish().expect("shutdown joins");
}

fn fixture() -> guiwright_core::test_util::Fixture {
    guiwright_core::test_util::Fixture::buttons()
}
