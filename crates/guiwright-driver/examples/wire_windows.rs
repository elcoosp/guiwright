//! Rust-client window parity over the wire (§16 v0.4, M6): the `Window`
//! domain as typed methods on the client seam — [`ClientConn::window_list`]
//! and [`ClientConn::window_use`] — driven over the in-process door with
//! the two-window fixture. The two windows both carry a "Save" button
//! under independent subtree namespaces, so the ambiguous resolve fails
//! unscoped and only the aimed connection's scope tells them apart — the
//! same conversation `window list` / `window use` speak in the REPL and
//! `windowList()` / `windowUse()` in the TypeScript client.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_windows
//! # prints the window targets, the aim ack, and the scoped resolves
//! ```

use guiwright_core::test_util::MultiWindowFixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::{ErrorCode, RequestV1, ResponseV1, SelectorV1, TextMatchV1};
use guiwright_proto::{ClientConn, DuplexClient, GwpError};

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        let fx = MultiWindowFixture::default();
        let w2 = fx.w2.0.to_string();
        let w2_bytes = *fx.w2.0.as_bytes();

        // App side: the two-window fixture over the embedded door (any
        // transport — UDS, TCP, WS — serves the same conversation).
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_multi_window(&fx),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        // Client side: the typed window surface on the seam.
        let mut client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        client.handshake("wire-windows-demo/0.1").await.unwrap();

        println!("window targets:");
        for w in client.window_list().await.expect("window targets") {
            println!(
                "  {} {}{}",
                w.handle,
                w.title,
                if w.focused { " focused" } else { "" }
            );
        }

        // Unscoped, the two "Save" buttons are a strict-mode rejection:
        let unscoped = client
            .request(RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: true,
                        ignore_case: false,
                    }),
                },
                timeout_ms: 2000,
            })
            .await
            .expect_err("strict mode rejects the ambiguity");
        match unscoped {
            GwpError::Rejected { code, message } => {
                println!("unscoped Save → {code:?}: {message}");
            }
            other => panic!("expected the strict-mode rejection, got {other:?}"),
        }

        // Aim at Preview; the connection's later resolves are scoped:
        let aimed = client.window_use(&w2).await.expect("Preview aimed");
        println!("aimed at {aimed}");
        let scoped = client
            .request(RequestV1::Resolve {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: true,
                        ignore_case: false,
                    }),
                },
                timeout_ms: 2000,
            })
            .await
            .unwrap();
        match scoped {
            ResponseV1::Resolved {
                node: guiwright_proto::schema::NodeKeyV1::AccessKit { tree, .. },
                ..
            } => {
                assert_eq!(tree, w2_bytes, "the scoped resolve found Preview's Save");
                println!("scoped Save → Preview's subtree (tree {tree:02x?})");
            }
            other => panic!("expected the scoped resolve, got {other:?}"),
        }

        // A dead handle is the typed rejection, not a string match:
        let dead = client
            .window_use("00000000-dead-beef-0000-000000000000")
            .await
            .expect_err("dead handle");
        match dead {
            GwpError::Rejected {
                code: ErrorCode::UnknownWindow,
                ..
            } => {
                println!("dead handle → typed UnknownWindow");
            }
            other => panic!("expected UnknownWindow, got {other:?}"),
        }

        let bye = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(bye, ResponseV1::Bye);
        println!("bye — window parity rides the seam");

        handle.shutdown();
    });
}
