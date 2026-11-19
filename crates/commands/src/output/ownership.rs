//! In-process ownership for confined output publication. Every potentially
//! destructive public-name operation first captures into a private directory.
//! Restart recovery is a separate protocol; these held proofs are process-local.
use super::Directory;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
use tagrex_core::resources::{AdmissionError, Budget, Charge, Lease};

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Staged,
    CandidateCaptured,
    BeforePublish,
    BackupCaptured,
    BeforeCommit,
    BeforeCleanup,
    BeforePrivateRemove,
    BeforeSync,
    BeforeRestore,
    RestoreCaptured,
}
#[cfg(test)]
type Observer = Box<dyn FnMut(Event, &Path) -> io::Result<()>>;
#[cfg(test)]
thread_local! { static OBSERVER:std::cell::RefCell<Option<Observer>>=const{std::cell::RefCell::new(None)}; }
#[cfg(test)]
pub(super) fn set_observer(observer: Option<Observer>) {
    OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}
#[cfg(test)]
fn observe(event: Event, path: &Path) -> io::Result<()> {
    let mut observer = OBSERVER.with(|slot| slot.borrow_mut().take());
    let result = observer
        .as_mut()
        .map_or(Ok(()), |callback| callback(event, path));
    OBSERVER.with(|slot| *slot.borrow_mut() = observer);
    result
}
macro_rules! observe {
    ($event:expr,$path:expr) => {{
        #[cfg(test)]
        {
            observe($event, &$path)?;
        }
    }};
}

const MAX_OUTPUTS: usize = 128;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_OLD_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_TOTAL: usize = 256 * 1024;
const FD_LIMIT: usize = 768;
static DESCRIPTORS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(super) fn test_descriptors() -> usize {
    DESCRIPTORS.load(Ordering::Acquire)
}

fn nonce(prefix: &str) -> io::Result<OsString> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| io::Error::other(format!("output nonce unavailable: {error}")))?;
    let mut name = String::with_capacity(prefix.len() + 32);
    name.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        name.push(HEX[usize::from(byte >> 4)] as char);
        name.push(HEX[usize::from(byte & 15)] as char);
    }
    Ok(name.into())
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to String cannot fail");
    }
    text
}
struct RecoveryPaths<'a>(&'a [PathBuf]);
impl std::fmt::Display for RecoveryPaths<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (number, path) in self.0.iter().take(8).enumerate() {
            if number > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{}", path.display())?;
        }
        if self.0.len() > 8 {
            write!(
                formatter,
                " ({} further paths available via recovery_paths)",
                self.0.len() - 8
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Resource(#[from] AdmissionError),
    #[error("Output changed since its publication baseline was read: {0}")]
    Changed(PathBuf),
    #[error(
        "Output recovery is required ({phase}); retained artifacts: {}; cause: {cause}",
        RecoveryPaths(paths)
    )]
    RecoveryRequired {
        phase: &'static str,
        paths: Vec<PathBuf>,
        cause: String,
        _retained: Option<Lease>,
    },
}
impl OutputError {
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::Io(error) => error.kind(),
            Self::Resource(AdmissionError::Busy) => io::ErrorKind::WouldBlock,
            Self::Resource(AdmissionError::TooLarge) => io::ErrorKind::InvalidInput,
            Self::Changed(_) => io::ErrorKind::Interrupted,
            Self::RecoveryRequired { .. } => io::ErrorKind::Other,
        }
    }
    pub fn recovery_paths(&self) -> &[PathBuf] {
        match self {
            Self::RecoveryRequired { paths, .. } => paths,
            _ => &[],
        }
    }
}
#[cfg(unix)]
impl From<rustix::io::Errno> for OutputError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}
struct Descriptors(usize);
impl Descriptors {
    fn acquire(count: usize) -> Result<Self, OutputError> {
        if count > FD_LIMIT {
            return Err(AdmissionError::TooLarge.into());
        }
        let mut used = DESCRIPTORS.load(Ordering::Acquire);
        loop {
            let next = used
                .checked_add(count)
                .filter(|next| *next <= FD_LIMIT)
                .ok_or(AdmissionError::Busy)?;
            match DESCRIPTORS.compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(current) => used = current,
            }
        }
        Ok(Self(count))
    }
}
impl Drop for Descriptors {
    fn drop(&mut self) {
        DESCRIPTORS.fetch_sub(self.0, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Image {
    len: u64,
    hash: [u8; 32],
}
impl Image {
    fn bytes(bytes: &[u8]) -> Self {
        Self {
            len: bytes.len() as u64,
            hash: Sha256::digest(bytes).into(),
        }
    }
}
#[cfg(windows)]
fn windows_witness(file: &File) -> io::Result<(u64, [u8; 16], i64, u32)> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO, FILE_ID_INFO,
    };
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::uninit();
    let mut identity = MaybeUninit::<FILE_ID_INFO>::uninit();
    // SAFETY: the live File handle is borrowed, each output points to aligned
    // storage of its exact SDK size, and initialization is used only on success.
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
            return Err(io::Error::last_os_error());
        }
        let basic = basic.assume_init();
        let identity = identity.assume_init();
        Ok((
            identity.VolumeSerialNumber,
            identity.FileId.Identifier,
            basic.ChangeTime,
            basic.FileAttributes,
        ))
    }
}
fn same_handles(left: &same_file::Handle, right: &same_file::Handle) -> io::Result<bool> {
    if left != right {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        let a = windows_witness(left.as_file())?;
        let b = windows_witness(right.as_file())?;
        if (a.0, a.1) != (b.0, b.1) {
            return Ok(false);
        }
    }
    Ok(true)
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    change: (i64, i64),
    #[cfg(windows)]
    windows: (u64, [u8; 16], i64, u32),
}
impl Stamp {
    fn of(file: &File, m: &Metadata) -> io::Result<Self> {
        #[cfg(not(windows))]
        let _ = file;
        Ok(Self {
            len: m.len(),
            modified: m.modified().ok(),
            #[cfg(unix)]
            change: {
                use std::os::unix::fs::MetadataExt;
                (m.ctime(), m.ctime_nsec())
            },
            #[cfg(windows)]
            windows: windows_witness(file)?,
        })
    }
}
fn image(file: &File) -> io::Result<(Image, Stamp)> {
    let before = file.metadata()?;
    if !before.is_file() || before.len() > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output proof requires a bounded regular file",
        ));
    }
    let stamp = Stamp::of(file, &before)?;
    let mut read = file;
    read.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = read.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count = count
            .checked_add(n as u64)
            .filter(|n| *n <= MAX_FILE_BYTES)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output exceeds proof byte limit",
                )
            })?;
        hash.update(&buffer[..n]);
    }
    if stamp != Stamp::of(file, &file.metadata()?)? || count != stamp.len {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "output changed while reading ownership proof",
        ));
    }
    Ok((
        Image {
            len: count,
            hash: hash.finalize().into(),
        },
        stamp,
    ))
}
struct Proof {
    file: same_file::Handle,
    image: Image,
    stamp: Stamp,
}
impl Proof {
    fn read(file: File) -> io::Result<Self> {
        let (image, stamp) = image(&file)?;
        Ok(Self {
            file: same_file::Handle::from_file(file)?,
            image,
            stamp,
        })
    }
    fn matches(&self, file: File, stamp: bool) -> io::Result<bool> {
        let actual = same_file::Handle::from_file(file)?;
        if !same_handles(&self.file, &actual)? {
            return Ok(false);
        }
        let (data, witness) = image(actual.as_file())?;
        Ok(data == self.image && (!stamp || witness == self.stamp))
    }
    fn at(&self, dir: &Directory, name: &OsStr, stamp: bool) -> io::Result<bool> {
        match dir.open_regular(name, false) {
            Ok(file) => self.matches(file, stamp),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn refresh(&mut self) -> io::Result<()> {
        let (data, stamp) = image(self.file.as_file())?;
        if data != self.image {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "owned output bytes changed",
            ));
        }
        self.stamp = stamp;
        Ok(())
    }
}

