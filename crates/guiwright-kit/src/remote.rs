//! The kit-side "remote-to-self" handle (§7's embedded door at kit
//! width — the m16 record's remaining-gaps list: *"A kit-level embedded
//! handle — the kit drives sessions through its own adapter surface;
//! wiring `spawn_inprocess` into a kit-side 'remote-to-self' handle is
//! a follow-on, deliberately not squeezed into this milestone"*, a list
//! the m18 record's honest list restates unchanged).
//!
//! The kit's [`App`](crate::App) drives the adapter directly (§5.1
//! embedded mode); this handle is the missing other half: **one kit
//! call brings the whole remote stack up in-process** — the session
//! thread, the GWP bridge and a typed [`ClientConn`] client joined to
//! it over the duplex door — no socket, no filesystem, the conversation
//! byte-identical to the UDS, TCP and WS doors (the m16 contract).
//! Everything the remote seams gained since M3 — the resolve/act pair
//! (m47), the describe family (M36–m70), the bulk reads (m46) — answers
//! here from a kit test, so suite authors can exercise the full wire
//! path without leaving the process.
//!
//! The sync surface is deliberate: the kit is a synchronous crate (the
//! embedded `App` returns `Result`s, no async runtime on the test
//! thread), so the handle owns the door's tokio runtime and drives the
//! typed rows with `block_on` — the exact shape the §15.1 remote bench
//! samples ride (`resolve_remote_uds`). Async hosts skip the wrappers
//! and take [`RemoteToSelf::conn`] directly.

use guiwright_driver::SeamRegistry;
use guiwright_driver::adapter::FrameworkAdapter;
use guiwright_driver::server::{Endpoint, GwpServerHandle};
use guiwright_proto::client::{ActionReceipt, ClientConn};
use guiwright_proto::duplex::DuplexConnect;
use guiwright_proto::schema::{
    AdapterInfoV1, DescribedV1, NodeKeyV1, ResolvedMatchV1, SelectorV1, SemActionV1,
};
use guiwright_proto::{DuplexClient as DoorClient, GwpError};

use crate::app::KitError;

/// The kit-level "remote-to-self" handle: the whole remote stack
/// (session thread + bridge + typed client) over the in-process door,
/// joinable through [`Self::finish`].
pub struct RemoteToSelf {
    /// The door's runtime — it owns the serve task, so it must outlive
    /// every conversation. Sync rows `block_on` through it.
    runtime: tokio::runtime::Runtime,
    server: GwpServerHandle,
    /// The connect half, kept so hosts can attach more clients (the
    /// live capture's feed rides exactly this — the door's documented
    /// clone-and-connect contract).
    connect: DuplexConnect,
    client: DoorClient,
    agent: String,
    adapter: AdapterInfoV1,
}

impl std::fmt::Debug for RemoteToSelf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteToSelf")
            .field("endpoint", &self.server.endpoint())
            .field("agent", &self.agent)
            .field("adapter", &self.adapter.name)
            .finish_non_exhaustive()
    }
}

/// What one remote-to-self session answered for: the facts a test
/// report (or a downstream log line) names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteToSelfReport {
    /// The agent string the handle handshaked under.
    pub agent: String,
    /// The adapter's `Welcome` name (the fixture app's adapter, e.g.
    /// `"mock"`).
    pub adapter: String,
    /// The adapter's framework self-description.
    pub framework: String,
    /// The endpoint the door reported — always
    /// [`Endpoint::InProcess`] for this handle.
    pub endpoint: Endpoint,
}

