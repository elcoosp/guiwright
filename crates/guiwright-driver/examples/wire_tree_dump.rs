//! The tree-dump wire domain (M11/M13): the read-side counterpart of
//! resolution as typed rows — `Snapshot::Full` walks every window's
//! subtree depth-first (§16 v0.4: one walk per window root), and
//! `Snapshot::Filtered` resolves a selector first (the same auto-waiting
//! resolve `Resolve`/`Screenshot::Element` run) and dumps only that
//! subtree. Each `SnapshotNodeV1` row carries identity + role + name +
//! value + bounds + child count — a dump row is directly usable as an
//! `Action`/`Inspect` handle (the dump-row rule, M11), and DFS order +
//! `children` rebuild the hierarchy without parent pointers.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_tree_dump
//! # prints the full dump summary, the filtered subtree walk, and one
//! # row reused as an inspect-highlight handle
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_proto::schema::{RequestV1, ResponseV1, SelectorV1, SnapshotV1};

fn describe(key: &guiwright_proto::schema::NodeKeyV1) -> String {
    match key {
        guiwright_proto::schema::NodeKeyV1::AccessKit { node, .. } => format!("accesskit#{node}"),
        other => format!("{other:?}"),
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
            .handshake("wire-tree-dump-demo/0.1")
            .await
            .expect("handshake");

        // The full dump: every window's subtree, depth-first.
        let done = client
            .request(RequestV1::Snapshot(SnapshotV1::Full))
            .await
            .expect("full dump");
        if let ResponseV1::TreeDump {
            tree_seq,
            node_count,
            focus,
            nodes,
        } = done
        {
            println!(
                "full dump @ tree {tree_seq}: {node_count} nodes, focus {}",
                focus
                    .as_ref()
                    .map(describe)
                    .unwrap_or_else(|| "none".into())
            );
            for row in &nodes {
                println!(
                    "  {} {:?}{}",
                    describe(&row.key),
                    row.role,
                    row.name
                        .as_ref()
                        .map(|n| format!(" {n:?}"))
                        .unwrap_or_default()
                );
            }

            // The dump-row rule: a row's identity speaks the same
            // vocabulary every handle-bearing request takes — highlight
            // the Save row (the REPL's `inspect` command over the wire),
            // no second resolve.
            let save = nodes
                .iter()
                .find(|row| row.role == "Button" && row.name.as_deref() == Some("Save"))
                .expect("the fixture's Save row");
            client
                .request(RequestV1::Inspect(
                    guiwright_proto::schema::InspectV1::Highlight {
                        node: Some(save.key.clone()),
                    },
                ))
                .await
                .expect("highlight on the dump row");
            println!(
                "highlight on dump row {} → overlay aimed",
                describe(&save.key)
            );
        }

        // The filtered dump (M13): resolve `role Button`, dump only that
        // subtree — the auto-waiting resolve rides the same selector
        // vocabulary every other wire read takes.
        let done = client
            .request(RequestV1::Snapshot(SnapshotV1::Filtered {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(guiwright_proto::schema::TextMatchV1 {
                        text: "Cancel".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 2000,
            }))
            .await
            .expect("filtered dump");
        if let ResponseV1::TreeDump {
            tree_seq,
            node_count,
            nodes,
            ..
        } = done
        {
            println!("filtered dump @ tree {tree_seq}: {node_count} nodes");
            for row in &nodes {
                println!(
                    "  {} {:?}{} value {:?}",
                    describe(&row.key),
                    row.role,
                    row.name
                        .as_ref()
                        .map(|n| format!(" {n:?}"))
                        .unwrap_or_default(),
                    row.value
                );
            }
        }
        handle.shutdown();
    });
}
