//! Bounded, fingerprint-validated display/preview snapshots. Executor staleness
//! checks never use this cache. Failed reads are never cached.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use tagrex_core::model::{TagEngine, TagIoError, TrackRead};
use tagrex_core::resources::{Budget, Charge, Lease};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
    #[cfg(windows)]
    identity: (u64, [u8; 16], i64),
    policy_revision: u64,
}

impl Fingerprint {
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let policy_revision = tagrex_core::model::read_policy_revision();
        if policy_revision == u64::MAX {
            return None;
        }
        #[cfg(windows)]
        {
            use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
                FILE_ID_INFO,
            };
            let file = std::fs::File::open(path).ok()?;
            let metadata = file.metadata().ok()?;
            let mut basic = MaybeUninit::<FILE_BASIC_INFO>::uninit();
            let mut identity = MaybeUninit::<FILE_ID_INFO>::uninit();
            // SAFETY: file owns a live handle for both calls, and the output
            // pointers each refer to correctly sized, aligned writable storage.
            // We assume initialization only after the API reports success.
            unsafe {
                if GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileBasicInfo,
                    basic.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                ) == 0
                    || GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileIdInfo,
                        identity.as_mut_ptr().cast(),
                        std::mem::size_of::<FILE_ID_INFO>() as u32,
                    ) == 0
                {
                    return None;
                }
                let basic = basic.assume_init();
                let identity = identity.assume_init();
                Some(Self {
                    len: metadata.len(),
                    modified: metadata.modified().ok()?,
                    identity: (
                        identity.VolumeSerialNumber,
                        identity.FileId.Identifier,
                        basic.ChangeTime,
                    ),
                    policy_revision,
                })
            }
        }
        #[cfg(not(windows))]
        {
            let m = std::fs::metadata(path).ok()?;
            let mut fingerprint = Self::from_metadata(&m)?;
            // Keep the revision from before the syscall; never label an old
            // observation with a policy installed while it was in progress.
            fingerprint.policy_revision = policy_revision;
            Some(fingerprint)
        }
    }
    #[cfg(not(windows))]
    pub(crate) fn from_metadata(m: &std::fs::Metadata) -> Option<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        let policy_revision = tagrex_core::model::read_policy_revision();
        if policy_revision == u64::MAX {
            return None;
        }
        Some(Self {
            len: m.len(),
            modified: m.modified().ok()?,
            #[cfg(unix)]
            identity: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
            policy_revision,
        })
    }
}

