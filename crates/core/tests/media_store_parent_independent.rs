//! Independent held-parent authority and recovery-opening probes.
#![cfg(target_os = "linux")]
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::{self, Seek, SeekFrom, Write},
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
use tagrex_core::{
    media_mutation::{BoundMedia, Current, Error, Event, Limits, Observer, Phase, Record, Store},
    resources::{Budget, Charge, Limits as ResourceLimits},
};
static CASE: Mutex<()> = Mutex::new(());
struct Fixture(PathBuf, MutexGuard<'static, ()>);
impl Fixture {
    fn new() -> Self {
        let guard = CASE.lock().unwrap_or_else(|p| p.into_inner());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-parent-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("profile")).unwrap();
        fs::write(path.join("original.flac"), b"untouched encoded audio").unwrap();
        Self(path, guard)
    }
    fn profile(&self) -> PathBuf {
        self.0.join("profile")
    }
    fn store(&self) -> PathBuf {
        self.profile().join("store")
    }
    fn media(&self) -> PathBuf {
        self.0.join("original.flac")
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
        entries: 16,
        store_bytes: 8 << 20,
        media_bytes: 1 << 20,
    }
}
fn budget() -> Budget {
    Budget::new(ResourceLimits {
        retained: 4 << 20,
        transient: 4 << 20,
        total: 8 << 20,
        jobs: 4,
    })
}
fn sink(_: &Record) -> io::Result<()> {
    Ok(())
}
fn edit(file: &mut File, _: &mut fn(&Record) -> io::Result<()>) -> io::Result<()> {
    let bytes = b"changed encoded audio and metadata";
    file.seek(SeekFrom::Start(0))?;
    file.write_all(bytes)?;
    file.set_len(bytes.len() as u64)
}
fn prepared(store: &Store, path: &Path) -> tagrex_core::media_mutation::Prepared {
    let mut ack: fn(&Record) -> io::Result<()> = sink;
    store
        .prepare_file(
            BoundMedia::open(path).unwrap(),
            "flac",
            128,
            &mut ack,
            &mut (),
            edit,
        )
        .unwrap()
}
#[derive(Debug, PartialEq, Eq)]
struct Item {
    inode: u64,
    mode: u32,
    links: u64,
    ctime: (i64, i64),
    bytes: Vec<u8>,
    marker: Option<Vec<u8>>,
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Item> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Item>) {
        let m = fs::symlink_metadata(path).unwrap();
        let bytes = if m.is_file() {
            fs::read(path).unwrap()
        } else if m.is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else {
            Vec::new()
        };
        let marker = if m.is_symlink() {
            None
        } else {
            let file = File::open(path).unwrap();
            let mut bytes = [0; 128];
            match rustix::fs::fgetxattr(&file, "user.tagrex.media_recovery_v1", &mut bytes) {
                Ok(n) => Some(bytes[..n].to_vec()),
                Err(rustix::io::Errno::NODATA) => None,
                Err(e) => panic!("xattr: {e}"),
            }
        };
        out.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            Item {
                inode: m.ino(),
                mode: m.mode(),
                links: m.nlink(),
                ctime: (m.ctime(), m.ctime_nsec()),
                bytes,
                marker,
            },
        );
        if m.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn run_child(root: &Path, mode: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "held_parent_owned_child", "--nocapture"])
        .env("TAGREX_PARENT_REVIEW_ROOT", root)
        .env("TAGREX_PARENT_REVIEW_MODE", mode)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}
