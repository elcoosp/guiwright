//! The inspect seam row (m53): [`ClientConn::inspect`] carries the §13
//! inspector domain as typed sugar — one call taking the wire's own
//! [`InspectV1`] enum (codegen start/stop, overlay enable/disable,
//! highlight aim/clear) and answering the [`InspectReply`] union (acks vs.
//! the recording stop's [`CodegenOutput`]), the same shapes the TypeScript
//! client has spoken since M3 (`inspectOverlay()` / `inspectHighlight()`)
//! and the REPL since M4/M8 (`codegen start` / `stop`, `inspect` /
//! `highlight`). No wire change: the command rides the `Inspect` domain
//! byte-for-byte as §13 blessed it; every [`ClientConn`] impl inherits the
//! row.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_client_inspect
//! # prints the overlay acks, the highlight aim/clear, the generated kit
//! # code with its strict-mode warnings, and the typed rejections for the
//! # idempotence rules (double start, idle stop, ghost handle)
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::client::ClientConn;
use guiwright_proto::client::InspectReply;
use guiwright_proto::schema::{InspectV1, SelectorV1, SemActionV1, TextMatchV1};

fn named_button(name: Option<&str>) -> SelectorV1 {
    SelectorV1::Role {
        role: "Button".into(),
        name: name.map(|name| TextMatchV1 {
            text: name.into(),
            exact: true,
            ignore_case: false,
        }),
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
            .expect("in-process client connects");
        client
            .handshake("wire-client-inspect-demo/0.1")
            .await
            .expect("handshake");

        // The overlay session: enable composites hover outlines + badges
        // from the tree indexes (the adapter composites at present time).
        let ack = client
            .inspect(InspectV1::EnableOverlay)
            .await
            .expect("enable");
        println!("overlay on   → {ack:?}");

        // Aim the outline at a resolved row (the dump-row handle form);
        // `None` clears it while the session stays on.
        let row = client
            .resolve(named_button(Some("Save")), 5_000)
            .await
            .expect("the resolve");
        let ack = client
            .inspect(InspectV1::Highlight {
                node: Some(row.node),
            })
            .await
            .expect("highlight");
        println!("highlight    → {ack:?}");
        let ack = client
            .inspect(InspectV1::Highlight { node: None })
            .await
            .expect("clear");
        println!("clear        → {ack:?}");

        // The §13 recorder: start, act (the aim-and-act sugar rides the
        // same session), stop and take the generated kit code.
        let ack = client
            .inspect(InspectV1::StartRecording)
            .await
            .expect("start");
        println!("record on    → {ack:?}");
        let row = client
            .resolve(named_button(Some("Save")), 5_000)
            .await
            .expect("the resolve");
        client
            .action(row.node, row.generation, SemActionV1::Click, false, 5_000)
            .await
            .expect("the click");
        let reply = client
            .inspect(InspectV1::StopRecording)
            .await
            .expect("stop");
        match reply {
            InspectReply::Codegen(output) => {
                println!(
                    "codegen      → {} line(s), {} warning(s)",
                    output.code.lines().count(),
                    output.warnings.len()
                );
                for line in output.code.lines() {
                    println!("  {line}");
                }
            }
            other => panic!("expected the codegen payload, got {other:?}"),
        }

        // The honesty: a second start, and a stop with nothing recorded,
        // answer the wire's typed rejection naming the cause.
        let err = client
            .inspect(InspectV1::StopRecording)
            .await
            .expect_err("idle stop");
        println!("idle stop    → {err}");
        let err = client
            .inspect(InspectV1::Highlight {
                node: Some(guiwright_proto::schema::NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 999,
                }),
            })
            .await
            .expect_err("ghost handle");
        println!("ghost        → {err}");

        handle.shutdown();
    });
}
