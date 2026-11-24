//! The kit-side "remote-to-self" handle (§7's embedded door at kit
//! width — the m16 record's remaining-gaps list: *"A kit-level embedded
//! handle — the kit drives sessions through its own adapter surface;
//! wiring `spawn_inprocess` into a kit-side 'remote-to-self' handle is
//! a follow-on, deliberately not squeezed into this milestone"*): one
//! call brings the whole remote stack up in-process — the session
//! thread, the GWP bridge and a typed [`ClientConn`] client joined to
//! it over the duplex door — no socket, no filesystem, the conversation
//! byte-identical to the UDS, TCP and WS doors.
//!
//! ```text
//! cargo run -p guiwright-kit --example remote_to_self
//! ```
//!
//! Resolves the fixture's Save button through the door (the kit's core
//! selector vocabulary riding the `From<Selector> for SelectorV1`
//! bridge), reads its identity row (the M36 describe), clicks it (the
//! m47 aim-and-act pair, the actionability gate riding unchanged),
//! walks the fixture's button set in one round trip (the M39 walk) and
//! shuts the door down, printing the report — everything the remote
//! seams gained since M3, reachable from a kit test without leaving
//! the process.

use accesskit::Role;
use guiwright_core::selector::{Selector, TextMatch, TextPattern};
use guiwright_kit::RemoteToSelf;
use guiwright_proto::schema::{SelectorV1, SemActionV1};

fn main() {
    // One call: server + bridge + typed client, in-process.
    let mut door = RemoteToSelf::launch_fixture();
    println!("door up: {:?}", door.endpoint());
    println!(
        "adapter: {} ({})",
        door.adapter().name,
        door.adapter().framework
    );
    println!("agent: {}", door.agent());

    // The kit's selector vocabulary crosses the bridge untouched.
    let save = door
        .resolve(
            Selector::Role {
                role: Role::Button,
                name: Some(TextMatch {
                    pattern: TextPattern::Str("Save".into()),
                    exact: true,
                    ignore_case: false,
                }),
            },
            2_000,
        )
        .expect("strict resolve over the door");
    println!("resolved: {save:?}");

    // The M36 identity row answers for the handle the door just gave.
    let row = door
        .describe(save.node.clone(), save.generation)
        .expect("describe over the door");
    println!("described: {} named {:?}", row.role, row.name);

    // The m47 act half: the actionability gate rides unchanged.
    let receipt = door
        .act(save.node, save.generation, SemActionV1::Click, false, 2_000)
        .expect("act over the door");
    println!("acted at frame {}: {}", receipt.frame_seq, receipt.detail);

    // The M39 set walk: the fixture's whole button set, one round trip.
    let rows = door
        .describe_all(SelectorV1::Role {
            role: "Button".into(),
            name: None,
        })
        .expect("describe-all over the door");
    println!(
        "buttons: {:?}",
        rows.iter()
            .filter_map(|r| r.name.as_deref())
            .collect::<Vec<_>>()
    );

    // Graceful shutdown joins the session thread; later connects fail
    // honestly ("the in-process listener is gone").
    let report = door.finish().expect("the door shuts down");
    println!(
        "done: adapter {} on {:?} as {}",
        report.adapter, report.endpoint, report.agent
    );
}
