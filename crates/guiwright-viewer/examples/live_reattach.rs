//! The §7 resume vocabulary end to end (the m18 record's remaining-gaps
//! list: *"The feed is one-shot per connection. After `Bye` the
//! connection is done; a reconnecting viewer re-attaches (and re-dumps)
//! — the session-side resume vocabulary (a `Subscribe` replay cursor)
//! stays a §7 follow-on"*): a [`LiveFeed`] rides one connection to the
//! mid-session rename, the connection ends the m18 way (`bye`), the app
//! moves on while nobody watches — and the reattach resumes from the
//! feed's cursor instead of paying the attach dump again. The quiet
//! reattach replays nothing (the cursor already names the position —
//! the snapshot count does not move); the reattach after a change
//! replays the coalesced position (one auto-sync, the new label).
//!
//! ```text
//! cargo run -p guiwright-viewer --example live_reattach
//! # prints the cursor the first feed left behind, the quiet reattach's
//! # no-op, the replayed position after the detached change, and the
//! # resolved view carrying the second rename
//! ```
//!
//! The feed is transport-blind ([`guiwright_proto::ClientConn`]): swap
//! the [`DuplexClient`] for the UDS, TCP or WS client and the rest of
//! the file is unchanged — the reattach conversation rides any §7 door.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use accesskit::{Node, NodeId, Role, TreeUpdate};
use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::DuplexClient;
use guiwright_viewer::live::{LiveChange, LiveFeed};

/// Script a rename of the fixture's Save button (the late-change
/// pattern from the `live_tree` demo): one pump integrates the diff.
fn rename(label: &str) -> TreeUpdate {
    let mut save = Node::new(Role::Button);
    save.set_label(label);
    save.set_bounds(accesskit::Rect {
        x0: 16.0,
        y0: 16.0,
        x1: 136.0,
        y1: 48.0,
    });
    TreeUpdate {
        nodes: vec![(NodeId(2), save)],
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(2),
    }
}

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

        // First feed: attach, watch one rename — the timeline this feed
        // owns is what the reattach keeps.
        let client = DuplexClient::connect(&connect)
            .await
            .expect("client connects");
        let mut feed = LiveFeed::new(client);
        let info = feed
            .attach("viewer-reattach-demo/0.1")
            .await
            .expect("attach");
        println!(
            "attached to {} — the attach dump anchored the cursor",
            info.adapter.name
        );
        shared
            .lock()
            .expect("scripts lock")
            .push(rename("Save (renamed)"));
        loop {
            match feed
                .pump(Duration::from_secs(5))
                .await
                .expect("the pump never errors on a live door")
            {
                Some(LiveChange::TreeChanged { .. }) => break,
                Some(_) => {}
                None => panic!("quiet window while a rename was pending"),
            }
        }
        let cursor = feed.resume_cursor();
        println!(
            "first connection ends — cursor: tree_seq {}, frame_seq {}",
            cursor.tree_seq, cursor.frame_seq
        );

        // The m18 one-shot rule: `bye` ends the conversation.
        feed.bye().await.expect("bye");

        // The quiet reattach: nothing changed while nobody watched, so
        // the cursor already names the current position — the replay is
        // empty and the attach dump is skipped (the snapshot count does
        // not move).
        let snapshots_before = feed.snapshot_count();
        let client = DuplexClient::connect(&connect)
            .await
            .expect("second client connects");
        feed.reattach(client, "viewer-reattach-demo/0.1")
            .await
            .expect("the quiet reattach resumes");
        println!(
            "quiet reattach: replayed nothing, snapshots {} → {} (the re-dump skipped)",
            snapshots_before,
            feed.snapshot_count()
        );
        feed.bye().await.expect("bye");

        // The change lands while nobody is attached — the missed move
        // the re-dump used to bill unconditionally.
        shared
            .lock()
            .expect("scripts lock")
            .push(rename("Save (renamed twice)"));

        // The reattach after the detached change: the session stands
        // past the cursor, so the replay carries the coalesced position
        // and the drain's auto-sync records the new dump.
        let snapshots_before = feed.snapshot_count();
        let client = DuplexClient::connect(&connect)
            .await
            .expect("third client connects");
        feed.reattach(client, "viewer-reattach-demo/0.1")
            .await
            .expect("the reattach resumes past the cursor");
        let synced = feed.snapshot_count();
        println!(
            "resume reattach: the position replayed — snapshots {} → {} (one auto-sync)",
            snapshots_before, synced
        );

        // The kept timeline answers at the newest frame: the second
        // rename is there, and the first one never left.
        let frame = feed.current_frame();
        let view = feed.tree_at(frame).expect("the newest frame resolves");
        let label = view
            .nodes
            .iter()
            .find(|n| n.role == "Button")
            .and_then(|n| n.label.clone())
            .expect("the renamed button");
        println!(
            "resolved view at frame {frame}: the button reads \"{label}\" — the timeline the \
             first connection built survives the reattach"
        );

        feed.bye().await.expect("bye");
        println!("bye — the door stays open for the next viewer");
        handle.shutdown();
    });
}
