//! The aim-and-act seam rows (m47): [`ClientConn::resolve`] +
//! [`ClientConn::action`] carry the §7 locate-and-act loop as typed
//! sugar — a strict-mode resolve with the driver's auto-wait answering
//! an `Action`-usable row (the dump-row rule, M11, made typable), and
//! the action applied on that row, answering the frame it landed at.
//! No wire change: both ride the `Resolve` and `Action` domains
//! byte-for-byte as M0/M17 blessed them; every [`ClientConn`] impl
//! inherits the rows, the same shape the TypeScript client has spoken
//! since M3 (`resolve()` / `action()`).
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_resolve_act
//! # prints the resolved row, the click's receipt (frame + detail), and
//! # the typed strict-mode rejection for the ambiguous locator
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::client::ClientConn;
use guiwright_proto::schema::{ErrorCode, SelectorV1, SemActionV1, TextMatchV1};
use guiwright_proto::{DuplexClient, GwpError};

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
            .handshake("wire-resolve-act-demo/0.1")
            .await
            .expect("handshake");

        // Locate: one strict resolve with the driver's auto-wait —
        // handle, generation, bounds, ready to aim.
        let row = client
            .resolve(named_button(Some("Save")), 2000)
            .await
            .expect("the Save row resolves");
        println!(
            "resolved — generation {} bounds {:?}",
            row.generation, row.rect
        );

        // Act: the click rides the row — applied at a named frame, the
        // receipt in one answer.
        let receipt = client
            .action(
                row.node.clone(),
                row.generation,
                SemActionV1::Click,
                false,
                5000,
            )
            .await
            .expect("the click lands");
        println!(
            "clicked — {detail:?} (frame {frame})",
            detail = receipt.detail,
            frame = receipt.frame_seq
        );

        // Strict mode is the resolve's own policy: the bare role locator
        // matches four buttons and answers the typed rejection — never a
        // first-match guess.
        match client.resolve(named_button(None), 200).await {
            Err(GwpError::Rejected {
                code: ErrorCode::StrictModeViolation,
                message,
            }) => println!("strict mode — {message}"),
            other => panic!("expected the typed rejection, got {other:?}"),
        }

        handle.shutdown();
    });
}
