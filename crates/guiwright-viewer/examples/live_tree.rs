//! The viewer live mode end to end (architecture §12 "live", M18): a
//! [`LiveFeed`] attached to a running session over the embedded door —
//! no socket, no filesystem — building the same scrubber timeline the
//! archive path builds, live. The "app" renames its Save button
//! mid-session through the mock's shared-script channel; the feed pumps
//! the pushed `TreeChanged`, auto-syncs a fresh dump, and the resolved
//! view at the change frame carries the new label.
//!
//! ```text
//! cargo run -p guiwright-viewer --example live_tree
//! # prints the attach report, the live ticks, and the renamed-node view
//! ```
//!
//! The feed is transport-blind ([`guiwright_proto::ClientConn`], M18):
//! swap the [`DuplexClient`] for the UDS, TCP or WS client and the rest
//! of the file is unchanged — the same conversation over any §7 door.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_viewer::live::{LiveChange, LiveFeed};

fn main() {
    let shared: Arc<Mutex<Vec<TreeUpdate>>> = Arc::new(Mutex::new(Vec::new()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("demo runtime");
    runtime.block_on(async move {
        // App side: the buttons fixture over the embedded door, with the
        // shared-script channel the "app thread" pushes changes into.
        let scripts = Arc::clone(&shared);
        let (handle, connect) = GwpServerHandle::spawn_inprocess(
            move || MockAdapter::with_fixture(&Fixture::buttons()).with_shared_scripts(scripts),
            guiwright_driver::SeamRegistry::new(),
        )
        .expect("in-process server starts");

        // Viewer side: attach over any door — handshake, subscribe to the
        // live vocabulary, and the initial dump.
        let client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        let mut feed = LiveFeed::new(client);
        let info = feed.attach("viewer-live-demo/0.1").await.expect("attach");
        println!(
            "attached to {} ({}) — session axis at frame {}, initial dump recorded",
            info.adapter.name,
            info.adapter.framework,
            feed.current_frame()
        );

        let initial = feed.tree_at(feed.current_frame()).expect("initial view");
        println!(
            "live tree: {} nodes; buttons: {}",
            initial.nodes.len(),
            initial
                .nodes
                .iter()
                .filter(|n| n.role == "Button")
                .map(|n| n.label.as_deref().unwrap_or("(unnamed)"))
                .collect::<Vec<_>>()
                .join(", ")
        );

        // App-side change mid-session: the rename lands on the session's
        // next pump and arrives as a pushed TreeChanged.
        let mut save = Node::new(Role::Button);
        save.set_label("Save (renamed)");
        shared.lock().expect("scripts lock").push(TreeUpdate {
            nodes: vec![(NodeId(2), save)],
            tree: None,
            tree_id: accesskit::TreeId::ROOT,
            focus: accesskit::NodeId(2),
        });

        // Pump the live vocabulary: frame ticks fly by, the TreeChanged
        // triggers the auto-sync.
        let mut seen_frames = 0u32;
        let change = loop {
            match feed
                .pump(Duration::from_secs(5))
                .await
                .expect("the pump never errors on a live door")
            {
                Some(LiveChange::FramePresented { frame, hash }) => {
                    seen_frames += 1;
                    let _ = (frame, hash);
                }
                Some(LiveChange::TreeChanged { frame }) => break frame,
                Some(LiveChange::Log) => {}
                None => panic!("quiet window while a rename was pending"),
            }
        };
        println!("rename pumped: {seen_frames} frame tick(s), change synced at frame {change}");

        let view = feed.tree_at(change).expect("the change frame resolves");
        let renamed = view
            .nodes
            .iter()
            .find(|n| n.label.as_deref() == Some("Save (renamed)"))
            .expect("the renamed row after the auto-sync");
        println!(
            "resolved view at frame {change}: role={} label=\"{}\" parent={:?}",
            renamed.role,
            renamed.label.as_deref().expect("the label"),
            renamed.parent
        );

        // The scrubber model is the archive one: same ticks, same
        // resolution — replayable, cacheable, and (unlike the archive)
        // still growing.
        let scrubber = feed.timeline();
        println!(
            "timeline so far: {} tick(s), {} snapshot(s), test label \"{}\"",
            scrubber.len(),
            feed.snapshot_count(),
            scrubber.meta_test,
        );

        feed.bye().await.expect("bye");
        println!("bye — the live feed ends; the door stays open for the next viewer");
        handle.shutdown();
    });
}
