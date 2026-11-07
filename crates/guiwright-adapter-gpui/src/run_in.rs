//! The state-seam runner-context contract (§14, §4.5).
//!
//! GPUI app state lives in `Entity` fields on the UI thread. The seam
//! surface (`FrameworkAdapter::run_in`, §14 "state writes need the
//! runner's context") hands a `&mut dyn Any` runner context to
//! app-registered hooks; a GPUI adapter implements `run_in` by
//! dispatching the task on the **foreground executor** with the app's
//! root entity as context — the M2 spike validated exactly this flow over
//! the wire (canned override → clear → real hook in the runner context).
//!
//! The plan is the pure routing table the glue executes; the
//! main-thread-confinement rules are §5.1's (the task never crosses
//! threads — the foreground executor is `!Send` by construction).

use guiwright_driver::seams::SeamTask;

/// How a seam task reaches the UI thread (§5.1 waker table, GPUI row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    /// `ForegroundExecutor::dispatch` — runs on the main thread at the
    /// next dispatch round (§14 clock plan drives the rounds).
    ForegroundExecutor,
    /// `dispatch_after(Duration::ZERO)` — the §5.1 GPUI waker; same queue,
    /// explicitly zero-delay for test determinism.
    ForegroundAfterZero,
}

/// One seam execution plan.
#[derive(Clone, Debug)]
pub struct RunInPlan {
    /// The registered seam id (§14 `Mock::Call { id }`).
    pub seam_id: String,
    pub dispatch: Dispatch,
}

impl RunInPlan {
    /// The default plan for `run_in` (the spike doc §2 contract).
    pub fn new(seam_id: impl Into<String>) -> Self {
        Self {
            seam_id: seam_id.into(),
            dispatch: Dispatch::ForegroundAfterZero,
        }
    }

    /// Execute the plan: the glue calls this **on the foreground thread**
    /// with the app's root entity handed over as the runner context.
    /// `run` is the executor-side hop — it receives the seam task and the
    /// `&mut dyn Any` context and returns the hook's detail.
    pub fn execute(
        self,
        task: SeamTask,
        run: impl FnOnce(SeamTask, &mut dyn std::any::Any) -> Result<String, String>,
        context: &mut dyn std::any::Any,
    ) -> Result<String, String> {
        let _ = self.dispatch; // the glue already hopped to the foreground
        run(task, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct Db {
        rows: usize,
    }

    #[test]
    fn seam_task_runs_inside_the_runner_context() {
        let plan = RunInPlan::new("reset_state");
        let task: SeamTask = Box::new(|ctx: &mut dyn Any| {
            let db = ctx
                .downcast_mut::<Db>()
                .ok_or("wrong runner context type")?;
            db.rows = 0;
            Ok("state reset".to_string())
        });
        let mut db = Db { rows: 9 };
        let detail = plan
            .execute(task, |task, ctx| task(ctx), &mut db as &mut dyn Any)
            .expect("hook runs");
        assert_eq!(detail, "state reset");
        assert_eq!(db.rows, 0, "the hook wrote through the context");
    }

    #[test]
    fn wrong_context_type_is_a_clean_error() {
        let plan = RunInPlan::new("reset_state");
        let task: SeamTask = Box::new(|ctx: &mut dyn Any| {
            ctx.downcast_mut::<Db>()
                .ok_or("wrong runner context type")?;
            Ok("unreachable".to_string())
        });
        let mut other = String::from("not a Db");
        let err = plan
            .execute(task, |task, ctx| task(ctx), &mut other as &mut dyn Any)
            .expect_err("type mismatch");
        assert!(err.contains("wrong runner context type"), "{err}");
    }
}
