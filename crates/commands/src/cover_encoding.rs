//! A request-local encoding pool. Strong cached bytes are bounded; a weak
//! index reuses canonical strings still owned by the current plan. Optional
//! indexes stop admitting new identities under pressure rather than growing
//! without bound; existing admitted live identities remain reusable.
use base64::Engine as _;
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Weak},
};
type Key = (u64, usize);
type EncodedImage = (Arc<[u8]>, Arc<str>);
use tagrex_core::resources::{Budget, Charge, Lease};
const IDENTITY_LIMIT: usize = 256;
const IDENTITY_PRUNE_INTERVAL: usize = 16;
const IDENTITY_BYTES: usize = 32 * 1024 * 1024;
const WEAK_LIMIT: usize = 4096;
const WEAK_CANDIDATES: usize = 8;
const WEAK_BYTES: usize = 64 * 1024 * 1024;
// Map/vector bookkeeping, Arc headers and the retained-resource lease. Charge
// even tiny images so metadata cannot dominate the accounted content bytes.
const HOT_ENTRY_OVERHEAD: usize = 192;

struct Identity {
    raw: Weak<[u8]>,
    encoded: Weak<str>,
    bytes: usize,
    _retained: Lease,
}
impl Identity {
    fn new(raw: &Arc<[u8]>, encoded: &Arc<str>) -> Option<Self> {
        let bytes = raw
            .len()
            .checked_add(encoded.len())?
            .checked_add(HOT_ENTRY_OVERHEAD)?;
        let retained = Budget::process()
            .try_acquire(Charge {
                retained: bytes,
                ..Charge::default()
            })
            .ok()?;
        Some(Self {
            raw: Arc::downgrade(raw),
            encoded: Arc::downgrade(encoded),
            bytes,
            _retained: retained,
        })
    }
}
struct WeakEncoding {
    value: Weak<str>,
    bytes: usize,
    _retained: Lease,
}

#[derive(Default)]
struct Pool {
    hot: HashMap<Key, Vec<EncodedImage>>,
    weak: HashMap<Key, Vec<WeakEncoding>>,
    weak_entries: usize,
    weak_bytes: usize,
    bytes: usize,
    identity: HashMap<usize, Identity>,
    identity_bytes: usize,
    encodes_since_prune: usize,
    retained: Vec<Lease>,
}
impl Pool {
    fn encode(&mut self, data: &Arc<[u8]>, limit: usize) -> Arc<str> {
        // Weak slices retain their allocations even after strong owners die.
        // Sweep on a bounded cadence, including identity hits: scanning on
        // every insertion penalizes diverse/tiny artwork and misses hit-only
        // traffic entirely. Dead entries survive at most this many calls (or
        // until Scope drops), remain byte-charged until removal, and all index
        // scans are capped by the fixed entry/candidate limits.
        self.encodes_since_prune += 1;
        if self.encodes_since_prune == IDENTITY_PRUNE_INTERVAL {
            self.prune_dead();
            self.encodes_since_prune = 0;
        }
        let address = Arc::as_ptr(data) as *const u8 as usize;
        if let Some(identity) = self.identity.get(&address) {
            if let Some(bytes) = identity.raw.upgrade() {
                if Arc::ptr_eq(&bytes, data) {
                    if let Some(encoded) = identity.encoded.upgrade() {
                        return encoded;
                    }
                }
            }
            if let Some(old) = self.identity.remove(&address) {
                self.identity_bytes -= old.bytes;
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        let key = (hasher.finish(), data.len());
        if let Some(found) = self.hot.get(&key).and_then(|entries| {
            entries
                .iter()
                .find(|(bytes, _)| bytes.as_ref() == data.as_ref())
        }) {
            let encoded = found.1.clone();
            self.remember_identity(data, &encoded);
            return encoded;
        }
        let found = self.weak.get(&key).and_then(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.value.upgrade())
                .find(|encoded| {
                    // Hash narrows the candidates; exact bytes decide identity.
                    same_encoding(encoded, data)
                })
        });
        let encoded = if let Some(found) = found {
            found
        } else {
            let encoded: Arc<str> = base64::engine::general_purpose::STANDARD
                .encode(data)
                .into();
            self.remember_weak(key, &encoded);
            encoded
        };
        let cost = data
            .len()
            .saturating_add(encoded.len())
            .saturating_add(HOT_ENTRY_OVERHEAD);
        if cost <= limit {
            if let Ok(retained) = Budget::process().try_acquire(Charge {
                retained: cost,
                ..Charge::default()
            }) {
                // Secure new ownership before releasing the old hot cache;
                // pressure must not clear useful strong entries for no result.
                if self.bytes.saturating_add(cost) > limit {
                    self.hot.clear();
                    self.bytes = 0;
                    self.retained.clear();
                    self.prune_dead();
                }
                self.bytes += cost;
                self.retained.push(retained);
                self.hot
                    .entry(key)
                    .or_default()
                    .push((data.clone(), encoded.clone()));
            }
        }
        self.remember_identity(data, &encoded);
        encoded
    }

