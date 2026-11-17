//! Bounded, fingerprint-validated display/preview snapshots. Executor staleness
//! checks never use this cache. Failed reads are never cached.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::SystemTime,
};

use tagrex_core::model::{TagEngine, TagIoError, TrackRead};

#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Fingerprint {
    fn of(path: &Path) -> Option<Self> {
        let m = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            len: m.len(),
            modified: m.modified().ok()?,
            #[cfg(unix)]
            identity: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
        })
    }
}

struct Entry {
    stamp: Fingerprint,
    read: TrackRead,
    bytes: usize,
    used_at: u64,
}

/// Request-independent display snapshots bounded by entry count and estimated
/// metadata bytes. Writes and Undo clear these; executor checks bypass them.
pub(crate) struct ReadCache {
    entries: HashMap<PathBuf, Entry>,
    bytes: usize,
    order: BTreeMap<u64, PathBuf>,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for ReadCache {
    fn default() -> Self {
        Self::with_limits(65_536, 64 * 1024 * 1024)
    }
}

impl ReadCache {
    fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            order: BTreeMap::new(),
            clock: 0,
            max_entries,
            max_bytes,
        }
    }

    /// Discard all snapshots after writes or a session change.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.order.clear();
        self.clock = 0;
    }

    /// Reuse a snapshot only while its filesystem fingerprint still matches.
    /// Failed or visibly concurrent reads are never added to the cache.
    pub fn read(&mut self, path: &Path) -> Result<TrackRead, TagIoError> {
        // Rebase exceptionally long-lived sessions without wrapping usage keys.
        if self.clock == u64::MAX {
            self.clear();
        }
        self.clock += 1;
        let used_at = self.clock;
        let before = Fingerprint::of(path);
        if let Some(entry) = self.entries.get_mut(path) {
            if before.as_ref() == Some(&entry.stamp) {
                self.order.remove(&entry.used_at);
                self.order.insert(used_at, path.to_path_buf());
                entry.used_at = used_at;
                return Ok(entry.read.clone());
            }
        }
        if let Some(old) = self.entries.remove(path) {
            self.bytes -= old.bytes;
            self.order.remove(&old.used_at);
        }
        let read = TagEngine::read_with_props(path)?;
        // Cache only when the observed fingerprint is unchanged across the read.
        // This does not detect every concurrent or same-size external edit: on
        // Windows, length + mtime can collide on coarse timestamp filesystems.
        // Executor staleness checks always re-read the file independently.
        if let Some(stamp) = before.filter(|s| Some(s) == Fingerprint::of(path).as_ref()) {
            // Lookup, recency and the snapshot each own a path. Account for
            // retained capacities, not just visible text, with saturating sums.
            // Allocator overhead is estimated; this is not an RSS guarantee.
            let bytes = read.file.tags.iter().fold(
                640usize
                    .saturating_add(path.as_os_str().len().saturating_mul(2))
                    .saturating_add(read.file.path.capacity())
                    .saturating_add(
                        read.blocks
                            .capacity()
                            .saturating_mul(std::mem::size_of::<tagrex_core::model::TagBlock>()),
                    ),
                |bytes, (key, value)| {
                    let key_bytes = match key {
                        tagrex_core::model::TagField::Custom(name) => name.capacity(),
                        _ => 0,
                    };
                    bytes
                        .saturating_add(key_bytes)
                        .saturating_add(value.capacity())
                        .saturating_add(96)
                },
            );
            if self.max_entries > 0 && bytes <= self.max_bytes {
                while self.bytes > self.max_bytes - bytes || self.entries.len() >= self.max_entries
                {
                    let Some((_, oldest)) = self.order.pop_first() else {
                        break;
                    };
                    if let Some(entry) = self.entries.remove(&oldest) {
                        self.bytes -= entry.bytes;
                    }
                }
                self.bytes += bytes;
                self.order.insert(used_at, path.to_path_buf());
                self.entries.insert(
                    path.to_path_buf(),
                    Entry {
                        stamp,
                        read: read.clone(),
                        bytes,
                        used_at,
                    },
                );
            }
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tagrex_core::model::TagField;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-cache-lru-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn track(&self, name: &str, title: &str) -> PathBuf {
            let path = self.0.join(name);
            let mut wav = Vec::new();
            wav.extend(b"RIFF");
            wav.extend(356u32.to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(8000u32.to_le_bytes());
            wav.extend(16000u32.to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(320u32.to_le_bytes());
            wav.extend([0; 320]);
            std::fs::write(&path, wav).unwrap();
            let mut file = TagEngine::read(&path).unwrap();
            file.tags.insert(TagField::Title, title.into());
            TagEngine::write(&file).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn consistent(cache: &ReadCache) {
        assert_eq!(cache.entries.len(), cache.order.len());
        assert_eq!(
            cache.bytes,
            cache
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
        );
        assert!(cache.entries.len() <= cache.max_entries);
        assert!(cache.bytes <= cache.max_bytes);
        for (path, entry) in &cache.entries {
            assert_eq!(cache.order.get(&entry.used_at), Some(path));
        }
    }
    #[test]
    fn lru_capacity_retains_recently_used_entries_instead_of_clearing_everything() {
        let dir = Fixture::new();
        let a = dir.track("a.wav", "A");
        let b = dir.track("b.wav", "B");
        let c = dir.track("c.wav", "C");
        let mut cache = ReadCache::with_limits(2, 64 * 1024);
        cache.read(&a).unwrap();
        cache.read(&b).unwrap();
        cache.read(&a).unwrap();
        cache.read(&c).unwrap();
        assert!(cache.entries.contains_key(&a));
        assert!(cache.entries.contains_key(&c));
        assert!(!cache.entries.contains_key(&b));
        for _ in 0..1000 {
            cache.read(&a).unwrap();
            cache.read(&c).unwrap();
        }
        consistent(&cache);
    }
    #[test]
    fn byte_pressure_evicts_only_necessary_entries_and_skips_oversized_reads() {
        let dir = Fixture::new();
        let a = dir.track("a.wav", "A");
        let b = dir.track("b.wav", "B");
        let c = dir.track("c.wav", "C");
        let large = dir.track("large.wav", &"x".repeat(10000));
        let mut cache = ReadCache::with_limits(20, 64 * 1024);
        cache.read(&a).unwrap();
        cache.read(&b).unwrap();
        cache.max_bytes = cache.bytes;
        cache.read(&c).unwrap();
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries.contains_key(&b));
        assert!(cache.entries.contains_key(&c));
        assert_eq!(
            cache.read(&large).unwrap().file.tags[&TagField::Title].len(),
            10000
        );
        assert!(!cache.entries.contains_key(&large));
        assert_eq!(cache.entries.len(), 2);
        consistent(&cache);
    }
    #[test]
    fn external_writes_and_failed_reads_never_reuse_stale_metadata() {
        let dir = Fixture::new();
        let path = dir.track("one.wav", "Old");
        let mut cache = ReadCache::default();
        let mut snapshot = cache.read(&path).unwrap();
        snapshot
            .file
            .tags
            .insert(TagField::Title, "Local only".into());
        assert_eq!(
            cache.read(&path).unwrap().file.tags[&TagField::Title],
            "Old"
        );
        let mut changed = TagEngine::read(&path).unwrap();
        changed
            .tags
            .insert(TagField::Title, "External longer".into());
        TagEngine::write(&changed).unwrap();
        assert_eq!(
            cache.read(&path).unwrap().file.tags[&TagField::Title],
            "External longer"
        );
        std::fs::remove_file(&path).unwrap();
        assert!(cache.read(&path).is_err());
        assert!(cache.entries.is_empty());
        consistent(&cache);
    }
    #[cfg(unix)]
    #[test]
    fn same_size_write_with_restored_modification_time_invalidates_the_snapshot() {
        let dir = Fixture::new();
        let path = dir.track("same-size.wav", "Old");
        let mut cache = ReadCache::default();
        cache.read(&path).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        let mut file = TagEngine::read(&path).unwrap();
        file.tags.insert(TagField::Title, "New".into());
        TagEngine::write(&file).unwrap();
        let handle = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        handle
            .set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before.len());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before.modified().unwrap()
        );
        assert_eq!(
            cache.read(&path).unwrap().file.tags[&TagField::Title],
            "New"
        );
        consistent(&cache);
    }
    #[test]
    fn disabled_cache_clear_and_usage_clock_overflow_keep_bounds_consistent() {
        let dir = Fixture::new();
        let path = dir.track("one.wav", "Title");
        let mut disabled = ReadCache::with_limits(0, 0);
        disabled.read(&path).unwrap();
        consistent(&disabled);
        assert!(disabled.entries.is_empty());
        let mut cache = ReadCache::default();
        cache.read(&path).unwrap();
        cache.clock = u64::MAX;
        cache.read(&path).unwrap();
        assert_eq!(cache.clock, 1);
        consistent(&cache);
        cache.clear();
        assert_eq!(cache.bytes, 0);
        assert!(cache.order.is_empty());
        consistent(&cache);
    }
}
