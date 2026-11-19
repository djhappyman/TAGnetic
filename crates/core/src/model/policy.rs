//! Owned metadata interpretation, scoped to a synchronous operation/thread.
use super::{tag_type_from_key, DEFAULT_MULTI_VALUE_SEPARATOR, LEGACY_CODE_PAGES};
use encoding_rs::Encoding;
use lofty::tag::TagType;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, RwLock,
    },
};

static GENERATION: AtomicU64 = AtomicU64::new(0);
fn generation() -> u64 {
    let mut old = GENERATION.load(Ordering::Acquire);
    loop {
        let next = old.saturating_add(1);
        match GENERATION.compare_exchange_weak(old, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(current) => old = current,
        }
    }
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub revision: u64,
    pub write_id3v23: bool,
    pub read_priority: Vec<TagType>,
    pub separator: String,
    pub code_page: Option<&'static Encoding>,
}
impl Snapshot {
    fn same_settings(&self, other: &Self) -> bool {
        self.write_id3v23 == other.write_id3v23
            && self.read_priority == other.read_priority
            && self.separator == other.separator
            && self.code_page.map(Encoding::name) == other.code_page.map(Encoding::name)
    }
}
/// Metadata settings owned by one session. Clones share settings and generation.
/// Enter a scope on each worker thread that performs metadata IO; TLS does not
/// propagate into spawned workers. Guards hold no lock across IO or callbacks.
#[derive(Clone)]
pub struct ModelPolicy(Arc<RwLock<Arc<Snapshot>>>);
impl Default for ModelPolicy {
    fn default() -> Self {
        Self(Arc::new(RwLock::new(Arc::new(Snapshot {
            revision: generation(),
            write_id3v23: false,
            read_priority: Vec::new(),
            separator: DEFAULT_MULTI_VALUE_SEPARATOR.into(),
            code_page: None,
        }))))
    }
}
impl ModelPolicy {
    /// Atomically configure the complete interpretation. Empty/unknown settings
    /// have the same defaults as the standalone model setters.
    pub fn configure(&self, id3v23: bool, priority: &[String], separator: &str, code_page: &str) {
        let priority = priority
            .iter()
            .filter_map(|key| tag_type_from_key(key))
            .collect::<Vec<_>>();
        let separator = normalize_separator(separator);
        let code_page = normalize_code_page(code_page);
        self.update(|state| {
            state.write_id3v23 = id3v23;
            state.read_priority = priority.clone();
            state.separator = separator.clone();
            state.code_page = code_page;
        });
    }
    fn snapshot(&self) -> Arc<Snapshot> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn update(&self, change: impl Fn(&mut Snapshot)) {
        let mut current = self.0.write().unwrap_or_else(|e| e.into_inner());
        let mut next = (**current).clone();
        change(&mut next);
        if !next.same_settings(&current) {
            next.revision = generation();
            *current = Arc::new(next);
        }
    }
    /// Pin this operation's immutable interpretation. Nested/reentrant calls
    /// reuse the original snapshot of the same owner. Changing that owner
    /// invalidates its active generation rather than silently refreshing it.
    pub fn enter(&self) -> PolicyScope {
        let token = ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(frame) = active.last().filter(|f| Arc::ptr_eq(&f.owner.0, &self.0)) {
                // Common nested App/cache calls share one pin without allocating.
                return frame.token.clone();
            }
            let inherited = active
                .iter()
                .rev()
                .find(|frame| Arc::ptr_eq(&frame.owner.0, &self.0));
            let (snapshot, invalidated) = match inherited {
                Some(frame) => (frame.snapshot.clone(), frame.invalidated.clone()),
                None => (self.snapshot(), Rc::new(Cell::new(false))),
            };
            let token = Rc::new(());
            active.push(Frame {
                owner: self.clone(),
                snapshot,
                invalidated,
                token: token.clone(),
                sealed: false,
            });
            token
        });
        PolicyScope { token }
    }
}
struct Frame {
    owner: ModelPolicy,
    snapshot: Arc<Snapshot>,
    invalidated: Rc<Cell<bool>>,
    token: Rc<()>,
    sealed: bool,
}
thread_local! { static ACTIVE: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) }; }
static GLOBAL: LazyLock<ModelPolicy> = LazyLock::new(ModelPolicy::default);
/// Restores the previous thread policy on return, error or unwind. This guard
/// cannot move to another thread. Removal by token also handles non-LIFO drops.
pub struct PolicyScope {
    token: Rc<()>,
}
impl Drop for PolicyScope {
    fn drop(&mut self) {
        // One reference is in the frame. Only its final guard removes the pin,
        // including if nested same-owner guards are dropped out of order.
        if Rc::strong_count(&self.token) == 2 {
            let _ = ACTIVE.try_with(|active| {
                active
                    .borrow_mut()
                    .retain(|frame| !Rc::ptr_eq(&frame.token, &self.token))
            });
        }
    }
}
pub(super) fn capture() -> (Arc<Snapshot>, u64) {
    ACTIVE.with(|active| match active.borrow().last() {
        Some(frame) => {
            let revision = if frame.invalidated.get()
                || frame.owner.snapshot().revision != frame.snapshot.revision
            {
                u64::MAX
            } else {
                frame.snapshot.revision
            };
            (frame.snapshot.clone(), revision)
        }
        None => {
            let snapshot = GLOBAL.snapshot();
            let revision = snapshot.revision;
            (snapshot, revision)
        }
    })
}
pub(super) fn current() -> Arc<Snapshot> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .last()
            .map(|frame| frame.snapshot.clone())
            .unwrap_or_else(|| GLOBAL.snapshot())
    })
}
pub(super) fn update_global(change: impl Fn(&mut Snapshot)) {
    // Explicit standalone setters called by this operation's own callback must
    // still invalidate it. Peer threads and different App owners cannot do so.
    ACTIVE.with(|active| {
        if let Some(frame) = active.borrow().last() {
            let mut next = (*frame.snapshot).clone();
            change(&mut next);
            if !frame.sealed && !next.same_settings(&frame.snapshot) {
                frame.invalidated.set(true);
            }
        }
    });
    GLOBAL.update(change);
}
thread_local! { static OPERATIONS: RefCell<Vec<Rc<u64>>> = const { RefCell::new(Vec::new()) }; }
/// Pins the caller's active policy, or standalone defaults, for a synchronous
/// Executor call. Nested user operations have independent admission identities.
pub(crate) struct OperationPolicy {
    token: Rc<u64>,
    _scope: PolicyScope,
}
impl OperationPolicy {
    pub(crate) fn enter() -> Option<Self> {
        let owner = ACTIVE
            .with(|active| active.borrow().last().map(|frame| frame.owner.clone()))
            .unwrap_or_else(|| GLOBAL.clone());
        let scope = owner.enter();
        let revision = capture().1;
        if revision == u64::MAX {
            return None;
        }
        let token = Rc::new(revision);
        OPERATIONS.with(|operations| operations.borrow_mut().push(token.clone()));
        Some(Self {
            token,
            _scope: scope,
        })
    }
}
impl Drop for OperationPolicy {
    fn drop(&mut self) {
        let _ = OPERATIONS.try_with(|operations| {
            operations
                .borrow_mut()
                .retain(|token| !Rc::ptr_eq(token, &self.token))
        });
    }
}
pub(crate) fn operation_policy_valid() -> bool {
    // Only trusted private rollback may keep using the originally admitted
    // interpretation after a public callback invalidated its parent operation.
    let recovery = ACTIVE.with(|active| {
        active
            .borrow()
            .iter()
            .rev()
            .find(|frame| frame.sealed)
            .map(|frame| frame.snapshot.revision)
    });
    let expected = recovery
        .or_else(|| OPERATIONS.with(|operations| operations.borrow().last().map(|token| **token)));
    let revision = capture().1;
    revision != u64::MAX && expected.is_none_or(|expected| revision == expected)
}

