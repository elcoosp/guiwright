//! The describe-all wire domain (M39): the m37 record's standing gap —
//! "enumerating a set's facts would open one `Describe` round trip per
//! row" — closed as `DescribeAll` → `DescribedAll` over GWP, with the
//! typed [`ClientConn::describe_all`] sugar. "What IS everything in the
//! set" is the debugging read the REPL's `all` rows and viewer-style
//! clients speak: the resolve-all walk's rows carrying the identity
//! facts (role/name/value/bounds) — one round trip instead of N, each
//! row still directly usable as the `Action` node (the dump-row rule).
//! The set read is total: an empty selector answers an empty list, and
//! the connection's window scope narrows the walk exactly like
//! `ResolveAll`.
//!
//! ```text
//! cargo run -p guiwright-driver --example wire_describe_all
//! # prints every button's fact row in document order, the input's value
//! # row, and the empty set's total answer
//! ```

use guiwright_core::test_util::Fixture;
use guiwright_driver::mock::MockAdapter;
use guiwright_driver::server::GwpServerHandle;
use guiwright_proto::schema::SelectorV1;
use guiwright_proto::{ClientConn, DuplexClient};

fn role(role: &str) -> SelectorV1 {
    SelectorV1::Role {
        role: role.into(),
        name: None,
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
            .handshake("wire-describe-all-demo/0.1")
            .await
            .expect("handshake");

        // The set read carrying facts: every current match of the
        // selector, document order, one round trip — the identity facts
        // the single-node `Describe` read returns, at set width.
        let rows = client
            .describe_all(role("Button"))
            .await
            .expect("describe-all round-trip");
        println!("buttons — {} fact rows:", rows.len());
        for row in &rows {
            let name = row.name.as_deref().unwrap_or("(unnamed)");
            let bounds = row
                .bounds
                .map(|[x0, y0, x1, y1]| format!("rect=[{x0:.0},{y0:.0} → {x1:.0},{y1:.0}]"))
                .unwrap_or_default();
            println!(
                "  {} {name:?} (generation {}) {bounds}",
                row.role, row.generation
            );
        }

        // The value rides the row: one pass answers "what does the set
        // carry" without a second round trip per input.
        let inputs = client
            .describe_all(role("TextInput"))
            .await
            .expect("describe-all round-trip");
        for row in &inputs {
            let value = row.value.as_deref().unwrap_or("(no value)");
            println!("input — {} {value:?}", row.role);
        }

        // The read is total: an empty set answers an empty list, never
        // an error (the m30 snapshot rule).
        let empty = client
            .describe_all(SelectorV1::TestId("ghost".into()))
            .await
            .expect("the empty set read");
        println!("empty set → {} rows (total, never an error)", empty.len());

        handle.shutdown();
    });
}
