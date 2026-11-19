//! Byte- and entry-bounded LRU storage for immutable derived query data.
use std::{
    borrow::Borrow,
    collections::{BTreeMap, HashMap},
    hash::Hash,
};
use tagrex_core::resources::{Budget, Charge, Lease};

#[derive(Debug)]
pub(crate) struct Cache<K, V> {
    entries: HashMap<K, (V, usize, u64, Lease)>,
    order: BTreeMap<u64, K>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
    budget: Budget,
}
impl<K: Clone + Eq + Hash, V> Cache<K, V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            bytes: 0,
            clock: 0,
            max_entries,
            max_bytes,
            budget: Budget::process(),
        }
    }
    /// The admission limits this store was built with. Test-only: callers must
    /// never rely on them instead of the documented per-cache budget.
    #[cfg(test)]
    pub fn limits(&self) -> (usize, usize) {
        (self.max_entries, self.max_bytes)
    }
    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            // Keep LRU order on counter rollover, including long-lived sessions.
            self.order = self
                .order
                .values()
                .cloned()
                .enumerate()
                .map(|(i, key)| {
                    let stamp = i as u64;
                    if let Some(entry) = self.entries.get_mut(&key) {
                        entry.2 = stamp;
                    }
                    (stamp, key)
                })
                .collect();
            self.clock = self.order.len() as u64;
        }
        self.clock += 1;
        self.clock
    }
    pub fn get<Q: ?Sized + Eq + Hash>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        if !self.entries.contains_key(key) {
            return None;
        }
        let stamp = self.tick();
        let (owned, entry) = self.entries.get_key_value(key)?;
        let owned = owned.clone();
        self.order.remove(&entry.2);
        self.order.insert(stamp, owned);
        let entry = self.entries.get_mut(key)?;
        entry.2 = stamp;
        Some(&entry.0)
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if let Some((_, size, stamp, _)) = self.entries.remove(&key) {
            self.bytes -= size;
            self.order.remove(&stamp);
        }
        if self.max_entries == 0 || bytes > self.max_bytes {
            return;
        }
        // Invalidate a replaced key above, but do not evict unrelated useful
        // data until the newcomer's shared admission is secured. Conservatively
        // skip retention if the simultaneous old+new charge cannot fit.
        let Ok(retained) = self.budget.try_acquire(Charge {
            retained: bytes,
            ..Charge::default()
        }) else {
            return;
        };
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            let Some((_, key)) = self.order.pop_first() else {
                break;
            };
            if let Some((_, size, _, _)) = self.entries.remove(&key) {
                self.bytes -= size;
            }
        }
        let stamp = self.tick();
        self.order.insert(stamp, key.clone());
        self.bytes += bytes;
        self.entries.insert(key, (value, bytes, stamp, retained));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hot_entries_survive_count_byte_pressure_and_clock_rollover() {
        let mut c = Cache::new(3, 10);
        c.insert("a".to_string(), 1, 3);
        c.insert("b".to_string(), 2, 3);
        c.insert("c".to_string(), 3, 3);
        assert_eq!(c.get("a"), Some(&1));
        c.insert("d".into(), 4, 4);
        assert!(c.get("b").is_none());
        assert_eq!(c.get("a"), Some(&1));
        c.clock = u64::MAX;
        assert_eq!(c.get("c"), Some(&3));
        c.insert("e".into(), 5, 4);
        assert!(c.get("d").is_none());
        assert!(c.bytes <= 10);
        assert_eq!(c.entries.len(), c.order.len());
        assert_eq!(c.bytes, c.entries.values().map(|e| e.1).sum::<usize>());
    }
    #[test]
    fn oversize_and_disabled_entries_are_not_retained_and_replacement_is_accounted() {
        let mut c = Cache::new(2, 5);
        c.insert(1, 1, 4);
        c.insert(1, 2, 3);
        assert_eq!(c.bytes, 3);
        assert_eq!(c.get(&1), Some(&2));
        c.insert(2, 3, 6);
        assert!(c.get(&2).is_none());
        assert_eq!(c.get(&1), Some(&2));
        let mut c = Cache::new(0, 100);
        c.insert(1, 1, 1);
        assert!(c.get(&1).is_none());
    }

    #[test]
    fn global_pressure_skips_admission_and_drop_returns_budget_to_other_caches() {
        use tagrex_core::resources::Limits;
        let budget = Budget::new(Limits {
            retained: 12,
            transient: 12,
            total: 12,
            jobs: 1,
        });
        let mut first = Cache::new(10, 100);
        first.budget = budget.clone();
        let mut second = Cache::new(10, 100);
        second.budget = budget.clone();
        first.insert(1, "first", 8);
        second.insert(2, "refused", 8);
        assert_eq!(first.get(&1), Some(&"first"));
        assert!(second.get(&2).is_none());
        assert_eq!(budget.usage().0.retained, 8);
        drop(first);
        second.insert(2, "now admitted", 8);
        assert_eq!(second.get(&2), Some(&"now admitted"));
        drop(second);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn refused_newcomer_preserves_hot_entries_but_refused_replacement_invalidates_its_key() {
        use tagrex_core::resources::Limits;
        let budget = Budget::new(Limits {
            retained: 20,
            transient: 20,
            total: 20,
            jobs: 1,
        });
        let mut cache = Cache::new(2, 10);
        cache.budget = budget.clone();
        cache.insert(1, "hot", 8);
        let pressure = budget
            .try_acquire(Charge {
                retained: 12,
                ..Charge::default()
            })
            .unwrap();
        cache.insert(2, "refused", 9);
        assert_eq!(cache.get(&1), Some(&"hot"));
        assert!(cache.get(&2).is_none());
        cache.insert(3, "oversized", usize::MAX);
        assert_eq!(cache.get(&1), Some(&"hot"));
        drop(pressure);
        cache.insert(2, "admitted after release", 9);
        assert_eq!(cache.get(&2), Some(&"admitted after release"));
        assert!(cache.get(&1).is_none());
        drop(cache);

        let mut cache = Cache::new(4, 20);
        cache.budget = budget.clone();
        cache.insert(1, "stale", 6);
        cache.insert(2, "unrelated hot", 4);
        let pressure = budget
            .try_acquire(Charge {
                retained: 10,
                ..Charge::default()
            })
            .unwrap();
        cache.insert(1, "new but cannot retain", 7);
        assert!(cache.get(&1).is_none());
        assert_eq!(cache.get(&2), Some(&"unrelated hot"));
        assert_eq!(budget.usage().0.retained, 14);
        drop(pressure);
        drop(cache);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn zero_estimates_still_obey_entry_limits_and_disabled_caches_do_not_retain() {
        // Cost is caller-supplied inclusive accounting. A zero estimate must
        // never bypass this store's independent entry-count boundary.
        let mut cache = Cache::new(2, 0);
        for key in 0..1000 {
            cache.insert(key, (), 0);
            assert!(cache.entries.len() <= 2);
            assert_eq!(cache.entries.len(), cache.order.len());
            assert_eq!(cache.bytes, 0);
        }
        let mut disabled = Cache::new(0, 0);
        disabled.insert(1, (), 0);
        assert!(disabled.entries.is_empty());
    }
}