    fn prune_dead(&mut self) {
        self.identity
            .retain(|_, entry| entry.raw.strong_count() > 0 && entry.encoded.strong_count() > 0);
        self.identity_bytes = self.identity.values().map(|entry| entry.bytes).sum();
        self.weak.retain(|_, entries| {
            entries.retain(|entry| entry.value.strong_count() > 0);
            !entries.is_empty()
        });
        self.weak_entries = self.weak.values().map(Vec::len).sum();
        self.weak_bytes = self.weak.values().flatten().map(|entry| entry.bytes).sum();
    }

    fn remember_weak(&mut self, key: Key, encoded: &Arc<str>) {
        let bytes = encoded.len().saturating_add(HOT_ENTRY_OVERHEAD);
        if bytes > WEAK_BYTES {
            return;
        }
        if self.weak_entries >= WEAK_LIMIT
            || self.weak_bytes > WEAK_BYTES - bytes
            || self
                .weak
                .get(&key)
                .is_some_and(|entries| entries.len() >= WEAK_CANDIDATES)
        {
            return;
        }
        let Ok(retained) = Budget::process().try_acquire(Charge {
            retained: bytes,
            ..Charge::default()
        }) else {
            return;
        };
        self.weak.entry(key).or_default().push(WeakEncoding {
            value: Arc::downgrade(encoded),
            bytes,
            _retained: retained,
        });
        self.weak_entries += 1;
        self.weak_bytes += bytes;
    }

    fn remember_identity(&mut self, data: &Arc<[u8]>, encoded: &Arc<str>) {
        // A temporary equal-but-separate input should not keep its allocation
        // reserved through a Weak after the caller drops it. Shared inputs (or
        // bytes already owned by the hot cache) are the useful identity case.
        if Arc::strong_count(data) == 1 {
            return;
        }
        let cost = data
            .len()
            .saturating_add(encoded.len())
            .saturating_add(HOT_ENTRY_OVERHEAD);
        if cost > IDENTITY_BYTES {
            return;
        }
        if self.identity.len() >= IDENTITY_LIMIT || self.identity_bytes > IDENTITY_BYTES - cost {
            return;
        }
        // The address only selects a candidate. Upgrading and ptr_eq validate
        // its allocation, so allocator reuse cannot accept a stale entry.
        // Arc::make_mut dissociates Weak pointers (or clones shared bytes),
        // making subsequent edits miss this index and use content matching.
        // Weak slices keep payload allocations alive after their last strong
        // owner dies; reserve that lifetime until this index actually prunes it.
        if let Some(identity) = Identity::new(data, encoded) {
            if let Some(old) = self
                .identity
                .insert(Arc::as_ptr(data) as *const u8 as usize, identity)
            {
                self.identity_bytes -= old.bytes;
            }
            self.identity_bytes += cost;
        }
    }
}

