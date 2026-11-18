//! Byte- and entry-bounded LRU storage for immutable derived query data.
use std::{
    borrow::Borrow,
    collections::{BTreeMap, HashMap},
    hash::Hash,
};

#[derive(Debug)]
pub(crate) struct Cache<K, V> {
    entries: HashMap<K, (V, usize, u64)>,
    order: BTreeMap<u64, K>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
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
        if let Some((_, size, stamp)) = self.entries.remove(&key) {
            self.bytes -= size;
            self.order.remove(&stamp);
        }
        if self.max_entries == 0 || bytes > self.max_bytes {
            return;
        }
        while self.entries.len() >= self.max_entries || self.bytes > self.max_bytes - bytes {
            let Some((_, key)) = self.order.pop_first() else {
                break;
            };
            if let Some((_, size, _)) = self.entries.remove(&key) {
                self.bytes -= size;
            }
        }
        let stamp = self.tick();
        self.order.insert(stamp, key.clone());
        self.bytes += bytes;
        self.entries.insert(key, (value, bytes, stamp));
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
}
