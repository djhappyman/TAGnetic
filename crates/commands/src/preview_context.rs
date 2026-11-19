//! Cooperative cancellation inherited by legacy synchronous preview reads.
//! Scopes are worker-local and never cross an await or cancel a write queue.
use std::{cell::RefCell, rc::Rc};

struct Context {
    requested: Box<dyn Fn() -> bool>,
    parent: Option<Rc<Context>>,
}
thread_local! {
    static CURRENT: RefCell<Option<Rc<Context>>> = const { RefCell::new(None) };
}
struct Scope(Option<Rc<Context>>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = self.0.take());
    }
}

/// Run a synchronous read-only preview with cooperative cancellation at native
/// metadata-read boundaries. Nested scopes inherit outer cancellation. The
/// scope is restored on errors and unwinding; no helper thread is created.
///
/// This does not interrupt a parser/syscall already executing. Use only on
/// preview jobs; mutation admission and journaled cancellation remain separate.
pub fn with_preview_cancellation<T>(
    requested: impl Fn() -> bool + 'static,
    job: impl FnOnce() -> T,
) -> T {
    let previous = CURRENT.with(|current| {
        let previous = current.borrow().clone();
        *current.borrow_mut() = Some(Rc::new(Context {
            requested: Box::new(requested),
            parent: previous.clone(),
        }));
        previous
    });
    let _scope = Scope(previous);
    job()
}

pub(crate) fn requested() -> bool {
    let mut current = CURRENT.with(|current| current.borrow().clone());
    // Release the TLS borrow before invoking a callback: a callback may itself
    // inspect/run a nested preview without borrowing the cell recursively.
    while let Some(context) = current {
        if (context.requested)() {
            return true;
        }
        current = context.parent.clone();
    }
    false
}

pub(crate) fn check() -> Result<(), super::AppError> {
    if requested() {
        Err(tagrex_core::plan::PlanError::Cancelled.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_scopes_inherit_cancellation_and_restore_after_unwinding() {
        assert!(!requested());
        with_preview_cancellation(
            || true,
            || {
                assert!(requested());
                with_preview_cancellation(|| false, || assert!(requested()));
                assert!(requested());
            },
        );
        assert!(!requested());
        assert!(std::panic::catch_unwind(|| with_preview_cancellation(
            || true,
            || panic!("scope unwind canary")
        ))
        .is_err());
        assert!(!requested());
        with_preview_cancellation(|| false, || assert!(check().is_ok()));
    }
    #[test]
    fn cancellation_is_local_to_the_worker_thread() {
        with_preview_cancellation(
            || true,
            || {
                std::thread::spawn(|| assert!(!requested())).join().unwrap();
                assert!(requested());
            },
        );
    }
}
