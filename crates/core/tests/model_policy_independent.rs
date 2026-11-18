//! Independent policy lifetime probes; no global serialization or media writes.
use std::sync::{Arc, Barrier};
use tagrex_core::model::{multi_value_separator, read_policy_revision, ModelPolicy};

fn policy(separator: &str) -> ModelPolicy {
    let p = ModelPolicy::default();
    p.configure(false, &[], separator, "");
    p
}

#[test]
fn reentrant_owner_retains_original_snapshot_and_non_lifo_guards_restore() {
    let a = policy("A");
    let b = policy("B");
    let a1 = a.enter();
    let generation = read_policy_revision();
    let b1 = b.enter();
    assert_ne!(generation, read_policy_revision());
    a.configure(false, &[], "new A", "");
    let a2 = a.enter();
    assert_eq!(multi_value_separator(), "A");
    assert_eq!(read_policy_revision(), u64::MAX);
    drop(a1);
    assert_eq!(multi_value_separator(), "A");
    drop(a2);
    assert_eq!(multi_value_separator(), "B");
    drop(b1);
    let _fresh = a.enter();
    assert_eq!(multi_value_separator(), "new A");
    assert_ne!(read_policy_revision(), u64::MAX);
}

#[test]
fn same_owner_non_lifo_drop_keeps_pin_until_last_guard() {
    let a = policy("same");
    let outer = a.enter();
    let original = read_policy_revision();
    let inner = a.enter();
    drop(outer);
    assert_eq!(read_policy_revision(), original);
    a.configure(false, &[], "replacement", "");
    assert_eq!(multi_value_separator(), "same");
    assert_eq!(read_policy_revision(), u64::MAX);
    drop(inner);
    let _new = a.enter();
    assert_eq!(multi_value_separator(), "replacement");
}

#[test]
fn normalized_noop_settings_keep_generation_but_change_and_restore_does_not() {
    let a = ModelPolicy::default();
    let first = {
        let _scope = a.enter();
        read_policy_revision()
    };
    a.configure(false, &["unknown".into()], "", "unknown");
    let second = {
        let _scope = a.enter();
        read_policy_revision()
    };
    assert_eq!(first, second);
    let old = a.enter();
    a.configure(false, &[], "other", "");
    a.configure(false, &[], "", "");
    assert_eq!(read_policy_revision(), u64::MAX);
    drop(old);
    let _new = a.enter();
    assert_ne!(read_policy_revision(), first);
}

#[test]
fn cross_thread_owner_change_invalidates_only_that_owner_and_unwind_restores() {
    let a = policy("A");
    let peer = policy("peer");
    let rendezvous = Arc::new(Barrier::new(2));
    let cloned = a.clone();
    let gate = rendezvous.clone();
    let thread = std::thread::spawn(move || {
        let _scope = cloned.enter();
        let old = read_policy_revision();
        gate.wait();
        gate.wait();
        assert_eq!(read_policy_revision(), u64::MAX);
        assert_eq!(multi_value_separator(), "A");
        old
    });
    let _scope = peer.enter();
    let peer_generation = read_policy_revision();
    rendezvous.wait();
    a.configure(false, &[], "changed", "");
    rendezvous.wait();
    assert_ne!(thread.join().unwrap(), peer_generation);
    assert_eq!(read_policy_revision(), peer_generation);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _nested = a.enter();
        panic!("independent scope unwind");
    }))
    .is_err());
    assert_eq!(multi_value_separator(), "peer");
    assert_eq!(read_policy_revision(), peer_generation);
}