#[cfg(unix)]
pub(super) fn native_result(result: rustix::io::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if [
                rustix::io::Errno::NOSYS,
                rustix::io::Errno::INVAL,
                rustix::io::Errno::NOTSUP,
                rustix::io::Errno::OPNOTSUPP,
            ]
            .contains(&error) =>
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "atomic no-replace capture is unsupported",
            ))
        }
        Err(error) => Err(error.into()),
    }
}

impl Directory {
    fn under_root(root: &Arc<Directory>, parent: &Path) -> io::Result<Arc<Self>> {
        // Reopening only relative to the held root would compare that root to
        // itself after its logical name was moved/replaced. Bind the current
        // public root name to the original held object before walking children.
        #[cfg(unix)]
        {
            use rustix::fs::{open, Mode, OFlags};
            let current = File::from(open(
                &root.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            if !same_handles(
                &same_file::Handle::from_file(root.directory_file()?)?,
                &same_file::Handle::from_file(current)?,
            )? {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "output root directory identity changed",
                ));
            }
        }
        #[cfg(windows)]
        let _root_guard = root.operation_guard()?;
        let canonical = parent.canonicalize()?;
        let relative = canonical.strip_prefix(&root.path).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output parent left the pinned root",
            )
        })?;
        if canonical.as_os_str().len() > MAX_PATH_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "canonical output parent is too long",
            ));
        }
        if relative.as_os_str().is_empty() {
            return Ok(root.clone());
        }
        #[cfg(unix)]
        {
            use rustix::fs::{openat, Mode, OFlags};
            let mut fd = root.fd.try_clone()?;
            for part in relative.components() {
                let std::path::Component::Normal(name) = part else {
                    return Err(io::ErrorKind::InvalidInput.into());
                };
                fd = openat(
                    &fd,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?;
            }
            Ok(Arc::new(Self {
                fd,
                path: canonical,
            }))
        }
        #[cfg(not(unix))]
        {
            Ok(Arc::new(Self::open(&root.path, &canonical)?))
        }
    }
    fn logical_current(&self, root: &Arc<Directory>) -> io::Result<bool> {
        let current = Self::under_root(root, &self.path)?;
        same_handles(
            &same_file::Handle::from_file(self.directory_file()?)?,
            &same_file::Handle::from_file(current.directory_file()?)?,
        )
    }
    #[cfg(windows)]
    fn operation_guard(&self) -> io::Result<same_file::Handle> {
        use std::os::windows::fs::OpenOptionsExt;
        // Traverse/read attributes participates in delete-sharing checks without
        // requiring directory listing. Hold this checked guard through the
        // subsequent path-based Windows syscall.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(0x20 | 0x80)
            .share_mode(0x1 | 0x2)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(&self.path)?;
        let guard = same_file::Handle::from_file(file)?;
        let expected = same_file::Handle::from_file(self.directory_file()?)?;
        if !same_handles(&guard, &expected)? {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "output directory identity changed",
            ));
        }
        Ok(guard)
    }
    fn open_regular(&self, name: &OsStr, create: bool) -> io::Result<File> {
        #[cfg(windows)]
        let _guard = self.operation_guard()?;
        #[cfg(unix)]
        let file = {
            use rustix::fs::{openat, Mode, OFlags};
            let mut flags = if create {
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL
            } else {
                OFlags::RDONLY
            };
            flags |= OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            File::from(openat(
                &self.fd,
                name,
                flags,
                Mode::from_bits_truncate(0o600),
            )?)
        };
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .write(create)
                .create_new(create)
                .share_mode(0x1 | 0x2 | 0x4)
                .custom_flags(0x0020_0000)
                .open(self.path.join(name))?
        };
        #[cfg(not(any(unix, windows)))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owned outputs are unsupported",
        ));
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "output is not a regular file",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output is a reparse point",
                ));
            }
        }
        Ok(file)
    }
    fn child(&self, name: &OsStr, created: &mut bool) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{mkdirat, openat, Mode, OFlags};
            mkdirat(&self.fd, name, Mode::from_bits_truncate(0o700))?;
            *created = true;
            let fd = openat(
                &self.fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            Ok(Self {
                fd,
                path: self.path.join(name),
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            let path = self.path.join(name);
            std::fs::create_dir(&path)?;
            *created = true;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0x1 | 0x2 | 0x4)
                .custom_flags(0x0200_0000 | 0x0020_0000)
                .open(&path)?;
            if !file.metadata()?.is_dir() || file.metadata()?.file_attributes() & 0x400 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private output folder changed",
                ));
            }
            Ok(Self {
                path,
                _ancestors: vec![file],
            })
        }
        #[cfg(not(any(unix, windows)))]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owned outputs are unsupported",
        ))
    }
    fn directory_file(&self) -> io::Result<File> {
        #[cfg(unix)]
        {
            Ok(File::from(self.fd.try_clone()?))
        }
        #[cfg(windows)]
        {
            self._ancestors
                .last()
                .ok_or_else(|| io::Error::other("missing directory pin"))?
                .try_clone()
        }
        #[cfg(not(any(unix, windows)))]
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(super) fn move_exclusive(
        &self,
        from: &OsStr,
        to_dir: &Directory,
        to: &OsStr,
    ) -> io::Result<()> {
        #[cfg(windows)]
        let (_source_guard, _target_guard) = (self.operation_guard()?, to_dir.operation_guard()?);
        #[cfg(any(target_os = "linux", target_vendor = "apple"))]
        {
            native_result(rustix::fs::renameat_with(
                &self.fd,
                from,
                &to_dir.fd,
                to,
                rustix::fs::RenameFlags::NOREPLACE,
            ))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
            let wide = |path: PathBuf| -> io::Result<Vec<u16>> {
                let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
                if path.contains(&0) {
                    return Err(io::ErrorKind::InvalidInput.into());
                }
                path.push(0);
                Ok(path)
            };
            let a = wide(self.path.join(from))?;
            let b = wide(to_dir.path.join(to))?;
            // SAFETY: live NUL-terminated buffers are borrowed for this call;
            // omitting REPLACE_EXISTING preserves a racing destination.
            if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        #[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
        {
            let _ = (from, to_dir, to);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "atomic no-replace capture is unsupported",
            ))
        }
    }
    fn unlink_private(&self, name: &OsStr, directory: bool) -> io::Result<()> {
        #[cfg(windows)]
        let _guard = self.operation_guard()?;
        #[cfg(unix)]
        {
            rustix::fs::unlinkat(
                &self.fd,
                name,
                if directory {
                    rustix::fs::AtFlags::REMOVEDIR
                } else {
                    rustix::fs::AtFlags::empty()
                },
            )
            .map_err(Into::into)
        }
        #[cfg(not(unix))]
        {
            if directory {
                std::fs::remove_dir(self.path.join(name))
            } else {
                std::fs::remove_file(self.path.join(name))
            }
        }
    }
    fn actual_path(&self) -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            if let Ok(path) = std::fs::read_link(format!("/proc/self/fd/{}", self.fd.as_raw_fd())) {
                return path;
            }
        }
        self.path.clone()
    }
}
struct Private {
    parent: Arc<Directory>,
    dir: Directory,
    name: OsString,
    identity: same_file::Handle,
    removed: AtomicBool,
    retained: std::sync::Mutex<Vec<(bool, OsString)>>,
    #[cfg(windows)]
    guard: std::sync::Mutex<Option<same_file::Handle>>,
}
impl Private {
    fn create(parent: Arc<Directory>, orphaned: &mut Vec<PathBuf>) -> io::Result<Self> {
        for _ in 0..100 {
            let name = nonce(".tagrex-output-private-")?;
            let mut created = false;
            let opened = (|| {
                let dir = parent.child(&name, &mut created)?;
                let identity = same_file::Handle::from_file(dir.directory_file()?)?;
                #[cfg(windows)]
                let guard = dir.operation_guard()?;
                Ok(Self {
                    parent: parent.clone(),
                    dir,
                    name: name.clone(),
                    identity,
                    removed: AtomicBool::new(false),
                    retained: std::sync::Mutex::default(),
                    #[cfg(windows)]
                    guard: std::sync::Mutex::new(Some(guard)),
                })
            })();
            match opened {
                Ok(private) => return Ok(private),
                Err(error) if created => {
                    // No Drop guesses ownership of a directory whose pinning
                    // failed after mkdir. Keep its exact name for inspection.
                    orphaned.push(parent.actual_path().join(&name));
                    return Err(error);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::ErrorKind::AlreadyExists.into())
    }
    fn path(&self) -> PathBuf {
        self.dir.actual_path()
    }
    fn remember(&self, parent: bool, name: &OsStr) {
        self.retained
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((parent, name.to_owned()));
    }
    fn forget(&self, parent: bool, name: &OsStr) {
        self.retained
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(base, entry)| *base != parent || entry != name);
    }
    fn retained_paths(&self) -> Vec<PathBuf> {
        self.retained
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(parent, name)| {
                if *parent {
                    self.parent.actual_path().join(name)
                } else {
                    self.path().join(name)
                }
            })
            .collect()
    }
    fn rescue(&self, proof: &Proof, label: &str) -> io::Result<()> {
        // A held original can survive an externally removed/replaced backup.
        // Give verified bytes a recovery name before releasing the last handle.
        if image(proof.file.as_file())?.0 != proof.image {
            return Err(io::Error::other("held recovery source was modified"));
        }
        let name = nonce(&format!("rescue-{label}-"))?;
        let mut destination = self.dir.open_regular(&name, true)?;
        self.remember(false, &name);
        let mut source = proof.file.as_file();
        source.seek(SeekFrom::Start(0))?;
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .filter(|size| *size <= MAX_FILE_BYTES)
                .ok_or_else(|| io::Error::other("recovery source grew"))?;
            destination.write_all(&buffer[..count])?;
        }
        destination.sync_all()?;
        if image(&destination)?.0 != proof.image || image(proof.file.as_file())?.0 != proof.image {
            return Err(io::Error::other(
                "recovery copy is unverified; retained for inspection",
            ));
        }
        Ok(())
    }
    fn capture(&self, from_dir: &Directory, from: &OsStr) -> io::Result<OsString> {
        let name = nonce("capture-")?;
        from_dir.move_exclusive(from, &self.dir, &name)?;
        if std::ptr::eq(from_dir, &self.dir) {
            self.forget(false, from);
        }
        self.remember(false, &name);
        Ok(name)
    }
    fn discard(
        &self,
        dir: &Directory,
        name: &OsStr,
        proof: &Proof,
        missing_ok: bool,
    ) -> io::Result<()> {
        match dir.open_regular(name, false) {
            Err(error) if missing_ok && error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
            Ok(file) => {
                if !proof.matches(file, true)? {
                    return Err(io::Error::other("output cleanup ownership changed"));
                }
            }
        }
        let captured = self.capture(dir, name)?;
        // Check the captured object, rather than unlinking a name after a
        // preceding path check. All verification failures restore or retain.
        match proof.at(&self.dir, &captured, false) {
            Ok(true) => {
                self.dir.unlink_private(&captured, false)?;
                self.forget(false, &captured);
                Ok(())
            }
            failure => {
                let restored = self.dir.move_exclusive(&captured, dir, name);
                if restored.is_ok() {
                    self.forget(false, &captured);
                }
                Err(io::Error::other(format!(
                    "output cleanup captured changed object {} (proof {:?}); restoration {:?}",
                    self.path().join(&captured).display(),
                    failure,
                    restored
                )))
            }
        }
    }
    fn remove_empty(&self) -> io::Result<()> {
        observe!(Event::BeforePrivateRemove, self.path());
        #[cfg(windows)]
        self.guard.lock().unwrap_or_else(|p| p.into_inner()).take();
        let captured = nonce(".tagrex-output-empty-")?;
        self.parent
            .move_exclusive(&self.name, &self.parent, &captured)?;
        self.remember(true, &captured);
        let verified: io::Result<bool> = (|| {
            #[cfg(unix)]
            let file = {
                use rustix::fs::{openat, Mode, OFlags};
                File::from(openat(
                    &self.parent.fd,
                    &captured,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )?)
            };
            #[cfg(windows)]
            let file = {
                use std::os::windows::fs::OpenOptionsExt;
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(7)
                    .custom_flags(0x0200_0000 | 0x0020_0000)
                    .open(self.parent.path.join(&captured))?
            };
            #[cfg(not(any(unix, windows)))]
            return Err(io::ErrorKind::Unsupported.into());
            same_handles(&self.identity, &same_file::Handle::from_file(file)?)
        })();
        if !matches!(verified, Ok(true)) {
            let restore = self
                .parent
                .move_exclusive(&captured, &self.parent, &self.name);
            if restore.is_ok() {
                self.forget(true, &captured);
                #[cfg(windows)]
                {
                    *self.guard.lock().unwrap_or_else(|p| p.into_inner()) =
                        self.dir.operation_guard().ok();
                }
            }
            return Err(io::Error::other(format!(
                "private output directory changed ({verified:?}); restoration {restore:?}"
            )));
        }
        match self.parent.unlink_private(&captured, true) {
            Ok(()) => {
                // A held descriptor remains valid after rmdir on Unix. It must
                // no longer be presented as a retained recovery namespace.
                self.removed.store(true, Ordering::Release);
                self.forget(true, &captured);
                Ok(())
            }
            Err(error) => {
                if self
                    .parent
                    .move_exclusive(&captured, &self.parent, &self.name)
                    .is_ok()
                {
                    self.forget(true, &captured);
                    #[cfg(windows)]
                    {
                        *self.guard.lock().unwrap_or_else(|p| p.into_inner()) =
                            self.dir.operation_guard().ok();
                    }
                }
                Err(error)
            }
        }
    }
}
struct Entry {
    root: Arc<Directory>,
    parent: Arc<Directory>,
    private: Arc<Private>,
    target: PathBuf,
    name: OsString,
    candidate: OsString,
    candidate_owned: bool,
    proof: Proof,
    before: Option<Proof>,
    backup: Option<OsString>,
    published: bool,
}
impl Entry {
    fn baseline_current(&self) -> Result<(), OutputError> {
        if !self.parent.logical_current(&self.root).unwrap_or(false) {
            return Err(OutputError::Changed(self.parent.path.clone()));
        }
        match &self.before {
            Some(proof) if proof.at(&self.parent, &self.name, true)? => Ok(()),
            None if !self.parent.regular(&self.name)? => Ok(()),
            _ => Err(OutputError::Changed(self.target.clone())),
        }
    }
    fn publish(&mut self) -> Result<(), OutputError> {
        self.baseline_current()?;
        if !self.proof.at(&self.private.dir, &self.candidate, true)? {
            return Err(OutputError::Changed(
                self.private.path().join(&self.candidate),
            ));
        }
        // Seal the candidate's known name into a fresh private capture before
        // transporting it to a public namespace. A replacement of its staging
        // name is restored/retained and never treated as the generated output.
        let prior = self.candidate.clone();
        let captured = self.private.capture(&self.private.dir, &prior)?;
        self.candidate = captured.clone();
        observe!(
            Event::CandidateCaptured,
            self.private.path().join(&captured)
        );
        match self.proof.at(&self.private.dir, &captured, false) {
            Ok(true) => self.proof.refresh()?,
            failure => {
                // Once a private capture is untrusted, retain it rather than
                // moving its replacement into another namespace.
                self.candidate_owned = false;
                return Err(io::Error::other(format!(
                    "staged output capture changed: {failure:?}"
                ))
                .into());
            }
        }
        if let Some(before) = &mut self.before {
            let backup = self.private.capture(&self.parent, &self.name)?;
            self.backup = Some(backup.clone());
            observe!(Event::BackupCaptured, self.private.path().join(&backup));
            if !before.at(&self.private.dir, &backup, false)? {
                return Err(OutputError::Changed(self.target.clone()));
            }
            before.refresh()?;
        }
        self.private
            .dir
            .move_exclusive(&self.candidate, &self.parent, &self.name)?;
        self.private.forget(false, &self.candidate);
        self.candidate_owned = false;
        self.published = true;
        if !self.proof.at(&self.parent, &self.name, false)? {
            return Err(OutputError::Changed(self.target.clone()));
        }
        self.proof.refresh()?;
        Ok(())
    }
    fn rollback(&mut self) -> io::Result<()> {
        if let Some(backup) = &self.backup {
            if !self
                .before
                .as_ref()
                .is_some_and(|before| before.at(&self.private.dir, backup, false).unwrap_or(false))
            {
                return Err(io::Error::other(
                    "original output backup changed; retained with published output",
                ));
            }
        }
        if self.published {
            if !self.proof.at(&self.parent, &self.name, true)? {
                return Err(io::Error::other(
                    "published output changed or disappeared; original backup retained",
                ));
            }
            self.private
                .discard(&self.parent, &self.name, &self.proof, false)?;
            self.published = false;
        }
        if let Some(prior) = self.backup.clone() {
            observe!(Event::BeforeRestore, self.private.path().join(&prior));
            // A backup name can be replaced after the initial rollback check.
            // Capture it again and verify the captured object before giving it
            // a public name; a foreign replacement remains private evidence.
            let backup = self.private.capture(&self.private.dir, &prior)?;
            self.backup = Some(backup.clone());
            observe!(Event::RestoreCaptured, self.private.path().join(&backup));
            if !self.before.as_ref().is_some_and(|before| {
                before
                    .at(&self.private.dir, &backup, false)
                    .unwrap_or(false)
            }) {
                return Err(io::Error::other(
                    "captured original backup changed before restoration",
                ));
            }
            self.private
                .dir
                .move_exclusive(&backup, &self.parent, &self.name)?;
            self.private.forget(false, &backup);
            self.backup = None;
            if !self
                .before
                .as_ref()
                .is_some_and(|before| before.at(&self.parent, &self.name, false).unwrap_or(false))
            {
                return Err(io::Error::other("restored output ownership changed"));
            }
        }
        Ok(())
    }
}
fn recovery(
    entries: &[Entry],
    privates: &[Arc<Private>],
    phase: &'static str,
    cause: impl std::fmt::Display,
) -> OutputError {
    let mut reason = cause.to_string();
    if reason.len() > 1024 {
        let mut end = 1024;
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        reason.truncate(end);
        reason.push('…');
    }
    // Recovery manifests are evidence for a live failed batch, not a claim that
    // a future process may replay or delete their paths automatically.
    for entry in entries {
        if entry.private.removed.load(Ordering::Acquire) {
            continue;
        }
        if let Some(before) = &entry.before {
            if entry
                .backup
                .as_ref()
                .is_some_and(|name| !before.at(&entry.private.dir, name, false).unwrap_or(false))
            {
                let _ = entry.private.rescue(before, "original");
            }
        }
        if entry.published
            && !entry
                .proof
                .at(&entry.parent, &entry.name, false)
                .unwrap_or(false)
        {
            let _ = entry.private.rescue(&entry.proof, "published");
        }
    }
    for private in privates
        .iter()
        .filter(|private| !private.removed.load(Ordering::Acquire))
    {
        let records:Vec<_>=entries.iter().filter(|entry|Arc::ptr_eq(&entry.private,private)).map(|entry|serde_json::json!({
            "target":entry.target,"candidate":entry.candidate.to_string_lossy(),"backup":entry.backup.as_ref().map(|name|name.to_string_lossy()),"published":entry.published,
            "before":entry.before.as_ref().map(|proof|serde_json::json!({"bytes":proof.image.len,"sha256":hex(&proof.image.hash)})),
            "candidate_bytes":entry.proof.image.len,"candidate_sha256":hex(&entry.proof.image.hash),"candidate_owned":entry.candidate_owned
        })).collect();
        if let Ok(mut file) = private.dir.open_regular(OsStr::new("recovery.json"), true) {
            if let Ok(bytes) = serde_json::to_vec(
                &serde_json::json!({"version":1,"automatic_recovery":false,"phase":phase,"cause":reason,"outputs":records}),
            ) {
                let _ = file.write_all(&bytes).and_then(|_| file.sync_all());
            }
        }
    }
    let mut paths: Vec<_> = privates
        .iter()
        .filter(|private| !private.removed.load(Ordering::Acquire))
        .map(|private| private.path())
        .collect();
    paths.extend(privates.iter().flat_map(|private| private.retained_paths()));
    paths.extend(
        entries
            .iter()
            .filter(|entry| entry.published || entry.backup.is_some())
            .map(|entry| entry.parent.actual_path().join(&entry.name)),
    );
    // Do not report retired public/capture slots as retained files. Keep
    // inaccessible paths: absence is the only definite reason to omit one.
    paths.retain(|path| {
        !matches!(std::fs::symlink_metadata(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound)
    });
    paths.sort();
    paths.dedup();
    OutputError::RecoveryRequired {
        phase,
        paths,
        cause: reason,
        _retained: None,
    }
}
fn cleanup(entries: &[Entry], privates: &[Arc<Private>]) -> io::Result<()> {
    let mut failures = Vec::new();
    for entry in entries {
        if let (Some(backup), Some(before)) = (&entry.backup, &entry.before) {
            if let Err(error) = entry
                .private
                .discard(&entry.private.dir, backup, before, false)
            {
                failures.push(error.to_string());
            }
        }
        if entry.candidate_owned {
            if let Err(error) =
                entry
                    .private
                    .discard(&entry.private.dir, &entry.candidate, &entry.proof, true)
            {
                failures.push(error.to_string());
            }
        } else {
            match entry.private.dir.open_regular(&entry.candidate, false) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => failures
                    .push("the moved candidate name was reused; replacement retained".into()),
            }
        }
    }
    if failures.is_empty() {
        for private in privates {
            if let Err(error) = private.remove_empty() {
                failures.push(error.to_string());
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(failures.join("; ")))
    }
}

pub(super) fn write_batch(
    root: &Path,
    outputs: &[(PathBuf, Vec<u8>)],
    overwrite: bool,
) -> Result<(), OutputError> {
    if outputs.is_empty() {
        return Ok(());
    }
    if outputs.len() > MAX_OUTPUTS || root.as_os_str().len() > MAX_PATH_BYTES {
        return Err(AdmissionError::TooLarge.into());
    }
    let total = outputs
        .iter()
        .try_fold(0usize, |sum, (_, bytes)| sum.checked_add(bytes.len()))
        .ok_or(AdmissionError::TooLarge)?;
    let path_bytes = outputs
        .iter()
        .try_fold(0usize, |sum, (path, _)| {
            let len = path.as_os_str().len();
            (len <= MAX_PATH_BYTES).then_some(())?;
            sum.checked_add(len)
        })
        .ok_or(AdmissionError::TooLarge)?;
    if outputs.len() > MAX_OUTPUTS || total > MAX_OUTPUT_BYTES || path_bytes > MAX_PATH_TOTAL {
        return Err(AdmissionError::TooLarge.into());
    }
    let retained = path_bytes
        .saturating_mul(8)
        .saturating_add(outputs.len().saturating_mul(MAX_PATH_BYTES * 8 + 4096));
    let mut lease = Budget::process().try_acquire(Charge {
        retained,
        transient: 2 * 1024 * 1024,
        jobs: 1,
    })?;
    let distinct: HashSet<_> = outputs
        .iter()
        .filter_map(|(path, _)| path.parent())
        .collect();
    let descriptors = distinct.iter().fold(
        32usize.saturating_add(outputs.len().saturating_mul(2)),
        |sum, parent| {
            #[cfg(windows)]
            let parent_handles = parent.components().count();
            #[cfg(not(windows))]
            let parent_handles = {
                let _ = parent;
                1
            };
            #[cfg(windows)]
            let private_handles = 3;
            #[cfg(not(windows))]
            let private_handles = 2;
            sum.saturating_add(parent_handles)
                .saturating_add(private_handles)
        },
    );
    let _descriptors = Descriptors::acquire(descriptors)?;
    let mut result = perform_batch(root, outputs, overwrite);
    if let Err(OutputError::RecoveryRequired {
        paths,
        cause,
        _retained,
        ..
    }) = &mut result
    {
        let actual = paths
            .iter()
            .fold(4096usize + cause.capacity(), |sum, path| {
                sum.saturating_add(path.as_os_str().len().saturating_mul(2))
                    .saturating_add(128)
            })
            .min(retained);
        lease
            .try_resize(Charge {
                retained: actual,
                ..Charge::default()
            })
            .expect("recovery transfer only reduces an existing reservation");
        *_retained = Some(lease);
    }
    result
}

fn perform_batch(
    root: &Path,
    outputs: &[(PathBuf, Vec<u8>)],
    overwrite: bool,
) -> Result<(), OutputError> {
    let root_pin = Arc::new(Directory::open(root, root)?);
    let mut parents: HashMap<PathBuf, Arc<Directory>> = HashMap::new();
    let mut names = HashSet::new();
    let mut planned = Vec::new();
    let mut old_bytes = 0u64;
    // Open every baseline and check all aggregate limits before creating staging
    // files. Duplicate output identities/names are refused rather than editing
    // one physical original twice through aliases.
    for (target, _) in outputs {
        let parent = target
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output parent"))?;
        let dir = Directory::under_root(&root_pin, parent)?;
        let canonical = dir.path.clone();
        let parent = parents.entry(canonical).or_insert(dir).clone();
        let name = target
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output name"))?
            .to_owned();
        #[cfg(windows)]
        let key = (parent.path.clone(), name.to_string_lossy().to_lowercase());
        #[cfg(not(windows))]
        let key = (parent.path.clone(), name.clone());
        if !names.insert(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate output destination",
            )
            .into());
        }
        let before = if parent.regular(&name)? {
            if !overwrite {
                return Err(
                    io::Error::new(io::ErrorKind::AlreadyExists, "output already exists").into(),
                );
            }
            let file = parent.open_regular(&name, false)?;
            let metadata = file.metadata()?;
            if metadata.permissions().readonly() {
                return Err(
                    io::Error::new(io::ErrorKind::PermissionDenied, "output is read-only").into(),
                );
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err(AdmissionError::TooLarge.into());
            }
            old_bytes = old_bytes
                .checked_add(metadata.len())
                .filter(|bytes| *bytes <= MAX_OLD_BYTES)
                .ok_or(AdmissionError::TooLarge)?;
            Some(file)
        } else {
            None
        };
        planned.push((parent, name, target.clone(), before));
    }
    let mut proofs = Vec::with_capacity(planned.len());
    for (_, _, _, before) in &mut planned {
        let proof = before.take().map(Proof::read).transpose()?;
        if let Some(proof) = &proof {
            if proofs
                .iter()
                .flatten()
                .any(|previous: &Proof| previous.file == proof.file)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate hard-linked output originals",
                )
                .into());
            }
        }
        proofs.push(proof);
    }
    let mut privates = Vec::<Arc<Private>>::new();
    let mut orphaned = Vec::new();
    let mut parent_private = HashMap::<PathBuf, Arc<Private>>::new();
    let mut entries = Vec::new();
    let staged: Result<(), OutputError> = (|| {
        for (index, ((parent, name, target, _), before)) in
            planned.into_iter().zip(proofs).enumerate()
        {
            let private = match parent_private.get(&parent.path) {
                Some(private) => private.clone(),
                None => {
                    let private = Arc::new(Private::create(parent.clone(), &mut orphaned)?);
                    privates.push(private.clone());
                    parent_private.insert(parent.path.clone(), private.clone());
                    private
                }
            };
            let candidate: OsString = format!("candidate-{index}").into();
            let file = private.dir.open_regular(&candidate, true)?;
            let proof = Proof::read(file)?;
            let entry = Entry {
                root: root_pin.clone(),
                parent,
                private,
                target,
                name,
                candidate,
                candidate_owned: true,
                proof,
                before,
                backup: None,
                published: false,
            };
            entries.push(entry);
            let entry = entries.last_mut().expect("entry was just inserted");
            if let Some(before) = &entry.before {
                entry
                    .proof
                    .file
                    .as_file()
                    .set_permissions(before.file.as_file().metadata()?.permissions())?;
            }
            let bytes = &outputs[index].1;
            let mut file = entry.proof.file.as_file();
            let mut written = 0usize;
            let mut hash = Sha256::new();
            while written < bytes.len() {
                let count = file.write(&bytes[written..bytes.len().min(written + 64 * 1024)])?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "output staging wrote zero bytes",
                    )
                    .into());
                }
                hash.update(&bytes[written..written + count]);
                written += count;
                entry.proof.image = Image {
                    len: written as u64,
                    hash: hash.clone().finalize().into(),
                };
                entry.proof.stamp = Stamp::of(
                    entry.proof.file.as_file(),
                    &entry.proof.file.as_file().metadata()?,
                )?;
            }
            if bytes.is_empty() {
                entry.proof.image = Image::bytes(bytes);
            }
            observe!(
                Event::BeforeSync,
                entry.private.path().join(&entry.candidate)
            );
            file.sync_all()?;
            entry.proof.refresh()?;
            observe!(Event::Staged, entry.private.path().join(&entry.candidate));
        }
        Ok(())
    })();
    if let Err(cause) = staged {
        if !orphaned.is_empty() {
            let cleanup = cleanup(&entries, &privates);
            let mut error = recovery(
                &entries,
                &privates,
                "private directory pinning incomplete",
                format!("{cause}; cleanup {cleanup:?}"),
            );
            if let OutputError::RecoveryRequired { paths, .. } = &mut error {
                paths.extend(orphaned);
                paths.sort();
                paths.dedup();
            }
            return Err(error);
        }
        return match cleanup(&entries, &privates) {
            Ok(()) => Err(cause),
            Err(error) => Err(recovery(
                &entries,
                &privates,
                "staging cleanup",
                format!("{cause}; {error}"),
            )),
        };
    }
    // Verify the complete batch before publishing any member, then repeat each
    // baseline check at its actual transport boundary.
    let result: Result<(), OutputError> = (|| {
        for entry in &entries {
            entry.baseline_current()?;
            if !entry.proof.at(&entry.private.dir, &entry.candidate, true)? {
                return Err(OutputError::Changed(
                    entry.private.path().join(&entry.candidate),
                ));
            }
        }
        for entry in &mut entries {
            observe!(Event::BeforePublish, entry.target.clone());
            entry.publish()?;
            #[cfg(test)]
            super::PUBLICATION_HOOK.with(|hook| {
                if let Some(callback) = hook.borrow_mut().as_mut() {
                    callback(&entry.target);
                }
            });
        }
        observe!(Event::BeforeCommit, root_pin.actual_path());
        for entry in &entries {
            if !entry.parent.logical_current(&entry.root).unwrap_or(false) {
                return Err(OutputError::Changed(entry.parent.path.clone()));
            }
            if !entry.proof.at(&entry.parent, &entry.name, true)? {
                return Err(OutputError::Changed(entry.target.clone()));
            }
            if let Some(backup) = &entry.backup {
                if !entry.before.as_ref().is_some_and(|proof| {
                    proof.at(&entry.private.dir, backup, true).unwrap_or(false)
                }) {
                    return Err(OutputError::Changed(entry.private.path().join(backup)));
                }
            }
        }
        Ok(())
    })();
    if let Err(cause) = result {
        let mut failures = Vec::new();
        for entry in entries.iter_mut().rev() {
            if let Err(error) = entry.rollback() {
                failures.push(error.to_string());
            }
        }
        if !failures.is_empty() {
            return Err(recovery(
                &entries,
                &privates,
                "rollback incomplete",
                format!("{cause}; {}", failures.join("; ")),
            ));
        }
        return match cleanup(&entries, &privates) {
            Ok(()) => Err(cause),
            Err(error) => Err(recovery(
                &entries,
                &privates,
                "rollback cleanup",
                format!("{cause}; {error}"),
            )),
        };
    }
    // All members were verified; publication is committed. Cleanup failure may
    // retain backups, but must never undo a batch after deleting an earlier one.
    #[cfg(test)]
    if let Err(error) = observe(Event::BeforeCleanup, &root_pin.actual_path()) {
        return Err(recovery(
            &entries,
            &privates,
            "published; cleanup incomplete",
            error,
        ));
    }
    cleanup(&entries, &privates)
        .map_err(|error| recovery(&entries, &privates, "published; cleanup incomplete", error))
}