struct Entry {
    stamp: Fingerprint,
    read: Arc<TrackRead>,
    bytes: usize,
    used_at: u64,
    _retained: Lease,
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
    budget: Budget,
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
            budget: Budget::process(),
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
    pub fn read(&mut self, path: &Path) -> Result<Arc<TrackRead>, TagIoError> {
        let mut attempt = 0;
        loop {
            match self.read_using(path, TagEngine::read_with_props) {
                Err(TagIoError::Io(error))
                    if error.kind() == std::io::ErrorKind::Interrupted && attempt < 2 =>
                {
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    fn read_using(
        &mut self,
        path: &Path,
        reader: impl FnOnce(&Path) -> Result<TrackRead, TagIoError>,
    ) -> Result<Arc<TrackRead>, TagIoError> {
        let policy_before = tagrex_core::model::read_policy_revision();
        // Rebase exceptionally long-lived sessions without wrapping usage keys.
        if self.clock == u64::MAX {
            self.clear();
        }
        self.clock += 1;
        let used_at = self.clock;
        let before = Fingerprint::of(path);
        if let Some(entry) = self.entries.get_mut(path) {
            if policy_before != u64::MAX
                && before.as_ref() == Some(&entry.stamp)
                && tagrex_core::model::read_policy_revision() == policy_before
            {
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
        let read = Arc::new(reader(path)?);
        if tagrex_core::model::read_policy_revision() != policy_before {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Metadata settings changed while reading; retry the operation",
            )
            .into());
        }
        // Cache only when the observed fingerprint is unchanged across the read.
        // Filesystems may offer weaker identity/change-time semantics; this
        // does not detect every concurrent or same-size external edit.
        // Executor staleness checks always re-read the file independently.
        if let Some(stamp) = before.filter(|s| {
            policy_before != u64::MAX
                && policy_before == tagrex_core::model::read_policy_revision()
                && Some(s) == Fingerprint::of(path).as_ref()
        }) {
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
                // A stale same-path entry was removed before parsing. Keep all
                // other hot data intact if this fresh result cannot be retained.
                let Ok(retained) = self.budget.try_acquire(Charge {
                    retained: bytes,
                    ..Charge::default()
                }) else {
                    return Ok(read);
                };
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
                        _retained: retained,
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
    fn hits_share_snapshots_and_caller_mutation_cannot_change_the_cached_read() {
        let dir = Fixture::new();
        let path = dir.track("shared.wav", "Original");
        let mut cache = ReadCache::default();
        let mut first = cache.read(&path).unwrap();
        let second = cache.read(&path).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        Arc::make_mut(&mut first)
            .file
            .tags
            .insert(TagField::Title, "Local only".into());
        assert_eq!(second.file.tags[&TagField::Title], "Original");
        assert!(Arc::ptr_eq(&second, &cache.read(&path).unwrap()));
        consistent(&cache);
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
        Arc::make_mut(&mut snapshot)
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
    #[cfg(any(unix, windows))]
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
    fn separate_caches_share_retention_with_transient_jobs_without_losing_reads() {
        use tagrex_core::resources::Limits;
        let dir = Fixture::new();
        let path = dir.track("budget.wav", "Title");
        let budget = Budget::new(Limits {
            retained: 1_000_000,
            transient: 1_000_000,
            total: 1_000_000,
            jobs: 1,
        });
        let mut first = ReadCache {
            budget: budget.clone(),
            ..ReadCache::default()
        };
        let hot = first.read(&path).unwrap();
        let cost = budget.usage().0.retained;
        assert!(cost > 0);
        let work = budget
            .try_acquire(Charge {
                transient: 1_000_000 - cost,
                jobs: 1,
                ..Charge::default()
            })
            .unwrap();
        let mut second = ReadCache {
            budget: budget.clone(),
            ..ReadCache::default()
        };
        assert_eq!(
            second.read(&path).unwrap().file.tags[&TagField::Title],
            "Title"
        );
        assert!(second.entries.is_empty());
        assert!(Arc::ptr_eq(&hot, &first.read(&path).unwrap()));
        drop(work);
        second.read(&path).unwrap();
        assert_eq!(second.entries.len(), 1);
        assert_eq!(budget.usage().0.retained, first.bytes + second.bytes);
        first.clear();
        second.clear();
        // The charge follows cache ownership; external snapshot owners are
        // deliberately outside this cache-accounting contract.
        assert_eq!(budget.usage().0, Charge::default());
        assert_eq!(hot.file.tags[&TagField::Title], "Title");
    }

    #[test]
    fn refused_new_metadata_retention_keeps_existing_hot_snapshot() {
        use tagrex_core::resources::Limits;
        let dir = Fixture::new();
        let old_path = dir.track("old.wav", "Old");
        let new_path = dir.track("new.wav", "A larger title");
        let mut probe = ReadCache::default();
        probe.read(&new_path).unwrap();
        let new_cost = probe.bytes;
        drop(probe);
        let budget = Budget::new(Limits {
            retained: 100_000,
            transient: 100_000,
            total: 100_000,
            jobs: 1,
        });
        let mut cache = ReadCache::with_limits(2, new_cost);
        cache.budget = budget.clone();
        let hot = cache.read(&old_path).unwrap();
        let old_cost = cache.bytes;
        assert!(new_cost > old_cost);
        let pressure = budget
            .try_acquire(Charge {
                retained: 100_000 - old_cost,
                ..Charge::default()
            })
            .unwrap();
        assert_eq!(
            cache.read(&new_path).unwrap().file.tags[&TagField::Title],
            "A larger title"
        );
        assert!(!cache.entries.contains_key(&new_path));
        assert!(cache.entries.contains_key(&old_path));
        assert!(Arc::ptr_eq(&hot, &cache.read(&old_path).unwrap()));
        assert_eq!(budget.usage().0.retained, 100_000);
        drop(pressure);
        cache.read(&new_path).unwrap();
        assert!(cache.entries.contains_key(&new_path));
        consistent(&cache);
        drop(cache);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn unretained_stale_replacement_and_oversized_read_preserve_unrelated_snapshot() {
        use tagrex_core::resources::Limits;
        let dir = Fixture::new();
        let changed = dir.track("changing.wav", "Old");
        let stable = dir.track("stable.wav", "Stable");
        let oversized = dir.track("large.wav", &"X".repeat(16_000));
        let budget = Budget::new(Limits {
            retained: 100_000,
            transient: 100_000,
            total: 100_000,
            jobs: 1,
        });
        let mut cache = ReadCache::with_limits(4, 10_000);
        cache.budget = budget.clone();
        cache.read(&changed).unwrap();
        let hot = cache.read(&stable).unwrap();
        let pressure = budget
            .try_acquire(Charge {
                retained: 100_000 - cache.bytes,
                ..Charge::default()
            })
            .unwrap();
        let mut track = TagEngine::read(&changed).unwrap();
        track.tags.insert(TagField::Title, "A".repeat(4000));
        TagEngine::write(&track).unwrap();
        assert_eq!(
            cache.read(&changed).unwrap().file.tags[&TagField::Title],
            "A".repeat(4000)
        );
        assert!(!cache.entries.contains_key(&changed));
        assert!(Arc::ptr_eq(&hot, &cache.read(&stable).unwrap()));
        cache.read(&oversized).unwrap();
        assert!(!cache.entries.contains_key(&oversized));
        assert!(Arc::ptr_eq(&hot, &cache.read(&stable).unwrap()));
        consistent(&cache);
        drop(pressure);
        drop(cache);
        assert_eq!(budget.usage().0, Charge::default());
        let empty = dir.track("empty.wav", "");
        let mut cache = ReadCache {
            budget: budget.clone(),
            ..ReadCache::default()
        };
        cache.read(&empty).unwrap();
        assert!(
            cache.bytes >= 640,
            "empty values still carry file/index metadata"
        );
        drop(cache);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn policy_change_in_another_session_or_during_read_never_reuses_stale_metadata() {
        // Policies are process-global. Isolate the mutation from unrelated
        // parallel tests instead of relying on test-thread scheduling.
        const CHILD: &str = "TAGREX_READ_POLICY_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "read_cache::tests::policy_change_in_another_session_or_during_read_never_reuses_stale_metadata", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let dir = Fixture::new();
        let path = dir.track("policy.wav", "Title");
        let mut cache = ReadCache::default();
        let first = cache.read(&path).unwrap();
        TagEngine::read(&path).unwrap();
        tagrex_core::model::set_multi_value_separator(" | ");
        let second = cache.read(&path).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        cache.clear();
        let result = cache.read_using(&path, |path| {
            let result = TagEngine::read_with_props(path);
            tagrex_core::model::set_multi_value_separator(" / ");
            result
        });
        assert!(
            matches!(result, Err(TagIoError::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted)
        );
        assert!(cache.entries.is_empty());
        assert!(cache.read(&path).is_ok());
        let app = crate::App::open(&dir.0, &dir.0.join("policy-journal.sqlite")).unwrap();
        let old = app.query_snapshot().unwrap();
        assert!(old.is_current());
        app.apply_settings(&crate::SettingsDto {
            multi_value_separator: " || ".into(),
            ..Default::default()
        });
        assert!(!old.is_current());
        let updated = app.query_snapshot().unwrap();
        assert_ne!(updated.id, old.id);
        assert!(updated.is_current());
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

    #[test]
    #[ignore = "manual resource/cache pressure timing on 2500 disposable WAVs"]
    fn shared_resource_cache_pressure_benchmark() {
        use tagrex_core::resources::Limits;
        let dir = Fixture::new();
        let paths: Vec<_> = (0..2500)
            .map(|i| dir.track(&format!("{i}.wav"), &format!("Title {i}")))
            .collect();
        let budget = Budget::new(Limits {
            retained: 16 * 1024 * 1024,
            transient: 16 * 1024 * 1024,
            total: 16 * 1024 * 1024,
            jobs: 4,
        });
        let mut cache = ReadCache {
            budget: budget.clone(),
            ..ReadCache::default()
        };
        let mut passes = Vec::new();
        for _ in 0..6 {
            let started = std::time::Instant::now();
            for (i, path) in paths.iter().enumerate() {
                assert_eq!(
                    cache.read(path).unwrap().file.tags[&TagField::Title],
                    format!("Title {i}")
                );
            }
            passes.push(started.elapsed());
            consistent(&cache);
        }
        assert_eq!(cache.entries.len(), paths.len());
        let retained = budget.usage().0.retained;
        let work = budget
            .try_acquire(Charge {
                transient: 16 * 1024 * 1024 - retained,
                jobs: 1,
                ..Charge::default()
            })
            .unwrap();
        let mut other = ReadCache {
            budget: budget.clone(),
            ..ReadCache::default()
        };
        let started = std::time::Instant::now();
        for path in paths.iter().rev() {
            other.read(path).unwrap();
        }
        let pressure = started.elapsed();
        assert!(other.entries.is_empty());
        assert_eq!(budget.usage().0.retained, retained);
        drop(work);
        cache.clear();
        drop(other);
        assert_eq!(budget.usage().0, Charge::default());
        println!("resource-cache count=2500 first_ms={:.3} warm_ms={:?} pressure_ms={:.3} retained_bytes={retained}",
            passes[0].as_secs_f64() * 1000.0,
            passes[1..].iter().map(|p| p.as_secs_f64() * 1000.0).collect::<Vec<_>>(), pressure.as_secs_f64() * 1000.0);
    }
}
