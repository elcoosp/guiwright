//! State seams — the mock domain (architecture §14 "State seams").
//!
//! Guiwright does not fake the network; **apps** register reset/init hooks
//! ("seams") and tests drive them between test phases for fixture parity
//! with Playwright contexts (§14 "each kit test gets a fresh app state").
//!
//! Execution model: seams write app state, and app state is confined to the
//! UI thread — so a seam is **compiled into a [`SeamTask`] and executed via
//! [`crate::adapter::FrameworkAdapter::run_in`]**, inside the adapter's
//! runner context (the `&mut dyn Any` state the adapter owns). The session
//! never touches app state directly; it only routes.
//!
//! The mock domain (§7 `Mock`) layers *overrides* on top of real hooks:
//! `install(id, response)` presets a canned result, `clear(id)` restores
//! the real hook, `reset_state(scope)` runs the session's reset seam.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A registered seam hook: receives the adapter's runner context (the app
/// state, as `&mut dyn Any`) and a scope string (fixture tag / seed).
pub type SeamFn = Arc<dyn Fn(&mut dyn Any, &str) -> Result<String, String> + Send + Sync>;

/// A seam compiled for execution inside the adapter's runner context
/// (§14 "state writes run in the runner context via run_in").
pub type SeamTask = Box<dyn FnOnce(&mut dyn Any) -> Result<String, String> + Send>;

/// Registry of app-registered seams (name → hook). Populated at app boot
/// (see `guiwright_kit`'s `#[seam]` + `register_seams!`), consulted by the
/// session when a mock-domain command runs.
#[derive(Clone, Default)]
pub struct SeamRegistry {
    seams: BTreeMap<String, SeamFn>,
}

impl std::fmt::Debug for SeamRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeamRegistry")
            .field("seams", &self.seams.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl SeamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a seam hook under `id`.
    pub fn register(
        &mut self,
        id: impl Into<String>,
        hook: impl Fn(&mut dyn Any, &str) -> Result<String, String> + Send + Sync + 'static,
    ) {
        self.seams.insert(id.into(), Arc::new(hook));
    }

    /// Merge every hook of `other` into `self` (`other` wins on collision).
    pub fn merge(&mut self, other: &SeamRegistry) {
        for (id, hook) in &other.seams {
            self.seams.insert(id.clone(), Arc::clone(hook));
        }
    }

    pub fn get(&self, id: &str) -> Option<SeamFn> {
        self.seams.get(id).cloned()
    }

    /// Registered seam ids, sorted (stable diagnostics order).
    pub fn ids(&self) -> Vec<&str> {
        self.seams.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.seams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seams.is_empty()
    }
}

/// Compile a seam for `run_in`: the closure captures the hook and scope;
/// the adapter supplies the runner context at execution time.
pub fn compile_task(hook: SeamFn, scope: String) -> SeamTask {
    Box::new(move |ctx: &mut dyn Any| hook(ctx, &scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct AppState {
        users: usize,
        seeded: Option<String>,
    }

    #[test]
    fn registry_stores_and_fetches_hooks() {
        let mut reg = SeamRegistry::new();
        assert!(reg.is_empty());
        reg.register("reset_db", |state, scope| {
            let app = state
                .downcast_mut::<AppState>()
                .ok_or("wrong runner context type")?;
            app.users = 0;
            app.seeded = Some(scope.to_string());
            Ok(format!("db reset to {scope}"))
        });
        reg.register("seed_users", |state, _scope| {
            let app = state
                .downcast_mut::<AppState>()
                .ok_or("wrong runner context type")?;
            app.users = 3;
            Ok("3 users seeded".into())
        });
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.ids(), vec!["reset_db", "seed_users"]);
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn compiled_task_writes_the_runner_context() {
        let mut reg = SeamRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        reg.register("reset_db", move |state, scope| {
            counter.fetch_add(1, Ordering::SeqCst);
            let app = state
                .downcast_mut::<AppState>()
                .ok_or("wrong runner context type")?;
            app.users = 0;
            app.seeded = Some(scope.to_string());
            Ok(format!("db reset to {scope}"))
        });

        let mut state: Box<dyn Any + Send> = Box::new(AppState {
            users: 9,
            seeded: None,
        });
        let hook = reg.get("reset_db").expect("registered");
        let task = compile_task(hook, "seed-42".into());
        // The adapter's run_in supplies the context — exactly this shape.
        let detail = task(state.as_mut_dyn()).expect("seam ok");
        assert_eq!(detail, "db reset to seed-42");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let app = state.downcast_ref::<AppState>().expect("app state");
        assert_eq!(app.users, 0, "state was written through the seam");
        assert_eq!(app.seeded.as_deref(), Some("seed-42"));
    }

    #[test]
    fn wrong_context_type_is_a_clean_error() {
        let mut reg = SeamRegistry::new();
        reg.register("reset_db", |state, _scope| {
            state
                .downcast_mut::<AppState>()
                .map(|_| "ok".into())
                .ok_or_else(|| "wrong runner context type".into())
        });
        let hook = reg.get("reset_db").expect("registered");
        let mut other: Box<dyn Any + Send> = Box::new(7u32);
        let task = compile_task(hook, String::new());
        let err = task(other.as_mut_dyn()).expect_err("type mismatch");
        assert!(err.contains("wrong runner context"), "{err}");
    }

    #[test]
    fn merge_later_registry_wins() {
        let mut base = SeamRegistry::new();
        base.register("reset_db", |_state, _scope| Ok("base".into()));
        let mut patch = SeamRegistry::new();
        patch.register("reset_db", |_state, _scope| Ok("patch".into()));
        patch.register("extra", |_state, _scope| Ok("extra".into()));
        base.merge(&patch);
        assert_eq!(base.len(), 2);
        let mut state: Box<dyn Any + Send> = Box::new(AppState::default());
        let hook = base.get("reset_db").expect("hook");
        let task = compile_task(hook, String::new());
        assert_eq!(task(state.as_mut_dyn()).expect("run"), "patch");
    }

    /// Helper: `Box<dyn Any + Send>` → `&mut dyn Any` (the run_in shape).
    trait AsMutDyn {
        fn as_mut_dyn(&mut self) -> &mut dyn Any;
    }
    impl AsMutDyn for Box<dyn Any + Send> {
        fn as_mut_dyn(&mut self) -> &mut dyn Any {
            &mut **self
        }
    }
}