/// An operation-local copy of an already admitted interpretation. This is
/// private rollback authority, never a persisted policy or a user IO scope.
pub(crate) struct RecoveryPolicy(ModelPolicy);
impl RecoveryPolicy {
    pub(crate) fn capture() -> Option<Self> {
        let (snapshot, revision) = capture();
        if revision == u64::MAX {
            return None;
        }
        let mut pinned = (*snapshot).clone();
        pinned.revision = generation();
        Some(Self(ModelPolicy(Arc::new(RwLock::new(Arc::new(pinned))))))
    }
    pub(crate) fn enter(&self) -> PolicyScope {
        let guard = self.0.enter();
        ACTIVE.with(|active| {
            if let Some(frame) = active
                .borrow_mut()
                .iter_mut()
                .find(|frame| Rc::ptr_eq(&frame.token, &guard.token))
            {
                frame.sealed = true;
            }
        });
        guard
    }
}
pub(crate) fn recovery_active() -> bool {
    ACTIVE.with(|active| active.borrow().iter().any(|frame| frame.sealed))
}

pub(super) fn normalize_separator(separator: &str) -> String {
    if separator.is_empty() {
        DEFAULT_MULTI_VALUE_SEPARATOR.into()
    } else {
        separator.into()
    }
}
pub(super) fn normalize_code_page(label: &str) -> Option<&'static Encoding> {
    LEGACY_CODE_PAGES
        .contains(&label)
        .then(|| Encoding::for_label(label.as_bytes()))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_and_unwinding_session_scopes_restore_without_refreshing_changed_owner() {
        let global = capture();
        let outer = ModelPolicy::default();
        outer.configure(false, &[], " | ", "");
        let inner = ModelPolicy::default();
        inner.configure(true, &["vorbis".into()], " / ", "windows-1251");
        let first = outer.enter();
        let revision = capture().1;
        assert_ne!(revision, global.1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _inner = inner.enter();
            assert_eq!(current().separator, " / ");
            assert!(current().write_id3v23);
            panic!("owned scope unwind");
        }));
        assert!(result.is_err());
        assert_eq!(current().separator, " | ");
        assert_eq!(capture().1, revision);
        outer.configure(false, &[], "changed", "");
        let second = outer.enter();
        assert_eq!(current().separator, " | ");
        assert_eq!(capture().1, u64::MAX);
        drop(first); // non-LIFO removal must leave the nested pin intact.
        assert_eq!(current().separator, " | ");
        drop(second);
        assert_eq!(capture().1, global.1);
        assert_eq!(current().separator, global.0.separator);
        let _fresh = outer.enter();
        assert_eq!(current().separator, "changed");
        assert_ne!(capture().1, u64::MAX);
    }
    #[test]
    fn worker_scope_is_explicit_and_peer_updates_cannot_change_an_active_snapshot() {
        let owner = ModelPolicy::default();
        owner.configure(false, &[], "owner", "");
        let peer = ModelPolicy::default();
        let _scope = owner.enter();
        let original = capture().1;
        let worker_owner = owner.clone();
        std::thread::spawn(move || {
            let _worker = peer.enter();
            peer.configure(true, &[], "peer", "");
            let _owned = worker_owner.enter();
            assert_eq!(current().separator, "owner");
        })
        .join()
        .unwrap();
        assert_eq!(capture().1, original);
        assert_eq!(current().separator, "owner");
        let worker_owner = owner.clone();
        std::thread::spawn(move || worker_owner.configure(false, &[], "new owner", ""))
            .join()
            .unwrap();
        assert_eq!(current().separator, "owner");
        assert_eq!(capture().1, u64::MAX);
    }
}
