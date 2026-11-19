//! Recovery-store initialization uses the caller's held profile authority.
#![cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    os::unix::fs::{symlink, MetadataExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
use tagrex_core::{
    media_mutation::{BoundMedia, Error, Limits, Phase, Record, Store},
    resources::{AdmissionError, Budget, Charge, Limits as ResourceLimits},
};
static CASE: Mutex<()> = Mutex::new(());
struct Fixture(PathBuf, MutexGuard<'static, ()>);
impl Fixture {
    fn new() -> Self {
        let guard = CASE.lock().unwrap_or_else(|p| p.into_inner());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-media-parent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("profile")).unwrap();
        Self(path, guard)
    }
    fn profile(&self) -> PathBuf {
        self.0.join("profile")
    }
    fn path(&self) -> PathBuf {
        self.profile().join("store")
    }
    fn held(&self) -> File {
        File::open(self.profile()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = &self.1;
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn limits() -> Limits {
    Limits {
        store_bytes: 32 << 20,
        entries: 128,
        media_bytes: 1 << 20,
    }
}
fn budget() -> Budget {
    Budget::new(ResourceLimits {
        retained: 16 << 20,
        transient: 16 << 20,
        total: 32 << 20,
        jobs: 8,
    })
}
fn sink(_: &Record) -> io::Result<()> {
    Ok(())
}
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    mode: u32,
    inode: u64,
    links: u64,
    ctime: (i64, i64),
    bytes: Vec<u8>,
    protection: Option<Vec<u8>>,
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let metadata = fs::symlink_metadata(at).unwrap();
        let bytes = if metadata.is_symlink() {
            fs::read_link(at)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            fs::read(at).unwrap()
        } else {
            Vec::new()
        };
        let protection = if metadata.is_symlink() {
            None
        } else {
            let file = File::open(at).unwrap();
            let mut bytes = [0u8; 128];
            match rustix::fs::fgetxattr(&file, "user.tagrex.media_recovery_v1", &mut bytes) {
                Ok(n) => Some(bytes[..n].to_vec()),
                Err(rustix::io::Errno::NODATA) => None,
                Err(e) => panic!("xattr: {e}"),
            }
        };
        out.insert(
            at.strip_prefix(root).unwrap().to_path_buf(),
            Entry {
                mode: metadata.mode(),
                inode: metadata.ino(),
                links: metadata.nlink(),
                ctime: (metadata.ctime(), metadata.ctime_nsec()),
                bytes,
                protection,
            },
        );
        if metadata.is_dir() {
            for child in fs::read_dir(at).unwrap() {
                visit(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn prepare(store: &Store, path: &Path, bytes: &[u8]) -> tagrex_core::media_mutation::Prepared {
    store
        .prepare_file(
            BoundMedia::open(path).unwrap(),
            "flac",
            bytes.len() as u64,
            &mut sink,
            &mut (),
            |file, _| {
                file.seek(SeekFrom::Start(0))?;
                file.write_all(bytes)?;
                file.set_len(bytes.len() as u64)
            },
        )
        .unwrap()
}
#[test]
fn held_parent_prepares_publishes_reopens_and_retires_the_same_media_inode() {
    let f = Fixture::new();
    let resources = budget();
    let baseline = resources.usage().0;
    let original = f.0.join("original.flac");
    fs::write(&original, b"original audio").unwrap();
    let inode = fs::metadata(&original).unwrap().ino();
    let store = Store::open_in(f.held(), &f.path(), limits(), resources.clone()).unwrap();
    let candidate = prepare(&store, &original, b"edited audio");
    let id = candidate.record().id().to_owned();
    assert_eq!(fs::read(&original).unwrap(), b"original audio");
    let published = candidate.publish(&mut sink, &mut ()).unwrap();
    assert_eq!(published.record().phase(), Phase::Published);
    drop(published);
    drop(store);
    assert_eq!(resources.usage().0, baseline);
    let store = Store::open_in(f.held(), &f.path(), limits(), resources.clone()).unwrap();
    assert_eq!(store.records().unwrap()[0].id(), id);
    store
        .resume_published(&id, BoundMedia::open(&original).unwrap())
        .unwrap()
        .retire(|_| Ok(()), &mut sink, &mut ())
        .unwrap();
    assert!(store.records().unwrap().is_empty());
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(fs::read(&original).unwrap(), b"edited audio");
    assert_eq!(fs::metadata(original).unwrap().ino(), inode);
    drop(store);
    let legacy = Store::open(&f.path(), limits(), resources.clone()).unwrap();
    assert!(legacy.records().unwrap().is_empty());
    drop(legacy);
    assert_eq!(resources.usage().0, baseline);
}
#[test]
fn replaced_parent_is_refused_before_either_namespace_is_touched() {
    for populated in [false, true] {
        let f = Fixture::new();
        let held = f.held();
        let displaced = f.0.join("displaced");
        fs::rename(f.profile(), &displaced).unwrap();
        fs::create_dir(f.profile()).unwrap();
        if populated {
            let foreign = Store::open(&f.path(), limits(), budget()).unwrap();
            drop(foreign);
        }
        fs::write(f.profile().join("sentinel"), b"foreign profile").unwrap();
        let before = snapshot(&f.0);
        let resources = budget();
        assert!(matches!(
            Store::open_in(held, &f.path(), limits(), resources.clone()),
            Err(Error::Changed)
        ));
        assert_eq!(snapshot(&f.0), before);
        assert_eq!(resources.usage().0, Charge::default());
        assert!(!displaced.join("store").exists());
    }
}
#[test]
fn wrong_kind_parent_and_invalid_logical_paths_have_no_effects() {
    let f = Fixture::new();
    let regular = f.0.join("ordinary");
    fs::write(&regular, b"not a directory").unwrap();
    let other = f.0.join("other");
    fs::create_dir(&other).unwrap();
    let before = snapshot(&f.0);
    for parent in [File::open(&regular).unwrap(), File::open(&other).unwrap()] {
        assert!(Store::open_in(parent, &f.path(), limits(), budget()).is_err());
        assert_eq!(snapshot(&f.0), before);
    }
    for path in [
        PathBuf::from("relative/store"),
        f.profile().join("../escape"),
        PathBuf::from("/"),
        f.profile().join("bad\0name"),
    ] {
        assert!(Store::open_in(f.held(), &path, limits(), budget()).is_err());
        assert_eq!(snapshot(&f.0), before);
    }
}
#[test]
fn parent_or_store_links_cannot_redirect_initialization() {
    for parent_link in [false, true] {
        let f = Fixture::new();
        let other = f.0.join("other");
        fs::create_dir(&other).unwrap();
        if parent_link {
            let held = f.held();
            fs::rename(f.profile(), f.0.join("original-profile")).unwrap();
            symlink(&other, f.profile()).unwrap();
            let before = snapshot(&f.0);
            assert!(Store::open_in(held, &f.path(), limits(), budget()).is_err());
            assert_eq!(snapshot(&f.0), before);
        } else {
            symlink(&other, f.path()).unwrap();
            let before = snapshot(&f.0);
            assert!(Store::open_in(f.held(), &f.path(), limits(), budget()).is_err());
            assert_eq!(snapshot(&f.0), before);
        }
    }
}
#[test]
fn a_replaced_child_cannot_be_adopted_as_an_open_stores_current_namespace() {
    let f = Fixture::new();
    let store = Store::open_in(f.held(), &f.path(), limits(), budget()).unwrap();
    fs::rename(f.path(), f.profile().join("displaced-store")).unwrap();
    let foreign = Store::open(&f.path(), limits(), budget()).unwrap();
    drop(foreign);
    let before = snapshot(&f.0);
    assert!(matches!(store.records(), Err(Error::Changed)));
    assert_eq!(snapshot(&f.0), before);
}
#[test]
fn admission_lock_and_repeated_refusals_release_resources_and_descriptors() {
    let f = Fixture::new();
    let resources = budget();
    let baseline = resources.usage().0;
    let hold = resources
        .try_acquire(Charge {
            retained: 16 << 20,
            ..Default::default()
        })
        .unwrap();
    let before = snapshot(&f.0);
    assert!(matches!(
        Store::open_in(f.held(), &f.path(), limits(), resources.clone()),
        Err(Error::Resource(AdmissionError::Busy))
    ));
    assert_eq!(snapshot(&f.0), before);
    drop(hold);
    assert_eq!(resources.usage().0, baseline);
    let store = Store::open_in(f.held(), &f.path(), limits(), resources.clone()).unwrap();
    let active = resources.usage().0;
    let fds = || fs::read_dir("/proc/self/fd").unwrap().count();
    let count = fds();
    let before = snapshot(&f.0);
    for _ in 0..96 {
        assert!(matches!(
            Store::open_in(f.held(), &f.path(), limits(), resources.clone()),
            Err(Error::Capacity)
        ));
        assert_eq!(resources.usage().0, active);
    }
    assert_eq!(fds(), count);
    assert_eq!(snapshot(&f.0), before);
    drop(store);
    assert_eq!(resources.usage().0, baseline);
    let reopened = Store::open_in(f.held(), &f.path(), limits(), resources.clone()).unwrap();
    drop(reopened);
    assert_eq!(resources.usage().0, baseline);
}

#[test]
fn trailing_dot_cannot_change_which_parent_the_handle_authorizes() {
    let f = Fixture::new();
    let before = snapshot(&f.0);
    let logical = f.profile().join(".");
    let result = Store::open_in(f.held(), &logical, limits(), budget());
    assert!(result.is_err(), "trailing dot accepted");
    assert_eq!(
        snapshot(&f.0),
        before,
        "trailing-dot error had earlier effects"
    );
    assert!(!f.profile().join("profile").exists());
}

#[test]
fn path_only_directory_handles_refuse_before_creating_an_unsyncable_store() {
    let f = Fixture::new();
    let parent = File::from(
        rustix::fs::open(
            f.profile(),
            rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap(),
    );
    let before = snapshot(&f.0);
    assert!(Store::open_in(parent, &f.path(), limits(), budget()).is_err());
    assert_eq!(
        snapshot(&f.0),
        before,
        "unsupported parent capability had earlier namespace effects"
    );
}