/// Exact content verification with fixed scratch space. Hash collisions never
/// trigger a full decoded-image allocation or accept unequal picture bytes.
fn same_encoding(encoded: &str, data: &[u8]) -> bool {
    if base64::encoded_len(data.len(), true) != Some(encoded.len()) {
        return false;
    }
    let mut scratch = [0u8; 4096];
    data.chunks(3072)
        .zip(encoded.as_bytes().chunks(4096))
        .all(|(raw, expected)| {
            base64::engine::general_purpose::STANDARD
                .encode_slice(raw, &mut scratch)
                .is_ok_and(|written| scratch[..written] == *expected)
        })
}
thread_local! { static POOL: RefCell<Option<Pool>> = const { RefCell::new(None) }; }
pub(crate) struct Scope {
    owns: bool,
}
impl Scope {
    pub fn new() -> Self {
        let owns = POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.is_none() {
                *pool = Some(Pool::default());
                true
            } else {
                false
            }
        });
        Self { owns }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.owns {
            POOL.with(|pool| {
                pool.borrow_mut().take();
            });
        }
    }
}
pub(crate) fn encode(data: &Arc<[u8]>) -> Arc<str> {
    POOL.with(|pool| match pool.borrow_mut().as_mut() {
        Some(pool) => pool.encode(data, 32 * 1024 * 1024),
        None => base64::engine::general_purpose::STANDARD
            .encode(data)
            .into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evicted_or_oversized_images_reuse_live_plan_strings() {
        let mut pool = Pool::default();
        let first = pool.encode(&Arc::from(vec![1; 1024]), 64);
        for _ in 0..100 {
            let next = pool.encode(&Arc::from(vec![1; 1024]), 64);
            assert!(Arc::ptr_eq(&first, &next));
        }
        assert_eq!(pool.bytes, 0);
        assert!(pool.hot.is_empty());
    }
    #[test]
    fn byte_comparison_rejects_a_deliberately_colliding_candidate() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![1; 100].into();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hash);
        let wrong: Arc<str> = base64::engine::general_purpose::STANDARD
            .encode(vec![2; 100])
            .into();
        pool.remember_weak((hash.finish(), bytes.len()), &wrong);
        let result = pool.encode(&bytes, 64);
        assert_ne!(result, wrong);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(result.as_bytes())
                .unwrap(),
            bytes.as_ref()
        );
    }
    fn decoded(encoded: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .unwrap()
    }

    #[test]
    fn live_identity_survives_eviction_without_content_candidates() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![7; 1024].into();
        let first = pool.encode(&bytes, 4096);
        // Simulate eviction and remove fallback candidates to require the
        // identity path. Clones must still return the original plan string.
        pool.hot.clear();
        pool.bytes = 0;
        pool.weak.clear();
        for _ in 0..100 {
            assert!(Arc::ptr_eq(&first, &pool.encode(&bytes.clone(), 64)));
        }
        assert_eq!(decoded(&first), bytes.as_ref());
        assert_eq!(pool.bytes, 0);
    }

    #[test]
    fn separate_equal_allocations_still_share_canonical_strings() {
        let mut pool = Pool::default();
        let a: Arc<[u8]> = vec![7; 1024].into();
        let b: Arc<[u8]> = vec![7; 1024].into();
        assert!(!Arc::ptr_eq(&a, &b));
        let first = pool.encode(&a, 0);
        let next = pool.encode(&b, 0);
        assert!(Arc::ptr_eq(&first, &next));
        assert!(pool.identity.is_empty());
        let shared = b.clone();
        assert!(Arc::ptr_eq(&first, &pool.encode(&shared, 0)));
        assert_eq!(pool.identity.len(), 1);
        drop(shared);
        assert_eq!(Arc::strong_count(&a), 1);
        assert_eq!(Arc::strong_count(&b), 1);
    }

    #[test]
    fn different_bytes_of_same_length_do_not_alias() {
        let mut pool = Pool::default();
        let a: Arc<[u8]> = vec![1; 1024].into();
        let b: Arc<[u8]> = vec![2; 1024].into();
        let first = pool.encode(&a, 0);
        let next = pool.encode(&b, 0);
        assert!(!Arc::ptr_eq(&first, &next));
        assert_eq!(decoded(&first), a.as_ref());
        assert_eq!(decoded(&next), b.as_ref());
    }

    #[test]
    fn stale_or_wrong_allocation_candidate_is_never_trusted() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![1; 1024].into();
        let other: Arc<[u8]> = vec![2; 1024].into();
        let wrong = pool.encode(&other, 0);
        let address = Arc::as_ptr(&bytes) as *const u8 as usize;
        // Deliberately simulate an address-key collision, both while the old
        // owner is live and after it dies. No allocator timing is required.
        pool.identity
            .insert(address, Identity::new(&other, &wrong).unwrap());
        pool.identity_bytes = pool.identity.values().map(|entry| entry.bytes).sum();
        let first = pool.encode(&bytes, 0);
        assert_eq!(decoded(&first), bytes.as_ref());
        let dead = Arc::downgrade(&other);
        let dead_identity = Identity::new(&other, &wrong).unwrap();
        drop(other);
        assert!(dead.upgrade().is_none());
        pool.identity.insert(address, dead_identity);
        pool.identity_bytes = pool.identity.values().map(|entry| entry.bytes).sum();
        assert!(Arc::ptr_eq(&first, &pool.encode(&bytes, 0)));
    }

    #[test]
    fn make_mut_invalidates_identity_for_changed_bytes() {
        let mut pool = Pool::default();
        let mut bytes: Arc<[u8]> = vec![1; 1024].into();
        let shared = bytes.clone();
        let before = pool.encode(&bytes, 0);
        drop(shared);
        assert_eq!(pool.identity.len(), 1);
        let old_owner = Arc::downgrade(&bytes);
        Arc::make_mut(&mut bytes)[0] = 2;
        assert!(old_owner.upgrade().is_none());
        let after = pool.encode(&bytes, 0);
        assert_eq!(decoded(&before), vec![1; 1024]);
        assert_eq!(decoded(&after), bytes.as_ref());
        assert!(!Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn identity_is_bounded_and_prunes_dead_owners() {
        let mut pool = Pool::default();
        let mut keep = Vec::new();
        for i in 0..IDENTITY_LIMIT * 3 {
            let bytes: Arc<[u8]> = (i as u64).to_le_bytes().to_vec().into();
            let shared = bytes.clone();
            let encoded = pool.encode(&shared, 0);
            keep.push((bytes, encoded));
            assert!(pool.identity.len() <= IDENTITY_LIMIT);
        }
        drop(keep);
        for _ in 0..64 {
            let bytes: Arc<[u8]> = vec![5; 16].into();
            pool.encode(&bytes, 0);
        }
        assert!(pool.identity.is_empty());
    }

    #[test]
    fn expired_encoded_string_is_recreated() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![7; 1024].into();
        let first = pool.encode(&bytes.clone(), 0);
        assert_eq!(pool.identity.len(), 1);
        let weak = Arc::downgrade(&first);
        drop(first);
        assert!(weak.upgrade().is_none());
        assert_eq!(decoded(&pool.encode(&bytes, 0)), bytes.as_ref());
    }

    #[test]
    fn identity_hits_prune_dead_bytes_and_dead_strings_within_interval() {
        let mut pool = Pool::default();
        let live: Arc<[u8]> = vec![7; 8].into();
        let first = pool.encode(&live.clone(), 0);
        let dead_bytes: Arc<[u8]> = vec![8; 1024].into();
        let dead_address = Arc::as_ptr(&dead_bytes) as *const u8 as usize;
        let retained_string = pool.encode(&dead_bytes.clone(), 0);
        let retained_bytes: Arc<[u8]> = vec![9; 1024].into();
        let dead_string_address = Arc::as_ptr(&retained_bytes) as *const u8 as usize;
        let dead_string = pool.encode(&retained_bytes.clone(), 0);
        drop(dead_bytes);
        drop(dead_string);
        assert!(pool.identity.contains_key(&dead_address));
        assert!(pool.identity.contains_key(&dead_string_address));
        for _ in 0..IDENTITY_PRUNE_INTERVAL {
            assert!(Arc::ptr_eq(&first, &pool.encode(&live, 0)));
        }
        assert!(!pool.identity.contains_key(&dead_address));
        assert!(!pool.identity.contains_key(&dead_string_address));
        assert_eq!(pool.identity.len(), 1);
        assert_eq!(decoded(&retained_string), vec![8; 1024]);
    }

    #[test]
    fn pruning_keeps_live_shared_and_equal_copy_canonical_strings() {
        let mut pool = Pool::default();
        let live: Arc<[u8]> = vec![7; 8].into();
        let first = pool.encode(&live.clone(), 0);
        for i in 0..IDENTITY_PRUNE_INTERVAL * 3 {
            let bytes: Arc<[u8]> = (i as u64).to_le_bytes().to_vec().into();
            let result = pool.encode(&bytes.clone(), 0);
            assert_eq!(decoded(&result), bytes.as_ref());
            assert!(Arc::ptr_eq(&first, &pool.encode(&live, 0)));
            let equal: Arc<[u8]> = live.as_ref().into();
            assert!(Arc::ptr_eq(&first, &pool.encode(&equal, 0)));
        }
        assert_eq!(pool.bytes, 0);
        assert!(pool.identity.len() <= IDENTITY_LIMIT);
    }

    #[test]
    fn hot_budget_remains_bounded_and_evicted_identity_remains_weak() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![1; 100].into();
        let first = pool.encode(&bytes, 240 + HOT_ENTRY_OVERHEAD);
        assert_eq!(pool.bytes, 236 + HOT_ENTRY_OVERHEAD);
        let other: Arc<[u8]> = vec![2; 100].into();
        let next = pool.encode(&other, 240 + HOT_ENTRY_OVERHEAD);
        assert_eq!(pool.bytes, 236 + HOT_ENTRY_OVERHEAD);
        assert_eq!(Arc::strong_count(&bytes), 1);
        assert!(Arc::ptr_eq(
            &first,
            &pool.encode(&bytes, 240 + HOT_ENTRY_OVERHEAD)
        ));
        assert_eq!(decoded(&next), other.as_ref());
        let oversized: Arc<[u8]> = vec![3; 1024].into();
        let large = pool.encode(&oversized, 240 + HOT_ENTRY_OVERHEAD);
        assert_eq!(Arc::strong_count(&oversized), 1);
        assert!(Arc::ptr_eq(
            &large,
            &pool.encode(&oversized, 240 + HOT_ENTRY_OVERHEAD)
        ));
        assert_eq!(pool.bytes, 236 + HOT_ENTRY_OVERHEAD);
    }

    #[test]
    fn transient_equal_inputs_do_not_enter_identity_index() {
        let mut pool = Pool::default();
        let original: Arc<[u8]> = vec![7; 1024].into();
        let first = pool.encode(&original, 4096);
        assert_eq!(pool.identity.len(), 1);
        for _ in 0..100 {
            let transient: Arc<[u8]> = original.as_ref().into();
            let address = Arc::as_ptr(&transient) as *const u8 as usize;
            assert!(Arc::ptr_eq(&first, &pool.encode(&transient, 4096)));
            assert!(!pool.identity.contains_key(&address));
            assert_eq!(pool.identity.len(), 1);
        }
    }

    #[test]
    fn hot_hash_collision_also_requires_exact_bytes() {
        let mut pool = Pool::default();
        let bytes: Arc<[u8]> = vec![1; 100].into();
        let other: Arc<[u8]> = vec![2; 100].into();
        let wrong: Arc<str> = base64::engine::general_purpose::STANDARD
            .encode(&other)
            .into();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hash);
        pool.hot
            .insert((hash.finish(), bytes.len()), vec![(other, wrong.clone())]);
        let result = pool.encode(&bytes, 0);
        assert_ne!(result, wrong);
        assert_eq!(decoded(&result), bytes.as_ref());
    }

    #[test]
    fn nested_scopes_share_only_until_request_owner_drops() {
        let bytes: Arc<[u8]> = vec![1; 100].into();
        let first;
        {
            let _outer = Scope::new();
            first = encode(&bytes);
            {
                let _inner = Scope::new();
                assert!(Arc::ptr_eq(&first, &encode(&bytes)));
            }
            assert!(Arc::ptr_eq(&first, &encode(&bytes)));
        }
        POOL.with(|pool| assert!(pool.borrow().is_none()));
        let _next_request = Scope::new();
        let next = encode(&bytes);
        assert_eq!(first, next);
        assert!(!Arc::ptr_eq(&first, &next));
    }

    #[test]
    fn long_requests_bound_live_candidates_and_prune_dead_payload_owners() {
        let mut pool = Pool::default();
        let seed: Arc<[u8]> = vec![7; 1024].into();
        let first = pool.encode(&seed.clone(), 0);
        let mut live = Vec::new();
        for i in 0..20_000u64 {
            let raw: Arc<[u8]> = i.to_le_bytes().to_vec().into();
            let result = pool.encode(&raw, 0);
            if i < 8000 {
                live.push(result);
            }
            assert!(pool.weak_entries <= WEAK_LIMIT);
            assert!(pool.weak_bytes <= WEAK_BYTES);
            assert!(pool.identity.len() <= IDENTITY_LIMIT);
            assert!(pool.identity_bytes <= IDENTITY_BYTES);
            assert!(Arc::ptr_eq(&first, &pool.encode(&seed.clone(), 0)));
        }
        assert!(pool
            .weak
            .values()
            .all(|entries| entries.len() <= WEAK_CANDIDATES));
        drop(live);
        pool.prune_dead();
        assert_eq!(pool.weak_entries, 1);
        assert_eq!(pool.identity.len(), 1);
    }

    #[test]
    fn content_collision_checks_use_bounded_scratch_at_base64_boundaries() {
        for length in [0, 1, 2, 3, 3071, 3072, 3073, 6144, 65537] {
            let bytes = vec![5; length];
            let text = base64::engine::general_purpose::STANDARD.encode(&bytes);
            assert!(same_encoding(&text, &bytes));
            if !bytes.is_empty() {
                let mut changed = bytes;
                changed[length - 1] = 6;
                assert!(!same_encoding(&text, &changed));
            }
        }
    }

    #[test]
    fn refused_hot_admission_preserves_old_entries_and_empty_images_charge_metadata() {
        const ENV: &str = "TAGREX_POOL_ADMISSION_TEST_CHILD";
        if std::env::var_os(ENV).is_none() {
            // This source also compiles inside the independent review binary;
            // use its actual module path while dropping the crate-name prefix.
            let module = module_path!().split_once("::").unwrap().1;
            let case = format!("{module}::refused_hot_admission_preserves_old_entries_and_empty_images_charge_metadata");
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &case, "--nocapture"])
                .env(ENV, "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let budget = Budget::process();
        let baseline = budget.usage().0;
        let mut pool = Pool::default();
        let old: Arc<[u8]> = vec![1; 100].into();
        let encoded = pool.encode(&old, 500);
        let retained = pool.bytes;
        let pressure = budget
            .try_acquire(Charge {
                retained: 256 * 1024 * 1024 - budget.usage().0.retained,
                ..Charge::default()
            })
            .unwrap();
        let newcomer: Arc<[u8]> = vec![2; 101].into();
        let refused = pool.encode(&newcomer, 500);
        assert_eq!(decoded(&refused), newcomer.as_ref());
        assert_eq!(pool.bytes, retained);
        assert!(pool
            .hot
            .values()
            .flatten()
            .any(|(raw, _)| Arc::ptr_eq(raw, &old)));
        let mut changed = old.clone();
        Arc::make_mut(&mut changed)[0] = 9;
        assert_eq!(decoded(&pool.encode(&changed, 500)), changed.as_ref());
        assert!(Arc::ptr_eq(&encoded, &pool.encode(&old, 500)));
        pool.encode(&Arc::from(vec![3; 4000]), 500);
        assert_eq!(pool.bytes, retained);
        assert!(pool
            .hot
            .values()
            .flatten()
            .any(|(raw, _)| Arc::ptr_eq(raw, &old)));
        drop(pressure);
        pool.encode(&newcomer, 500);
        assert!(pool
            .hot
            .values()
            .flatten()
            .any(|(raw, _)| Arc::ptr_eq(raw, &newcomer)));
        drop(pool);
        assert_eq!(budget.usage().0, baseline);
        let mut empty = Pool::default();
        let bytes: Arc<[u8]> = Arc::from([]);
        empty.encode(&bytes, HOT_ENTRY_OVERHEAD - 1);
        assert!(empty.hot.is_empty());
        assert_eq!(empty.weak_bytes, HOT_ENTRY_OVERHEAD);
        empty.encode(&bytes, HOT_ENTRY_OVERHEAD);
        assert_eq!(empty.bytes, HOT_ENTRY_OVERHEAD);
        drop(empty);
        assert_eq!(budget.usage().0, baseline);
    }
}