impl RemoteToSelf {
    /// Bring the remote stack up in-process over `make_adapter` (the
    /// adapter is constructed on the session thread, exactly the
    /// `spawn_inprocess` shape): spawn the server on the embedded door,
    /// connect a typed client, handshake as the kit's agent.
    pub fn launch<A, F>(make_adapter: F) -> Result<Self, KitError>
    where
        A: FrameworkAdapter + 'static,
        F: FnOnce() -> A + Send + 'static,
    {
        let agent = format!(
            "{}/{} (remote-to-self)",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let brought: Result<(GwpServerHandle, DuplexConnect, DoorClient, AdapterInfoV1), GwpError> =
            runtime.block_on(async {
                let (server, connect) =
                    GwpServerHandle::spawn_inprocess(make_adapter, SeamRegistry::new())?;
                let mut client = DoorClient::connect(&connect).await?;
                let adapter = client.handshake(&agent).await?;
                Ok((server, connect, client, adapter))
            });
        let (server, connect, client, adapter) = brought?;
        Ok(Self {
            runtime,
            server,
            connect,
            client,
            agent,
            adapter,
        })
    }

    /// Launch the standard fixture app — the embedded default's remote
    /// twin ([`App::launch_fixture`](crate::App::launch_fixture) rides
    /// the same `Fixture::buttons()` mock through the direct-adapter
    /// path this one rides through the door).
    ///
    /// # Panics
    ///
    /// Panics when the door fails to come up — the embedded door has no
    /// sockets to bind and no filesystem to touch, so a failure here is
    /// a bug, not a test condition (the same posture
    /// [`App::launch_fixture`](crate::App::launch_fixture) takes).
    pub fn launch_fixture() -> Self {
        let fixture = guiwright_core::test_util::Fixture::buttons();
        Self::launch(move || {
            let mut adapter = guiwright_driver::mock::MockAdapter::with_fixture(&fixture);
            adapter.push_frame(1);
            adapter.push_frame(1);
            adapter
        })
        .expect("the embedded door always comes up")
    }

    /// The agent string the client handshaked under.
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// The adapter's `Welcome` facts (name, framework, capability
    /// flags).
    pub fn adapter(&self) -> &AdapterInfoV1 {
        &self.adapter
    }

    /// The endpoint the door reports — [`Endpoint::InProcess`] (the m16
    /// contract: no socket, no filesystem).
    pub fn endpoint(&self) -> &Endpoint {
        self.server.endpoint()
    }

    /// The connect half, for hosts that attach more clients to the same
    /// door (a second connection rides `DuplexClient::connect(&half)` —
    /// the door's clone-and-connect contract; the kit's live capture
    /// uses exactly this).
    pub fn connect_half(&self) -> &DuplexConnect {
        &self.connect
    }

    /// The typed client surface, for async hosts: every [`ClientConn`]
    /// row (the m47 resolve/act pair, the M36–m70 describe family, the
    /// m46 bulk reads, the m42 wait, the m52 mock rows) answers over
    /// the door without the sync wrappers.
    pub fn conn(&mut self) -> &mut DoorClient {
        &mut self.client
    }

    /// One strict-mode resolve over the door (the m47 aim half, kit
    /// selector vocabulary in — the `From<Selector> for SelectorV1`
    /// bridge carries it), the wire handle + generation out.
    ///
    /// # Errors
    ///
    /// The wire's own refusals surface as [`KitError::Wire`]: the
    /// strict-mode rejection (`ErrorCode::StrictModeViolation`), the
    /// auto-wait budget expiry (`ErrorCode::Timeout` naming the stuck
    /// check).
    pub fn resolve(
        &mut self,
        selector: impl Into<SelectorV1>,
        timeout_ms: u64,
    ) -> Result<ResolvedMatchV1, KitError> {
        let runtime = &self.runtime;
        let client = &mut self.client;
        Ok(runtime.block_on(client.resolve(selector.into(), timeout_ms))?)
    }

    /// One act over the door (the m47 act half): the actionability gate
    /// re-checks the pinned node at act time (the M17 contract),
    /// answering the [`ActionReceipt`] — the frame the action applied
    /// at and the adapter's detail.
    ///
    /// # Errors
    ///
    /// [`KitError::Wire`] carries the typed refusals — a stale
    /// generation (`ErrorCode::StaleGeneration`), a failed act-time
    /// gate (`ErrorCode::Timeout` naming the stuck check).
    pub fn act(
        &mut self,
        node: NodeKeyV1,
        generation: u32,
        sem: SemActionV1,
        force: bool,
        timeout_ms: u64,
    ) -> Result<ActionReceipt, KitError> {
        let runtime = &self.runtime;
        let client = &mut self.client;
        Ok(runtime.block_on(client.action(node, generation, sem, force, timeout_ms))?)
    }

    /// One node's identity facts over the door (the M36 describe read):
    /// role, accessible name, value, bounds — for a row the client
    /// already holds (the dump-row rule).
    ///
    /// # Errors
    ///
    /// [`KitError::Wire`] carries the boundary's typed refusals — a
    /// recycled handle (`ErrorCode::StaleGeneration`), a dead one
    /// (`ErrorCode::UnknownNode`).
    pub fn describe(&mut self, node: NodeKeyV1, generation: u32) -> Result<DescribedV1, KitError> {
        let runtime = &self.runtime;
        let client = &mut self.client;
        Ok(runtime.block_on(client.describe(node, generation))?)
    }

    /// The set's identity facts over the door (the M39 describe-all
    /// walk): every current match of `selector`, one round trip,
    /// non-strict — the m46 bulk readers' source domain, now reachable
    /// from kit tests.
    ///
    /// # Errors
    ///
    /// [`KitError::Wire`] carries the wire rejection (a pre-M39 driver
    /// answers the typed `Unsupported`; a same-version driver always
    /// speaks it).
    pub fn describe_all(
        &mut self,
        selector: impl Into<SelectorV1>,
    ) -> Result<Vec<DescribedV1>, KitError> {
        let runtime = &self.runtime;
        let client = &mut self.client;
        Ok(runtime.block_on(client.describe_all(selector.into()))?)
    }

    /// Graceful shutdown: signal the session thread, stop the accept
    /// loop, join everything — and report what the door served. Later
    /// connects over the connect half fail honestly (the duplex door's
    /// "listener is gone" contract).
    pub fn finish(self) -> Result<RemoteToSelfReport, KitError> {
        let Self {
            server,
            agent,
            adapter,
            ..
        } = self;
        let report = RemoteToSelfReport {
            endpoint: server.endpoint().clone(),
            adapter: adapter.name.clone(),
            framework: adapter.framework.clone(),
            agent,
        };
        server.shutdown();
        Ok(report)
    }
}
