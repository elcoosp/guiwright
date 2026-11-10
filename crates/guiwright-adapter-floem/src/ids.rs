//! **Contribution 1a (§4.7)** — the `test_id(...)` decorator engine.
//!
//! Floem has no accessibility and no test ids ("no stable ids in some
//! stacks (floem, …) → flaky locators", §18). The contribution is a
//! first-class `test_id(...)` decorator view (mirroring floem's
//! `debug_name` slot): registering stores the id in a **global
//! id → `ViewId` map**, the upstream side of which is a small patch to
//! floem's view plumbing (`docs/m3-floem-contributions.md`).
//!
//! This module is the driver-side engine that registry feeds, with the
//! semantics Guiwright requires of it:
//!
//! * **strict-mode duplicate detection** — two views with the same test
//!   id is an app bug; registration fails loudly rather than letting
//!   `getByTestId` become ambiguous (§8.1);
//! * **ABA generation guards** — a `ViewId` is a slotmap key: recycling
//!   happens. Every (re)registration of a handle bumps the handle's
//!   generation; stale handles from a previous pass resolve to nothing
//!   (§8.2);
//! * **removal hooks** — the harness calls [`TestIdRegistry::forget`]
//!   when a view dies, so the map never leaks dead views.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TestIdError {
    #[error(
        "strict mode violation: test id `{id}` is already registered (view {existing:?}); \
         test ids must be unique within the app (§8.1)"
    )]
    Duplicate { id: String, existing: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Registered {
    view: u64,
    generation: u32,
}

/// The global test-id → `ViewId` registry (§4.7 contribution 1a).
#[derive(Clone, Debug, Default)]
pub struct TestIdRegistry {
    by_id: HashMap<String, Registered>,
    by_view: HashMap<u64, String>,
    /// Per-handle generation counters (ABA guard, §8.2).
    generations: HashMap<u64, u32>,
}

impl TestIdRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// `test_id("checkout-btn", view)` — the decorator's registration
    /// hook. Duplicate ids are rejected (§8.1); re-registering a live
    /// handle under a new id is allowed (a view's id can legitimately
    /// change) and bumps the handle's generation.
    pub fn register(&mut self, id: impl Into<String>, view: u64) -> Result<u32, TestIdError> {
        let id = id.into();
        if let Some(existing) = self.by_id.get(&id) {
            if existing.view != view {
                return Err(TestIdError::Duplicate {
                    id,
                    existing: existing.view,
                });
            }
            // Same view re-registering the same id: idempotent.
            return Ok(existing.generation);
        }
        let generation = {
            let g = self.generations.entry(view).or_insert(0);
            *g = g.saturating_add(1);
            *g
        };
        self.by_id
            .insert(id.clone(), Registered { view, generation });
        self.by_view.insert(view, id);
        Ok(generation)
    }

    /// `getByTestId` resolution: the view handle + the generation the
    /// registration carries (the session binds it into the O(1) hash,
    /// §8.1).
    pub fn resolve(&self, id: &str) -> Option<(u64, u32)> {
        self.by_id.get(id).map(|r| (r.view, r.generation))
    }

    /// The test id a view currently carries (the logical tree pulls these
    /// per pass, §6.1 `LogicalTreeDiff.test_ids`).
    pub fn view_of(&self, view: u64) -> Option<&str> {
        self.by_view.get(&view).map(String::as_str)
    }

    /// View removal hook: drop the registrations for a dead handle.
    pub fn forget(&mut self, view: u64) {
        if let Some(id) = self.by_view.remove(&view) {
            self.by_id.remove(&id);
        }
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_resolve_roundtrip() {
        let mut reg = TestIdRegistry::new();
        let g = reg.register("save", 7).expect("register");
        assert_eq!(g, 1, "first registration bumps to 1");
        assert_eq!(reg.resolve("save"), Some((7, 1)));
        assert_eq!(reg.view_of(7), Some("save"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn duplicate_ids_are_strict_mode_errors() {
        let mut reg = TestIdRegistry::new();
        reg.register("save", 7).expect("register");
        let err = reg.register("save", 9).expect_err("duplicate");
        assert!(err.to_string().contains("save"), "{err}");
        assert!(
            err.to_string().contains('7'),
            "names the existing view: {err}"
        );
        // The original registration is untouched.
        assert_eq!(reg.resolve("save"), Some((7, 1)));
    }

    #[test]
    fn same_view_re_registering_is_idempotent() {
        let mut reg = TestIdRegistry::new();
        reg.register("save", 7).expect("register");
        let g = reg.register("save", 7).expect("re-register same view");
        assert_eq!(g, 1, "no extra bump");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn generation_guards_against_slot_recycling() {
        // ViewId is a slotmap key: handle 7 dies, slot reuses 7.
        let mut reg = TestIdRegistry::new();
        reg.register("old", 7).expect("register");
        assert_eq!(reg.resolve("old"), Some((7, 1)));
        reg.forget(7);
        assert_eq!(reg.resolve("old"), None, "dead view resolves to nothing");

        // A recycled 7 re-registering bumps its generation (ABA, §8.2).
        let g = reg.register("new", 7).expect("register recycled");
        assert_eq!(g, 2, "generation bumped across the recycle");
        assert_eq!(reg.resolve("new"), Some((7, 2)));
    }

    #[test]
    fn re_moving_a_view_releases_its_id() {
        let mut reg = TestIdRegistry::new();
        reg.register("save", 7).expect("register");
        reg.forget(7);
        // The id is free again after the view died.
        reg.register("save", 12).expect("id reusable after forget");
        assert_eq!(reg.resolve("save"), Some((12, 1)));
    }
}