#[test]
fn held_parent_owned_child() {
    let Some(root) = std::env::var_os("TAGREX_PARENT_REVIEW_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let profile = root.join("profile");
    let mode = std::env::var("TAGREX_PARENT_REVIEW_MODE").unwrap();
    let resources = budget();
    let opened = Store::open_in(
        File::open(&profile).unwrap(),
        &profile.join("store"),
        limits(),
        resources.clone(),
    );
    if mode.starts_with("race-") {
        assert!(
            matches!(opened, Err(Error::Changed)),
            "changed initialization accepted"
        );
        assert_eq!(resources.usage().0, Charge::default());
        return;
    }
    let store = opened.unwrap();
    let pending = prepared(&store, &root.join("original.flac"));
    fs::write(root.join("operation-id"), pending.record().id()).unwrap();
    if mode == "published" {
        let _published = pending.publish(&mut sink, &mut ()).unwrap();
        std::process::exit(0);
    }
    assert_eq!(mode, "prepared");
    std::process::exit(0);
}
#[test]
fn held_parent_reopens_prepared_and_published_evidence_after_real_process_exit() {
    for mode in ["prepared", "published"] {
        let f = Fixture::new();
        run_child(&f.0, mode);
        let held = f.held();
        let before = snapshot(&f.0);
        let id = fs::read_to_string(f.0.join("operation-id")).unwrap();
        let store = Store::open_in(held, &f.store(), limits(), budget()).unwrap();
        assert_eq!(store.records().unwrap()[0].id(), id);
        assert_eq!(
            store.records().unwrap()[0].phase(),
            if mode == "prepared" {
                Phase::Prepared
            } else {
                Phase::Published
            }
        );
        assert_eq!(
            store
                .inspect(&id, &mut BoundMedia::open(&f.media()).unwrap())
                .unwrap(),
            if mode == "prepared" {
                Current::Preimage
            } else {
                Current::Candidate
            }
        );
        assert_eq!(snapshot(&f.0), before);
        if mode == "published" {
            store
                .resume_published(&id, BoundMedia::open(&f.media()).unwrap())
                .unwrap()
                .retire(|_| Ok(()), &mut sink, &mut ())
                .unwrap();
            assert_eq!(store.usage(), (0, 0));
        }
    }
}
#[test]
fn held_parent_reopens_and_finishes_partially_retired_evidence() {
    struct Stop;
    impl Observer for Stop {
        fn event(&mut self, event: Event) -> io::Result<()> {
            if matches!(event, Event::ArtifactRemoved(_)) {
                Err(io::Error::other("owned retirement interruption"))
            } else {
                Ok(())
            }
        }
    }
    let f = Fixture::new();
    let resources = budget();
    let baseline = resources.usage().0;
    let store = Store::open_in(f.held(), &f.store(), limits(), resources.clone()).unwrap();
    let original_inode = fs::metadata(f.media()).unwrap().ino();
    let published = prepared(&store, &f.media())
        .publish(&mut sink, &mut ())
        .unwrap();
    let id = published.record().id().to_owned();
    assert!(published.retire(|_| Ok(()), &mut sink, &mut Stop).is_err());
    drop(store);
    assert_eq!(resources.usage().0, baseline);
    let store = Store::open_in(f.held(), &f.store(), limits(), resources.clone()).unwrap();
    assert_eq!(store.records().unwrap()[0].phase(), Phase::Retired);
    store
        .finish_retirement(
            &id,
            &mut BoundMedia::open(&f.media()).unwrap(),
            |_| Ok(()),
            &mut (),
        )
        .unwrap();
    assert!(store.records().unwrap().is_empty());
    assert_eq!(store.usage(), (0, 0));
    assert_eq!(
        fs::read(f.media()).unwrap(),
        b"changed encoded audio and metadata"
    );
    assert_eq!(fs::metadata(f.media()).unwrap().ino(), original_inode);
    drop(store);
    assert_eq!(resources.usage().0, baseline);
}
#[test]
fn rejected_replaced_parent_does_not_open_or_change_foreign_recovery_history() {
    let f = Fixture::new();
    run_child(&f.0, "published");
    let stale = f.held();
    fs::rename(f.profile(), f.0.join("saved-profile")).unwrap();
    fs::create_dir(f.profile()).unwrap();
    let foreign = Store::open(&f.store(), limits(), budget()).unwrap();
    let foreign_media = f.0.join("foreign.flac");
    fs::write(&foreign_media, b"foreign encoded audio").unwrap();
    let foreign_pending = prepared(&foreign, &foreign_media)
        .publish(&mut sink, &mut ())
        .unwrap();
    let foreign_id = foreign_pending.record().id().to_owned();
    drop(foreign_pending);
    drop(foreign);
    let before = snapshot(&f.0);
    let resources = budget();
    assert!(matches!(
        Store::open_in(stale, &f.store(), limits(), resources.clone()),
        Err(Error::Changed)
    ));
    assert_eq!(resources.usage().0, Charge::default());
    assert_eq!(snapshot(&f.0), before);
    let foreign = Store::open_in(f.held(), &f.store(), limits(), resources).unwrap();
    assert_eq!(foreign.records().unwrap()[0].id(), foreign_id);
}
#[test]
fn invalid_existing_lock_has_no_history_effect_and_all_failed_owners_release() {
    let f = Fixture::new();
    run_child(&f.0, "published");
    let external = f.0.join("foreign-hardlink");
    fs::hard_link(f.store().join("lock"), &external).unwrap();
    let before = snapshot(&f.0);
    let resources = budget();
    let fds = fs::read_dir("/proc/self/fd").unwrap().count();
    for _ in 0..48 {
        assert!(Store::open_in(f.held(), &f.store(), limits(), resources.clone()).is_err());
        assert_eq!(resources.usage().0, Charge::default());
    }
    assert_eq!(fs::read_dir("/proc/self/fd").unwrap().count(), fds);
    assert_eq!(snapshot(&f.0), before);
    fs::remove_file(external).unwrap();
    let store = Store::open_in(f.held(), &f.store(), limits(), resources).unwrap();
    assert_eq!(store.records().unwrap().len(), 1);
}
#[test]
fn non_utf8_and_unicode_store_names_keep_the_same_held_parent_authority() {
    let f = Fixture::new();
    let resources = budget();
    for name in [
        OsString::from("recovery-尾-🎵"),
        OsString::from_vec(b"recovery-\xff".to_vec()),
    ] {
        let path = f.profile().join(name);
        let store = Store::open_in(f.held(), &path, limits(), resources.clone()).unwrap();
        assert!(store.records().unwrap().is_empty());
        drop(store);
        let reopened = Store::open_in(f.held(), &path, limits(), resources.clone()).unwrap();
        assert!(reopened.records().unwrap().is_empty());
        drop(reopened);
        assert_eq!(resources.usage().0, Charge::default());
    }
    assert_eq!(fs::read(f.media()).unwrap(), b"untouched encoded audio");
}
#[cfg(target_arch = "x86_64")]
#[test]
fn actual_parent_substitution_at_initialization_syscalls_never_adopts_the_replacement() {
    const TRACE: &str = r#"
import ctypes, os, signal, sys
libc=ctypes.CDLL(None, use_errno=True)
libc.ptrace.restype=ctypes.c_long
class Registers(ctypes.Structure):
    _fields_=[(name,ctypes.c_ulonglong) for name in 'r15 r14 r13 r12 rbp rbx r11 r10 r9 r8 rax rcx rdx rsi rdi orig_rax rip cs eflags rsp ss fs_base gs_base ds es fs gs'.split()]
def ptrace(request,pid,address=0,data=0):
    ctypes.set_errno(0)
    value=libc.ptrace(request,pid,ctypes.c_void_p(address),ctypes.c_void_p(data))
    if value==-1 and ctypes.get_errno(): raise OSError(ctypes.get_errno(),os.strerror(ctypes.get_errno()))
    return value
def string(pid,address):
    result=b''
    for at in range(0,4096,8):
        part=ptrace(2,pid,address+at).to_bytes(8,sys.byteorder,signed=True)
        result+=part
        if b'\0' in part: return os.fsdecode(result.split(b'\0',1)[0])
    raise RuntimeError('unbounded trace pathname')
root,exe,mode=sys.argv[1:]
profile=os.path.join(root,'profile'); child=None
signal.signal(signal.SIGALRM,lambda *_: (_ for _ in ()).throw(TimeoutError('owned ptrace child deadline')))
signal.alarm(20)
try:
    child=os.fork()
    if child==0:
        ptrace(0,0)
        os.kill(os.getpid(),signal.SIGSTOP)
        os.environ['TAGREX_PARENT_REVIEW_ROOT']=root
        os.environ['TAGREX_PARENT_REVIEW_MODE']='race-'+mode
        os.execv(exe,[exe,'--exact','held_parent_owned_child','--nocapture'])
    _,status=os.waitpid(child,0)
    assert os.WIFSTOPPED(status)
    ptrace(0x4200,child,0,1|0x100000|8)
    moved=False; parent_opens=0
    ptrace(24,child)
    while True:
        stopped,status=os.waitpid(-1,0x40000000)
        if os.WIFEXITED(status):
            if stopped!=child: continue
            assert moved,'target syscall was never observed'
            assert os.WEXITSTATUS(status)==0,('fixture child failed',status)
            child=None; break
        if os.WIFSIGNALED(status): raise RuntimeError(('fixture child signal',os.WTERMSIG(status)))
        stop=os.WSTOPSIG(status)
        if stop==133:
            regs=Registers(); ptrace(12,stopped,0,ctypes.addressof(regs))
            if regs.rax==2**64-38:
                at_mkdir=regs.orig_rax==258 and string(stopped,regs.rsi)=='store'
                path_address=regs.rdi if regs.orig_rax==2 else regs.rsi
                flags=regs.rsi if regs.orig_rax==2 else regs.rdx
                opening_parent=regs.orig_rax in (2,257) and flags & 0x10000 and string(stopped,path_address)==profile
                if opening_parent: parent_opens+=1
                at_final=opening_parent and parent_opens==2
                if not moved and ((mode=='mkdir' and at_mkdir) or (mode=='final' and at_final)):
                    os.rename(profile,os.path.join(root,'held-profile'))
                    os.mkdir(profile)
                    with open(os.path.join(profile,'sentinel'),'wb') as f: f.write(b'foreign profile must remain untouched')
                    stat=os.stat(profile)
                    foreign_witness=(stat.st_ino,stat.st_mode,stat.st_ctime_ns,os.listxattr(profile))
                    moved=True
        ptrace(24,stopped)
    stat=os.stat(profile)
    assert (stat.st_ino,stat.st_mode,stat.st_ctime_ns,os.listxattr(profile))==foreign_witness
    assert sorted(os.listdir(profile))==['sentinel']
    assert open(os.path.join(profile,'sentinel'),'rb').read()==b'foreign profile must remain untouched'
    assert os.path.isdir(os.path.join(root,'held-profile','store'))
    print('PARENT_INIT_TRACE',mode,'replacement untouched; initialized evidence remains held-parent anchored',flush=True)
finally:
    if child:
        try: os.kill(child,signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(child,0)
        except ChildProcessError: pass
    signal.alarm(0)
"#;
    for mode in ["mkdir", "final"] {
        let f = Fixture::new();
        let python = std::env::var_os("TAGREX_TEST_PYTHON").unwrap_or_else(|| "python3".into());
        let output = Command::new(python)
            .args(["-c", TRACE])
            .arg(&f.0)
            .arg(std::env::current_exe().unwrap())
            .arg(mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("PARENT_INIT_TRACE"));
        assert_eq!(fs::read(f.media()).unwrap(), b"untouched encoded audio");
        assert_eq!(
            fs::read(f.profile().join("sentinel")).unwrap(),
            b"foreign profile must remain untouched"
        );
        assert!(!f.profile().join("store").exists());
    }
}
