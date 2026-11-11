//! Transport-agnostic GWP client (§7): one connection type over the UDS
//! and TCP transports of `guiwright-proto`. The REPL is transport-blind —
//! `--uds /tmp/app.gwp` and `--tcp 127.0.0.1:41234` drive byte-identical
//! conversations.

use std::path::PathBuf;

use guiwright_proto::schema::{AdapterInfoV1, GwpEvent, RequestV1, ResponseV1};
use guiwright_proto::{GwpError, tcp, uds};

/// Where to attach: a Unix domain socket path or a TCP `host:port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectTarget {
    Uds(PathBuf),
    Tcp(String),
}

impl ConnectTarget {
    /// Parse a `uds:///path` / `tcp://host:port` URL (the
    /// `GUIWRIGHT_DRIVER` launch-mode form, §5.3) or a bare path/address.
    pub fn parse(value: &str) -> Result<Self, String> {
        if let Some(rest) = value.strip_prefix("uds://") {
            if rest.is_empty() {
                return Err("uds:// URL needs a socket path".into());
            }
            return Ok(ConnectTarget::Uds(PathBuf::from(rest)));
        }
        if let Some(rest) = value.strip_prefix("tcp://") {
            if rest.is_empty() {
                return Err("tcp:// URL needs a host:port".into());
            }
            return Ok(ConnectTarget::Tcp(rest.to_string()));
        }
        // Bare forms: a path containing `/` is a socket, else an address.
        if value.contains('/') {
            Ok(ConnectTarget::Uds(PathBuf::from(value)))
        } else {
            Ok(ConnectTarget::Tcp(value.to_string()))
        }
    }
}

/// One live GWP session over either transport.
pub enum Connection {
    Uds(uds::Client),
    Tcp(tcp::Client),
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Connection::Uds(_) => f.write_str("Connection(Uds)"),
            Connection::Tcp(_) => f.write_str("Connection(Tcp)"),
        }
    }
}

impl Connection {
    pub async fn connect(target: &ConnectTarget) -> Result<Self, GwpError> {
        Ok(match target {
            ConnectTarget::Uds(path) => Connection::Uds(uds::Client::connect(path).await?),
            ConnectTarget::Tcp(addr) => Connection::Tcp(tcp::Client::connect(addr).await?),
        })
    }

    /// Send `Hello` and validate the driver's `Welcome` (§7).
    pub async fn handshake(&mut self, agent: &str) -> Result<AdapterInfoV1, GwpError> {
        match self {
            Connection::Uds(c) => c.handshake(agent).await,
            Connection::Tcp(c) => c.handshake(agent).await,
        }
    }

    /// One request → response round-trip; `Error` replies map to
    /// [`GwpError::Rejected`].
    pub async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        match self {
            Connection::Uds(c) => c.request(request).await,
            Connection::Tcp(c) => c.request(request).await,
        }
    }

    /// The next pushed event (§7, M12) — blocked until one arrives.
    pub async fn next_event(&mut self) -> Result<GwpEvent, GwpError> {
        match self {
            Connection::Uds(c) => c.next_event().await,
            Connection::Tcp(c) => c.next_event().await,
        }
    }

    /// Non-blocking event drain: whatever already arrived (buffered by
    /// earlier responses or already readable), or `None`. The REPL flushes
    /// this between commands — "events flush at command boundaries" is the
    /// honest contract for a line-oriented debugger.
    pub async fn poll_event(&mut self) -> Result<Option<GwpEvent>, GwpError> {
        match tokio::time::timeout(std::time::Duration::ZERO, self.next_event()).await {
            Ok(event) => event.map(Some),
            Err(_elapsed) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls_and_bare_forms() {
        assert_eq!(
            ConnectTarget::parse("uds:///tmp/app.gwp"),
            Ok(ConnectTarget::Uds(PathBuf::from("/tmp/app.gwp")))
        );
        assert_eq!(
            ConnectTarget::parse("tcp://127.0.0.1:41234"),
            Ok(ConnectTarget::Tcp("127.0.0.1:41234".into()))
        );
        assert_eq!(
            ConnectTarget::parse("/tmp/app.gwp"),
            Ok(ConnectTarget::Uds(PathBuf::from("/tmp/app.gwp")))
        );
        assert_eq!(
            ConnectTarget::parse("127.0.0.1:41234"),
            Ok(ConnectTarget::Tcp("127.0.0.1:41234".into()))
        );
        assert!(ConnectTarget::parse("uds://").is_err());
        assert!(ConnectTarget::parse("tcp://").is_err());
    }
}
