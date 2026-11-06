//! Error taxonomy shared by kit, driver and adapters.
//!
//! Errors carry enough context for Playwright-style failure reports
//! (architecture §8.3 "on timeout the error embeds the last tree snapshot").

use std::fmt;

use crate::key::NodeKey;

/// A short, human-readable description of a resolved node, used in error
/// context (role + accessible name + bounds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeDesc {
    pub key: NodeKey,
    pub role: String,
    pub name: Option<String>,
    pub bounds: Option<String>,
}

impl fmt::Display for NodeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.role)?;
        if let Some(name) = &self.name {
            write!(f, " \"{name}\"")?;
        }
        if let Some(bounds) = &self.bounds {
            write!(f, " @ {bounds}")?;
        }
        Ok(())
    }
}

/// A compact snapshot of the tree used to enrich timeout errors
/// ("error embeds the last tree snapshot", §8.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSummary {
    pub node_count: usize,
    pub root_role: Option<String>,
    pub focus: Option<NodeDesc>,
    pub sample: Vec<NodeDesc>,
}

impl fmt::Display for TreeSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tree[{} nodes", self.node_count)?;
        if let Some(root) = &self.root_role {
            write!(f, ", root={root}")?;
        }
        if let Some(focus) = &self.focus {
            write!(f, ", focus={focus}")?;
        }
        write!(f, "]")?;
        for (i, node) in self.sample.iter().enumerate() {
            let sep = if i == 0 { ": " } else { ", " };
            write!(f, "{sep}{node}")?;
        }
        Ok(())
    }
}

/// Errors produced by the core (tree service, resolution, waiters, expect).
///
/// The tree-snapshot payloads are boxed so `Result<_, CoreError>` stays
/// cheap to move on the hot resolve path (clippy `result_large_err`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// Strict mode: the selector matched nothing (§8.2 strict policy).
    #[error("no node matches `{selector}` (last state: {last})")]
    NoMatch {
        selector: String,
        last: Box<TreeSummary>,
    },
    /// Strict mode: the selector matched 2+ nodes and no narrowing
    /// (`first()/nth()/last()/filter()`) was applied.
    #[error(
        "strict mode violation: `{selector}` resolved to {} nodes; \
         narrow it with `.first()`, `.nth(n)`, `.last()` or a `.filter()`",
        matches.len()
    )]
    Ambiguous {
        selector: String,
        matches: Vec<NodeDesc>,
    },
    /// A wait/assertion did not become true within its timeout.
    #[error("timed out after {timeout:?} waiting for {operation} (last state: {last})")]
    Timeout {
        operation: String,
        timeout: std::time::Duration,
        last: Box<TreeSummary>,
    },
    /// The resolved node identity was recycled between resolve and use
    /// (generation mismatch, §8.2 generation guard).
    #[error("node {key:?} is stale (generation {generation} no longer exists)")]
    StaleNode { key: NodeKey, generation: u32 },
    /// The adapter/transport rejected the operation (e.g. unsupported at M0).
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    /// Anything else, with context.
    #[error("{0}")]
    Other(String),
}

impl CoreError {
    /// Convenience constructor for ad-hoc errors.
    pub fn other(msg: impl Into<String>) -> Self {
        CoreError::Other(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_error_mentions_narrowing_helpers() {
        let err = CoreError::Ambiguous {
            selector: "button".to_string(),
            matches: vec![NodeDesc {
                key: crate::key::NodeKey::AccessKit {
                    tree: accesskit::TreeId::ROOT,
                    node: accesskit::NodeId(1),
                },
                role: "button".into(),
                name: Some("Save".into()),
                bounds: None,
            }],
        };
        let msg = err.to_string();
        assert!(msg.contains("2 nodes") || msg.contains("1 node"), "{msg}");
        assert!(msg.contains(".first()"), "{msg}");
    }

    #[test]
    fn timeout_error_renders_tree_summary() {
        let err = CoreError::Timeout {
            operation: "visible".into(),
            timeout: std::time::Duration::from_millis(10),
            last: Box::new(TreeSummary {
                node_count: 4,
                root_role: Some("window".into()),
                focus: None,
                sample: vec![],
            }),
        };
        let msg = err.to_string();
        assert!(msg.contains("10ms"), "{msg}");
        assert!(msg.contains("4 nodes"), "{msg}");
        assert!(msg.contains("window"), "{msg}");
    }
}
