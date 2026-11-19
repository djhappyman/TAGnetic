//! Durable full-media evidence used by the narrow Linux FLAC text Executor route.
//!
//! The general mutation/staging migration remains separate. This preserves the
//! original inode by publishing in place. An interrupted publication retains
//! both images; ambiguous current bytes are never automatically restored.
//! Linux is the supported checkpoint platform. Other platforms fail closed.
#[cfg(target_os = "linux")]
use std::fs::File;
use std::{io, path::Path};

pub const SUPPORTED: bool = cfg!(target_os = "linux");
pub const BUFFER_BYTES: usize = 64 * 1024;
#[cfg(target_os = "linux")]
const MANIFEST_LIMIT: usize = 8192;
#[cfg(target_os = "linux")]
pub(crate) const WORKING_BYTES: usize = 2 * BUFFER_BYTES + MANIFEST_LIMIT;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Resource(#[from] crate::resources::AdmissionError),
    #[error("Media recovery storage is full or busy")]
    Capacity,
    #[error("Media source or recovery artifact changed")]
    Changed,
    #[error("Invalid media recovery store: {0}")]
    Invalid(&'static str),
    #[error("Durable media mutation is unsupported on this platform/filesystem")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;
#[cfg(target_os = "linux")]
impl From<rustix::io::Errno> for Error {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub store_bytes: u64,
    pub entries: usize,
    pub media_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            store_bytes: 64 << 30,
            entries: 128,
            media_bytes: 8 << 30,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Phase {
    Preparing = 0,
    Prepared = 1,
    Publishing = 2,
    Published = 3,
    Retired = 4,
}
impl Phase {
    #[cfg(target_os = "linux")]
    fn name(self) -> &'static str {
        match self {
            Self::Preparing => "0-preparing",
            Self::Prepared => "1-prepared",
            Self::Publishing => "2-publishing",
            Self::Published => "3-published",
            Self::Retired => "4-retired",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Preimage,
    Candidate,
    Publish,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Preimage,
    Candidate,
    Original,
    Manifest(Phase),
    Directory,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    EntryCreated,
    CopyStarted(CopyKind),
    Copied(CopyKind, u64),
    CopyFinished(CopyKind),
    BeforeSync(SyncKind),
    Synced(SyncKind),
    StateDurable(Phase),
    BeforeCallback(Phase),
    AfterCallback(Phase),
    BeforeEdit,
    AfterEdit,
    BeforeRetire,
    ArtifactRemoved(u8),
}
/// Cooperative observation/fault boundary. Publication validates established
/// image proofs after successful callbacks. This detects observed changes; it
/// does not isolate private files from every uncooperative external writer.
pub trait Observer {
    fn event(&mut self, _event: Event) -> io::Result<()> {
        Ok(())
    }
    /// A positive short allowance exercises the same partial-write loop as a
    /// short OS write. Zero is WriteZero. Errors abort without hiding evidence.
    fn write_limit(&mut self, _kind: CopyKind, requested: usize) -> io::Result<usize> {
        Ok(requested)
    }
}
impl Observer for () {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub len: u64,
    pub sha256: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    id: String,
    phase: Phase,
    logical: String,
    hint: String,
    #[cfg(target_os = "linux")]
    identity: [u8; 32],
    #[cfg(target_os = "linux")]
    directory_identity: [u8; 32],
    #[cfg(target_os = "linux")]
    artifacts: Vec<(String, [u8; 32])>,
    #[cfg(target_os = "linux")]
    reservation: u64,
    before: Fingerprint,
    after: Option<Fingerprint>,
}
impl Record {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn original_path(&self) -> &Path {
        Path::new(&self.logical)
    }
    pub fn extension_hint(&self) -> &str {
        &self.hint
    }
    pub fn before(&self) -> Fingerprint {
        self.before
    }
    pub fn after(&self) -> Option<Fingerprint> {
        self.after
    }
}
/// Implementations must durably record each acknowledgement before returning.
/// A local synced manifest also records every intent independently of the sink.
pub trait StateSink {
    fn persist(&mut self, record: &Record) -> io::Result<()>;
}
impl<F: FnMut(&Record) -> io::Result<()>> StateSink for F {
    fn persist(&mut self, record: &Record) -> io::Result<()> {
        self(record)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Current {
    Preimage,
    Candidate,
    Ambiguous,
    DifferentIdentity,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::resources::{Budget, Charge, Lease};
    use rustix::fs::{mkdirat, open, openat, unlinkat, AtFlags, FallocateFlags, Mode, OFlags};
    use sha2::{Digest, Sha256};
    use std::{
        collections::HashMap,
        ffi::OsStr,
        io::{Read, Seek, Write},
        os::unix::{fs::MetadataExt, io::AsRawFd},
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    impl Record {
        pub(crate) fn journal_bytes(&self) -> Result<Vec<u8>> {
            let bytes = encode(self);
            // Never persist a record that the strict recovery decoder rejects.
            decode(&bytes)?;
            Ok(bytes)
        }
        pub(crate) fn from_journal(bytes: &[u8]) -> Result<Self> {
            decode(bytes)
        }
        pub(crate) fn source_identity(&self) -> [u8; 32] {
            self.identity
        }
    }
    pub(crate) fn file_identity(file: &File) -> Result<[u8; 32]> {
        identity(file)
    }
    pub(crate) fn operation_id() -> Result<String> {
        new_id()
    }
    const OVERHEAD: u64 = 5 * MANIFEST_LIMIT as u64;
    const PROTECTION: &str = "user.tagrex.media_recovery_v1";
    const PROTECTION_MAGIC: &[u8] = b"TRX-MEDIA-OBJECT\x01";
    const PHASES: [Phase; 5] = [
        Phase::Preparing,
        Phase::Prepared,
        Phase::Publishing,
        Phase::Published,
        Phase::Retired,
    ];
    fn flags(write: bool) -> OFlags {
        (if write { OFlags::RDWR } else { OFlags::RDONLY })
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK
    }
    fn fd_path(file: &File) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
    }
    fn directory(path: &Path) -> Result<File> {
        Ok(File::from(open(
            path,
            flags(false) | OFlags::DIRECTORY,
            Mode::empty(),
        )?))
    }
    fn child_dir(parent: &File, name: &OsStr) -> Result<File> {
        Ok(File::from(openat(
            parent,
            name,
            flags(false) | OFlags::DIRECTORY,
            Mode::empty(),
        )?))
    }
    fn private(file: &File, dir: bool) -> Result<()> {
        let m = file.metadata()?;
        if (if dir { !m.is_dir() } else { !m.is_file() })
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o077 != 0
            || (!dir && m.nlink() != 1)
        {
            return Err(Error::Invalid(
                "artifacts must be private, owned and unlinked elsewhere",
            ));
        }
        Ok(())
    }
    fn create(parent: &File, name: &str) -> Result<File> {
        let file = File::from(openat(
            parent,
            name,
            flags(true) | OFlags::CREATE | OFlags::EXCL,
            Mode::from_bits_truncate(0o600),
        )?);
        private(&file, false)?;
        protect(&file)?;
        Ok(file)
    }
    fn artifact(parent: &File, name: &str) -> Result<File> {
        let file = File::from(openat(parent, name, flags(false), Mode::empty())?);
        private(&file, false)?;
        require_protected(&file)?;
        Ok(file)
    }
    fn same(a: &File, b: &File) -> Result<bool> {
        Ok(same_file::Handle::from_file(a.try_clone()?)?
            == same_file::Handle::from_file(b.try_clone()?)?)
    }
    fn identity(file: &File) -> Result<[u8; 32]> {
        let m = file.metadata()?;
        if !m.is_file() {
            return Err(Error::Invalid("original must be a regular file"));
        }
        object_identity(file)
    }
    fn object_identity(file: &File) -> Result<[u8; 32]> {
        let m = file.metadata()?;
        let birth = m
            .created()
            .map_err(|_| Error::Unsupported)?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Unsupported)?
            .as_nanos();
        let mut out = [0; 32];
        out[..8].copy_from_slice(&m.dev().to_le_bytes());
        out[8..16].copy_from_slice(&m.ino().to_le_bytes());
        out[16..].copy_from_slice(&birth.to_le_bytes());
        Ok(out)
    }
    fn protected(file: &File) -> Result<bool> {
        let mut bytes = [0; 64];
        match rustix::fs::fgetxattr(file, PROTECTION, &mut bytes) {
            Ok(n) => {
                if n != PROTECTION_MAGIC.len() + 32
                    || &bytes[..PROTECTION_MAGIC.len()] != PROTECTION_MAGIC
                    || bytes[PROTECTION_MAGIC.len()..n] != object_identity(file)?
                {
                    return Err(Error::Changed);
                }
                Ok(true)
            }
            Err(rustix::io::Errno::NODATA) => Ok(false),
            Err(error) => Err(protection_error(error)),
        }
    }
    fn require_protected(file: &File) -> Result<()> {
        if !protected(file)? {
            return Err(Error::Changed);
        }
        Ok(())
    }
    fn protect(file: &File) -> Result<()> {
        if protected(file)? {
            return Ok(());
        }
        let mut bytes = PROTECTION_MAGIC.to_vec();
        bytes.extend(object_identity(file)?);
        rustix::fs::fsetxattr(file, PROTECTION, &bytes, rustix::fs::XattrFlags::CREATE)
            .map_err(protection_error)?;
        Ok(())
    }
    fn protection_error(error: rustix::io::Errno) -> Error {
        if matches!(
            error,
            rustix::io::Errno::OPNOTSUPP | rustix::io::Errno::NOSYS
        ) {
            Error::Unsupported
        } else {
            error.into()
        }
    }
    type Witness = (u64, i64, i64, i64, i64);
    fn witness(file: &File) -> Result<Witness> {
        let m = file.metadata()?;
        Ok((
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        ))
    }
    fn fingerprint(file: &mut File, max: u64) -> Result<Fingerprint> {
        fingerprint_stamped(file, max).map(|(image, _)| image)
    }
    fn fingerprint_stamped(file: &mut File, max: u64) -> Result<(Fingerprint, Witness)> {
        if file.metadata()?.len() > max {
            return Err(Error::Capacity);
        }
        let before = witness(file)?;
        file.rewind()?;
        let mut hash = Sha256::new();
        let mut len = 0u64;
        let mut buffer = [0; BUFFER_BYTES];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            len = len
                .checked_add(n as u64)
                .filter(|n| *n <= max)
                .ok_or(Error::Capacity)?;
            hash.update(&buffer[..n]);
        }
        if before != witness(file)? || len != before.0 {
            return Err(Error::Changed);
        }
        Ok((
            Fingerprint {
                len,
                sha256: hash.finalize().into(),
            },
            before,
        ))
    }
    struct ImageSeal {
        file: File,
        identity: [u8; 32],
        witness: Witness,
    }
    impl ImageSeal {
        fn new(mut file: File, expected: Fingerprint, max: u64) -> Result<Self> {
            let (actual, witness) = fingerprint_stamped(&mut file, max)?;
            if actual != expected {
                return Err(Error::Changed);
            }
            Ok(Self {
                identity: identity(&file)?,
                file,
                witness,
            })
        }
        fn validate(&self) -> Result<()> {
            if identity(&self.file)? != self.identity || witness(&self.file)? != self.witness {
                return Err(Error::Changed);
            }
            Ok(())
        }
    }
    struct CheckedObserver<'a> {
        inner: &'a mut dyn Observer,
        guard: &'a dyn Fn() -> Result<()>,
    }
    impl Observer for CheckedObserver<'_> {
        fn event(&mut self, event: Event) -> io::Result<()> {
            self.inner.event(event)?;
            (self.guard)().map_err(io::Error::other)
        }
        fn write_limit(&mut self, kind: CopyKind, requested: usize) -> io::Result<usize> {
            let allowed = self.inner.write_limit(kind, requested)?;
            (self.guard)().map_err(io::Error::other)?;
            Ok(allowed)
        }
    }
    struct CheckedSink<'a> {
        inner: &'a mut dyn StateSink,
        guard: &'a dyn Fn() -> Result<()>,
    }
    impl StateSink for CheckedSink<'_> {
        fn persist(&mut self, record: &Record) -> io::Result<()> {
            self.inner.persist(record)?;
            (self.guard)().map_err(io::Error::other)
        }
    }
    fn image_seals(
        dir: &File,
        record: &Record,
        max: u64,
        partial: bool,
    ) -> Result<HashMap<u8, (String, ImageSeal)>> {
        let mut images = HashMap::new();
        for (index, (name, _)) in record.artifacts.iter().enumerate() {
            let expected = if name == "preimage" {
                Some(record.before)
            } else if name == &format!("candidate.{}", record.hint) {
                record.after
            } else {
                None
            };
            if let Some(expected) = expected {
                match artifact(dir, name) {
                    Ok(file) => {
                        images.insert(
                            index as u8,
                            (name.clone(), ImageSeal::new(file, expected, max)?),
                        );
                    }
                    Err(Error::Io(error)) if partial && error.kind() == io::ErrorKind::NotFound => {
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(images)
    }
    struct CleanupObserver<'a> {
        inner: &'a mut dyn Observer,
        store: &'a Store,
        original: &'a BoundMedia,
        source: &'a ImageSeal,
        dir: Option<File>,
        id: &'a str,
        images: HashMap<u8, (String, ImageSeal)>,
        terminal: Option<(String, ImageSeal, Fingerprint)>,
    }
    impl CleanupObserver<'_> {
        fn bind_terminal(&mut self, name: &str, file: &File, record: &Record) -> Result<()> {
            let bytes = encode(record);
            let expected = Fingerprint {
                len: bytes.len() as u64,
                sha256: Sha256::digest(&bytes).into(),
            };
            let seal = ImageSeal::new(file.try_clone()?, expected, MANIFEST_LIMIT as u64)?;
            self.terminal = Some((name.to_string(), seal, expected));
            self.validate()
        }
        fn validate(&self) -> Result<()> {
            self.store.validate()?;
            self.original.validate()?;
            self.source.validate()?;
            if let Some((name, seal, expected)) = &self.terminal {
                seal.validate()?;
                let mut named = artifact(&self.store.0.dir, name)?;
                if !same(&seal.file, &named)?
                    || fingerprint(&mut named, MANIFEST_LIMIT as u64)? != *expected
                {
                    return Err(Error::Changed);
                }
            }
            if !self.images.is_empty() {
                let dir = self.dir.as_ref().ok_or(Error::Changed)?;
                if !same(dir, &child_dir(&self.store.0.dir, OsStr::new(self.id))?)? {
                    return Err(Error::Changed);
                }
                for (name, seal) in self.images.values() {
                    seal.validate()?;
                    if !same(&seal.file, &artifact(dir, name)?)? {
                        return Err(Error::Changed);
                    }
                }
            }
            Ok(())
        }
    }
    impl Observer for CleanupObserver<'_> {
        fn event(&mut self, event: Event) -> io::Result<()> {
            // Only the explicitly removed artifact may stop participating in
            // the proof; all remaining images keep their original witnesses.
            if let Event::ArtifactRemoved(index) = event {
                self.images.remove(&index);
            }
            self.inner.event(event)?;
            self.validate().map_err(io::Error::other)
        }
    }
    fn sync(file: &File, kind: SyncKind, observer: &mut dyn Observer) -> Result<()> {
        observer.event(Event::BeforeSync(kind))?;
        file.sync_all()?;
        observer.event(Event::Synced(kind))?;
        Ok(())
    }
    struct CopyProof<'a> {
        guard: &'a dyn Fn() -> Result<()>,
        input: Witness,
        output: Witness,
    }
    fn copy(
        input: &mut File,
        output: &mut File,
        maximum: u64,
        kind: CopyKind,
        observer: &mut dyn Observer,
        proof: Option<CopyProof<'_>>,
    ) -> Result<Fingerprint> {
        let mut output_witness = proof.as_ref().map(|p| p.output).unwrap_or(witness(output)?);
        // These baselines are captured before public callbacks. Publication
        // supplies the witnesses already bound to the expected SHA-256 images.
        let before = proof.as_ref().map(|p| p.input).unwrap_or(witness(input)?);
        observer.event(Event::CopyStarted(kind))?;
        if let Some(proof) = &proof {
            (proof.guard)()?;
        }
        if witness(input)? != before {
            return Err(Error::Changed);
        }
        if before.0 > maximum {
            return Err(Error::Capacity);
        }
        input.rewind()?;
        output.rewind()?;
        let mut buffer = [0; BUFFER_BYTES];
        let mut len = 0u64;
        let mut hash = Sha256::new();
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            len = len
                .checked_add(n as u64)
                .filter(|n| *n <= maximum)
                .ok_or(Error::Capacity)?;
            hash.update(&buffer[..n]);
            let mut at = 0;
            while at < n {
                let allowed = observer.write_limit(kind, n - at)?.min(n - at);
                if allowed == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero).into());
                }
                if let Some(proof) = &proof {
                    (proof.guard)()?;
                    if witness(output)? != output_witness || witness(input)? != before {
                        return Err(Error::Changed);
                    }
                }
                let written = output.write(&buffer[at..at + allowed])?;
                if written == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero).into());
                }
                at += written;
                if proof.is_some() {
                    output_witness = witness(output)?;
                }
            }
            observer.event(Event::Copied(kind, len))?;
            if let Some(proof) = &proof {
                (proof.guard)()?;
            }
        }
        if before != witness(input)? || len != before.0 {
            return Err(Error::Changed);
        }
        if let Some(proof) = &proof {
            (proof.guard)()?;
            if witness(output)? != output_witness {
                return Err(Error::Changed);
            }
        }
        output.set_len(len)?;
        observer.event(Event::CopyFinished(kind))?;
        if let Some(proof) = &proof {
            (proof.guard)()?;
        }
        Ok(Fingerprint {
            len,
            sha256: hash.finalize().into(),
        })
    }
    fn preallocate(file: &File, len: u64) -> Result<()> {
        if len != 0 {
            rustix::fs::fallocate(file, FallocateFlags::KEEP_SIZE, 0, len)
                .map_err(io::Error::from)?;
        }
        Ok(())
    }
    fn valid_id(id: &str) -> bool {
        id.len() == 34
            && id.starts_with("m-")
            && id[2..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
    fn new_id() -> Result<String> {
        let mut random = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        Ok(format!(
            "m-{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ))
    }
    fn encode(record: &Record) -> Vec<u8> {
        let mut out = b"TRXMEDIA\x01".to_vec();
        out.push(record.phase as u8);
        out.extend(record.reservation.to_le_bytes());
        out.extend(record.identity);
        out.extend(record.directory_identity);
        out.push(record.artifacts.len() as u8);
        for (name, id) in &record.artifacts {
            out.push(name.len() as u8);
            out.extend(name.as_bytes());
            out.extend(id);
        }
        out.extend(record.before.len.to_le_bytes());
        out.extend(record.before.sha256);
        out.push(u8::from(record.after.is_some()));
        let after = record.after.unwrap_or(Fingerprint {
            len: 0,
            sha256: [0; 32],
        });
        out.extend(after.len.to_le_bytes());
        out.extend(after.sha256);
        for value in [&record.id, &record.logical, &record.hint] {
            out.extend((value.len() as u32).to_le_bytes());
            out.extend(value.as_bytes());
        }
        let hash = Sha256::digest(&out);
        out.extend(hash);
        out
    }
    fn decode(bytes: &[u8]) -> Result<Record> {
        if bytes.len() > MANIFEST_LIMIT || bytes.len() < 9 + 1 + 8 + 32 + 40 + 1 + 40 + 12 + 32 {
            return Err(Error::Invalid("manifest length"));
        }
        let (body, checksum) = bytes.split_at(bytes.len() - 32);
        if Sha256::digest(body).as_slice() != checksum || &body[..9] != b"TRXMEDIA\x01" {
            return Err(Error::Invalid("manifest version/checksum"));
        }
        let mut rest = &body[10..];
        fn take<'a>(rest: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
            let (a, b) = rest
                .split_at_checked(len)
                .ok_or(Error::Invalid("truncated manifest"))?;
            *rest = b;
            Ok(a)
        }
        fn number(rest: &mut &[u8]) -> Result<u64> {
            Ok(u64::from_le_bytes(
                take(rest, 8)?
                    .try_into()
                    .map_err(|_| Error::Invalid("manifest number"))?,
            ))
        }
        fn image(rest: &mut &[u8]) -> Result<Fingerprint> {
            Ok(Fingerprint {
                len: number(rest)?,
                sha256: take(rest, 32)?
                    .try_into()
                    .map_err(|_| Error::Invalid("manifest hash"))?,
            })
        }
        let phase = *PHASES
            .get(body[9] as usize)
            .ok_or(Error::Invalid("manifest phase"))?;
        let reservation = number(&mut rest)?;
        let identity = take(&mut rest, 32)?
            .try_into()
            .map_err(|_| Error::Invalid("manifest identity"))?;
        let directory_identity = take(&mut rest, 32)?
            .try_into()
            .map_err(|_| Error::Invalid("manifest directory identity"))?;
        let count = take(&mut rest, 1)?[0] as usize;
        if count > 7 {
            return Err(Error::Invalid("manifest artifact count"));
        }
        let mut artifacts = Vec::new();
        for _ in 0..count {
            let len = take(&mut rest, 1)?[0] as usize;
            let name = std::str::from_utf8(take(&mut rest, len)?)
                .map_err(|_| Error::Invalid("artifact name"))?
                .to_string();
            if artifacts.iter().any(|(old, _)| *old == name) {
                return Err(Error::Invalid("duplicate artifact"));
            }
            let id = take(&mut rest, 32)?
                .try_into()
                .map_err(|_| Error::Invalid("artifact identity"))?;
            artifacts.push((name, id));
        }
        let before = image(&mut rest)?;
        let has_after = take(&mut rest, 1)?[0];
        if has_after > 1 {
            return Err(Error::Invalid("manifest optional image"));
        }
        let after = image(&mut rest)?;
        let mut string = || -> Result<String> {
            let len = u32::from_le_bytes(
                take(&mut rest, 4)?
                    .try_into()
                    .map_err(|_| Error::Invalid("manifest string length"))?,
            ) as usize;
            Ok(std::str::from_utf8(take(&mut rest, len)?)
                .map_err(|_| Error::Invalid("manifest text"))?
                .to_string())
        };
        let id = string()?;
        let logical = string()?;
        let hint = string()?;
        if !rest.is_empty()
            || !valid_id(&id)
            || logical.len() > 4096
            || logical.contains('\0')
            || !Path::new(&logical).is_absolute()
            || Path::new(&logical)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !valid_hint(&hint)
            || reservation < OVERHEAD
            || phase >= Phase::Prepared && has_after != 1
        {
            return Err(Error::Invalid("manifest fields"));
        }
        let expected = if phase == Phase::Preparing {
            1
        } else {
            phase as usize + 3
        };
        if artifacts.len() != expected
            || artifacts.iter().any(|(name, _)| {
                name != "preimage"
                    && name != &format!("candidate.{hint}")
                    && !PHASES[..=phase as usize].iter().any(|p| name == p.name())
            })
            || !PHASES[..=phase as usize]
                .iter()
                .all(|phase| artifacts.iter().any(|(name, _)| name == phase.name()))
            || phase >= Phase::Prepared
                && (!artifacts.iter().any(|(name, _)| name == "preimage")
                    || !artifacts
                        .iter()
                        .any(|(name, _)| name == &format!("candidate.{hint}")))
        {
            return Err(Error::Invalid("manifest artifact namespace"));
        }
        let required = before
            .len
            .checked_add(if has_after == 1 {
                after.len
            } else {
                before.len
            })
            .and_then(|n| n.checked_add(OVERHEAD))
            .ok_or(Error::Invalid("manifest reservation overflow"))?;
        if required > reservation
            || phase == Phase::Preparing && (has_after != 0 || before.sha256 != [0; 32])
        {
            return Err(Error::Invalid("manifest image reservation"));
        }
        Ok(Record {
            id,
            phase,
            logical,
            hint,
            identity,
            directory_identity,
            artifacts,
            reservation,
            before,
            after: (has_after == 1).then_some(after),
        })
    }
    fn valid_hint(hint: &str) -> bool {
        !hint.is_empty() && hint.len() <= 16 && hint.bytes().all(|b| b.is_ascii_alphanumeric())
    }
    fn manifest(dir: &File, name: &str) -> Result<Record> {
        let file = artifact(dir, name)?;
        let mut bytes = Vec::new();
        file.take(MANIFEST_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)?;
        decode(&bytes)
    }
    fn tombstone(id: &str) -> String {
        format!("t-{}", &id[2..])
    }
    fn validate_inventory(dir: &File, record: &Record, partial: bool) -> Result<()> {
        private(dir, true)?;
        require_protected(dir)?;
        if object_identity(dir)? != record.directory_identity {
            return Err(Error::Changed);
        }
        let mut count = 0;
        let mut bytes = 0u64;
        for entry in std::fs::read_dir(fd_path(dir))? {
            count += 1;
            if count > 7 {
                return Err(Error::Invalid("excess artifacts"));
            }
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| Error::Invalid("artifact name"))?;
            let file = artifact(dir, &name)?;
            let expected = record.artifacts.iter().find(|(known, _)| *known == name);
            if let Some((_, expected)) = expected {
                if identity(&file)? != *expected {
                    return Err(Error::Changed);
                }
            } else if record.phase != Phase::Preparing
                || (name != "preimage" && name != format!("candidate.{}", record.hint))
            {
                return Err(Error::Invalid("unowned artifact"));
            }
            bytes = bytes
                .checked_add(file.metadata()?.len())
                .filter(|n| *n <= record.reservation)
                .ok_or(Error::Capacity)?;
        }
        if !partial && record.phase >= Phase::Prepared && count != record.artifacts.len() {
            return Err(Error::Invalid("missing artifact"));
        }
        Ok(())
    }
    fn read_record(dir: &File, id: &str) -> Result<Record> {
        private(dir, true)?;
        let mut names = Vec::new();
        for item in std::fs::read_dir(fd_path(dir))? {
            let item = item?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| Error::Invalid("artifact name"))?;
            if names.len() >= 7 {
                return Err(Error::Invalid("too many artifacts"));
            }
            names.push(name);
        }
        let mut latest: Option<Record> = None;
        for phase in PHASES {
            if !names.iter().any(|n| n == phase.name()) {
                if names.iter().any(|n| {
                    PHASES
                        .iter()
                        .skip(phase as usize + 1)
                        .any(|later| n == later.name())
                }) {
                    return Err(Error::Invalid("missing manifest predecessor"));
                }
                break;
            }
            let file = artifact(dir, phase.name())?;
            let mut bytes = Vec::new();
            file.take(MANIFEST_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)?;
            let record = decode(&bytes)?;
            if record.id != id || record.phase != phase {
                return Err(Error::Invalid("manifest identity/phase"));
            }
            if let Some(previous) = &latest {
                if record.logical != previous.logical
                    || record.identity != previous.identity
                    || record.hint != previous.hint
                    || record.reservation != previous.reservation
                    || phase > Phase::Prepared
                        && (record.before != previous.before || record.after != previous.after)
                    || previous
                        .artifacts
                        .iter()
                        .any(|known| !record.artifacts.contains(known))
                {
                    return Err(Error::Invalid("manifest chain"));
                }
            }
            latest = Some(record);
        }
        let latest = latest.ok_or(Error::Invalid("missing manifest"))?;
        let candidate = format!("candidate.{}", latest.hint);
        for name in names {
            if name != "preimage"
                && name != candidate
                && !PHASES.iter().any(|phase| name == phase.name())
            {
                return Err(Error::Invalid("unknown artifact"));
            }
        }
        validate_inventory(dir, &latest, false)?;
        Ok(latest)
    }
    /// A held file plus a held parent and final-entry identity. Construction
    /// validates correspondence; this is not authorization to leave approved roots.
    pub struct BoundMedia {
        file: File,
        parent: File,
        logical: PathBuf,
        name: std::ffi::OsString,
        identity: [u8; 32],
        attributes: (u32, u32, u32),
        lease: Arc<MediaLock>,
    }
    struct MediaLock(File);
    impl Drop for MediaLock {
        fn drop(&mut self) {
            let _ = self.0.unlock();
        }
    }
    impl BoundMedia {
        fn duplicate(&self) -> Result<Self> {
            Ok(Self {
                file: self.file.try_clone()?,
                parent: self.parent.try_clone()?,
                logical: self.logical.clone(),
                name: self.name.clone(),
                identity: self.identity,
                attributes: self.attributes,
                lease: self.lease.clone(),
            })
        }
        pub fn open(path: &Path) -> Result<Self> {
            let file = File::from(open(path, flags(true), Mode::empty())?);
            Self::from_file(file, path)
        }
        pub(crate) fn held_file(&self) -> io::Result<File> {
            self.file.try_clone()
        }
        pub fn from_file(file: File, logical: &Path) -> Result<Self> {
            let options = rustix::fs::fcntl_getfl(&file)?;
            if options & OFlags::ACCMODE != OFlags::RDWR || options.contains(OFlags::APPEND) {
                return Err(Error::Invalid(
                    "original capability must be read/write without append",
                ));
            }
            let metadata = file.metadata()?;
            if metadata.mode() & 0o222 == 0 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied).into());
            }
            let attributes = (metadata.mode(), metadata.uid(), metadata.gid());
            if !logical.is_absolute()
                || logical.to_str().is_none_or(|p| p.len() > 4096)
                || logical
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(Error::Invalid("original needs an absolute Unicode path"));
            }
            let parent = directory(logical.parent().ok_or(Error::Invalid("original parent"))?)?;
            let name = logical
                .file_name()
                .ok_or(Error::Invalid("original name"))?
                .to_os_string();
            let current = File::from(openat(&parent, &name, flags(false), Mode::empty())?);
            if !same(&file, &current)? {
                return Err(Error::Changed);
            }
            let identity = identity(&file)?;
            // A new open description prevents two caller-provided clones from
            // accidentally sharing one flock owner (and unlocking each other).
            let lock_file = File::from(openat(&parent, &name, flags(true), Mode::empty())?);
            if !same(&file, &lock_file)? {
                return Err(Error::Changed);
            }
            let file = lock_file.try_clone()?;
            match lock_file.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Capacity),
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
            let lease = Arc::new(MediaLock(lock_file));
            Ok(Self {
                file,
                parent,
                logical: logical.to_path_buf(),
                name,
                identity,
                attributes,
                lease,
            })
        }
        fn validate(&self) -> Result<()> {
            let metadata = self.file.metadata()?;
            if self.attributes != (metadata.mode(), metadata.uid(), metadata.gid()) {
                return Err(Error::Changed);
            }
            let current_parent = directory(self.logical.parent().ok_or(Error::Changed)?)?;
            let current = File::from(openat(
                &self.parent,
                &self.name,
                flags(false),
                Mode::empty(),
            )?);
            if !same(&self.parent, &current_parent)?
                || !same(&self.file, &current)?
                || identity(&self.file)? != self.identity
            {
                return Err(Error::Changed);
            }
            Ok(())
        }
        fn exclude_recovery_objects(&self) -> Result<()> {
            if protected(&self.file)? {
                return Err(Error::Invalid("original is a protected recovery object"));
            }
            let mut parent = self.parent.try_clone()?;
            // Follow physical .. from held directories, never a caller's
            // textual prefix. Markers survive aliases, bind mounts, closed
            // Store handles and other processes; the file marker covers hardlinks.
            for _ in 0..4096 {
                if protected(&parent)? {
                    return Err(Error::Invalid("original is inside recovery storage"));
                }
                let next = child_dir(&parent, OsStr::new(".."))?;
                if same(&parent, &next)? {
                    return Ok(());
                }
                parent = next;
            }
            Err(Error::Unsupported)
        }
    }
    struct Inner {
        path: PathBuf,
        dir: File,
        lock: File,
        entries: Mutex<HashMap<String, u64>>,
        limits: Limits,
        budget: Budget,
        _metadata: Lease,
    }
    #[derive(Clone)]
    pub struct Store(Arc<Inner>);
    struct DynSink<'a>(&'a mut dyn StateSink);
    impl StateSink for DynSink<'_> {
        fn persist(&mut self, record: &Record) -> io::Result<()> {
            self.0.persist(record)
        }
    }
    impl Store {
        /// Existing directories must already be private. This never chmods or
        /// takes ownership of an unrelated existing directory.
        pub fn open(path: &Path, limits: Limits, budget: Budget) -> Result<Self> {
            Self::open_with_parent(path, limits, budget, || {
                directory(path.parent().ok_or(Error::Invalid("store parent"))?)
            })
        }
        /// Open recovery storage relative to a caller-held parent directory.
        /// The parent must still be the logical path's parent before any store
        /// effects and at return. The caller owns profile/root authorization;
        /// this descriptor does not add filesystem authority. A later observed
        /// path replacement refuses operations and preserves recovery evidence.
        pub fn open_in(parent: File, path: &Path, limits: Limits, budget: Budget) -> Result<Self> {
            Self::open_with_parent(path, limits, budget, || Ok(parent))
        }
        fn open_with_parent(
            path: &Path,
            limits: Limits,
            budget: Budget,
            parent: impl FnOnce() -> Result<File>,
        ) -> Result<Self> {
            if limits.entries == 0
                || limits.entries > 4096
                || limits.store_bytes < OVERHEAD
                || limits.media_bytes == 0
                || !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(Error::Invalid("store limits/path"));
            }
            let metadata = budget.try_acquire(Charge {
                retained: limits.entries * MANIFEST_LIMIT,
                ..Default::default()
            })?;
            let parent_path = path.parent().ok_or(Error::Invalid("store parent"))?;
            let name = path.file_name().ok_or(Error::Invalid("store name"))?;
            let parent = parent()?;
            if !parent.metadata()?.is_dir()
                || rustix::fs::fcntl_getfl(&parent)?.contains(OFlags::PATH)
            {
                return Err(Error::Invalid("store parent must be a syncable directory"));
            }
            // Do not adopt a newly selected profile, or touch either namespace,
            // when the caller's pinned parent no longer has this logical name.
            if !same(&parent, &directory(parent_path)?)? {
                return Err(Error::Changed);
            }
            match mkdirat(&parent, name, Mode::from_bits_truncate(0o700)) {
                Ok(()) => parent.sync_all()?,
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(error) => return Err(io::Error::from(error).into()),
            }
            let dir = child_dir(&parent, name)?;
            private(&dir, true)?;
            let _identity = object_identity(&dir)?;
            if !protected(&dir)? {
                if std::fs::read_dir(fd_path(&dir))?.next().is_some() {
                    return Err(Error::Unsupported);
                }
                protect(&dir)?;
                dir.sync_all()?;
                parent.sync_all()?;
            }
            let lock = match create(&dir, "lock") {
                Ok(file) => file,
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let file = File::from(openat(&dir, "lock", flags(true), Mode::empty())?);
                    private(&file, false)?;
                    require_protected(&file)?;
                    file
                }
                Err(error) => return Err(error),
            };
            lock.try_lock().map_err(|_| Error::Capacity)?;
            let mut entries = HashMap::new();
            let mut used = 0u64;
            let mut names = Vec::new();
            for item in std::fs::read_dir(fd_path(&dir))? {
                if names.len() > 2 * limits.entries {
                    return Err(Error::Invalid("excess store namespace"));
                }
                names.push(
                    item?
                        .file_name()
                        .into_string()
                        .map_err(|_| Error::Invalid("store entry name"))?,
                );
            }
            let mut retired = HashMap::new();
            for name in &names {
                if let Some(suffix) = name.strip_prefix("t-") {
                    let id = format!("m-{suffix}");
                    if !valid_id(&id) {
                        return Err(Error::Invalid("retirement name"));
                    }
                    let record = manifest(&dir, name)?;
                    if record.id != id || record.phase != Phase::Retired {
                        return Err(Error::Invalid("retirement proof"));
                    }
                    retired.insert(id, record);
                }
            }
            for name in names {
                if name == "lock" {
                    continue;
                }
                if name.starts_with("t-") {
                    continue;
                }
                if !valid_id(&name) || entries.len() >= limits.entries {
                    return Err(Error::Invalid("unknown/excess store entry"));
                }
                let entry = child_dir(&dir, OsStr::new(&name))?;
                let record = if let Some(record) = retired.remove(&name) {
                    validate_inventory(&entry, &record, true)?;
                    record
                } else {
                    read_record(&entry, &name)?
                };
                used = used
                    .checked_add(record.reservation)
                    .filter(|n| *n <= limits.store_bytes)
                    .ok_or(Error::Capacity)?;
                entries.insert(name, record.reservation);
            }
            for (id, record) in retired {
                if entries.len() >= limits.entries {
                    return Err(Error::Capacity);
                }
                used = used
                    .checked_add(record.reservation)
                    .filter(|n| *n <= limits.store_bytes)
                    .ok_or(Error::Capacity)?;
                entries.insert(id, record.reservation);
            }
            let store = Self(Arc::new(Inner {
                path: path.to_path_buf(),
                dir,
                lock,
                entries: Mutex::new(entries),
                limits,
                budget,
                _metadata: metadata,
            }));
            // Namespace work stays relative to the held parent. If its name
            // changes during initialization, refuse without deleting evidence.
            if !same(&parent, &directory(parent_path)?)? {
                return Err(Error::Changed);
            }
            store.validate()?;
            Ok(store)
        }
        pub fn usage(&self) -> (u64, usize) {
            let entries = self.0.entries.lock().unwrap_or_else(|p| p.into_inner());
            (entries.values().sum(), entries.len())
        }
        fn validate(&self) -> Result<()> {
            private(&self.0.dir, true)?;
            require_protected(&self.0.dir)?;
            if !same(&self.0.dir, &directory(&self.0.path)?)? {
                return Err(Error::Changed);
            }
            if !same(&self.0.lock, &artifact(&self.0.dir, "lock")?)? {
                return Err(Error::Changed);
            }
            Ok(())
        }
        pub fn records(&self) -> Result<Vec<Record>> {
            self.validate()?;
            let entries = self.0.entries.lock().unwrap_or_else(|p| p.into_inner());
            let mut records = Vec::new();
            for id in entries.keys() {
                records.push(self.record(id)?);
            }
            Ok(records)
        }
        fn record(&self, id: &str) -> Result<Record> {
            match manifest(&self.0.dir, &tombstone(id)) {
                Ok(record) if record.id == id && record.phase == Phase::Retired => Ok(record),
                Ok(_) => Err(Error::Invalid("retirement proof")),
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    read_record(&child_dir(&self.0.dir, OsStr::new(id))?, id)
                }
                Err(error) => Err(error),
            }
        }
        /// Resume only a previously confirmed retirement. An immutable root
        /// tombstone survives every partial entry-cleanup step.
        pub fn finish_retirement(
            &self,
            id: &str,
            original: &mut BoundMedia,
            confirmed: impl FnOnce(&Record) -> io::Result<()>,
            observer: &mut dyn Observer,
        ) -> Result<()> {
            let _working = self.0.budget.try_acquire(Charge {
                transient: WORKING_BYTES,
                jobs: 1,
                ..Default::default()
            })?;
            self.validate()?;
            if !valid_id(id) {
                return Err(Error::Invalid("entry id"));
            }
            let record = self.record(id)?;
            if record.phase != Phase::Retired {
                return Err(Error::Invalid("entry is not retired"));
            }
            original.validate()?;
            if original.identity != record.identity
                || original.logical != Path::new(&record.logical)
                || Some(fingerprint(&mut original.file, self.0.limits.media_bytes)?) != record.after
            {
                return Err(Error::Changed);
            }
            confirmed(&record)?;
            original.validate()?;
            if Some(fingerprint(&mut original.file, self.0.limits.media_bytes)?) != record.after {
                return Err(Error::Changed);
            }
            let source = ImageSeal::new(
                original.file.try_clone()?,
                record.after.ok_or(Error::Invalid("candidate proof"))?,
                self.0.limits.media_bytes,
            )?;
            let directory = match child_dir(&self.0.dir, OsStr::new(id)) {
                Ok(dir) => Some(dir),
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let images = if let Some(dir) = &directory {
                validate_inventory(dir, &record, true)?;
                image_seals(dir, &record, self.0.limits.media_bytes, true)?
            } else {
                HashMap::new()
            };
            let original_guard = original.duplicate()?;
            let mut observed = CleanupObserver {
                inner: observer,
                store: self,
                original: &original_guard,
                source: &source,
                dir: directory,
                id,
                images,
                terminal: None,
            };
            observed.validate()?;
            let name = tombstone(id);
            let terminal = match create(&self.0.dir, &name) {
                Ok(mut file) => {
                    file.write_all(&encode(&record))?;
                    // Seal the durable fallback before exposing its first sync
                    // callback. Cleanup may not begin with invalid fallback proof.
                    observed.bind_terminal(&name, &file, &record)?;
                    sync(&file, SyncKind::Manifest(Phase::Retired), &mut observed)?;
                    sync(&self.0.dir, SyncKind::Directory, &mut observed)?;
                    file
                }
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let file = artifact(&self.0.dir, &name)?;
                    if manifest(&self.0.dir, &name)? != record {
                        return Err(Error::Changed);
                    }
                    file
                }
                Err(error) => return Err(error),
            };
            if observed.terminal.is_none() {
                observed.bind_terminal(&name, &terminal, &record)?;
            }
            observed.validate()?;
            match child_dir(&self.0.dir, OsStr::new(id)) {
                Ok(dir) => {
                    validate_inventory(&dir, &record, true)?;
                    for (index, (name, expected)) in record.artifacts.iter().enumerate() {
                        match artifact(&dir, name) {
                            Ok(file) => {
                                if identity(&file)? != *expected {
                                    return Err(Error::Changed);
                                }
                                unlinkat(&dir, name.as_str(), AtFlags::empty())?;
                                observed.event(Event::ArtifactRemoved(index as u8))?;
                            }
                            Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                            Err(error) => return Err(error),
                        }
                    }
                    sync(&dir, SyncKind::Directory, &mut observed)?;
                    if object_identity(&child_dir(&self.0.dir, OsStr::new(id))?)?
                        != record.directory_identity
                    {
                        return Err(Error::Changed);
                    }
                    unlinkat(&self.0.dir, OsStr::new(id), AtFlags::REMOVEDIR)?;
                    sync(&self.0.dir, SyncKind::Directory, &mut observed)?;
                }
                Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if !same(&terminal, &artifact(&self.0.dir, &name)?)? {
                return Err(Error::Changed);
            }
            observed.validate()?;
            unlinkat(&self.0.dir, name.as_str(), AtFlags::empty())?;
            observed.terminal.take();
            sync(&self.0.dir, SyncKind::Directory, &mut observed)?;
            self.0
                .entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(id);
            Ok(())
        }
        /// Reserve both complete images and maximum candidate growth before
        /// invoking an existing writer. The callback is trusted native code;
        /// its allocations and growth must obey its own admission contract.
        pub fn prepare(
            &self,
            original: BoundMedia,
            extension: &str,
            growth: u64,
            sink: &mut dyn StateSink,
            observer: &mut dyn Observer,
            edit: impl FnOnce(&Path) -> io::Result<()>,
        ) -> Result<Prepared> {
            self.prepare_internal(
                (None, extension, None),
                original,
                growth,
                &mut DynSink(sink),
                observer,
                |_, path, _| edit(path),
            )
        }
        /// Edit the held disposable candidate with the same typed sink later
        /// used to durably acknowledge Prepared. This permits a journal driver
        /// to capture its selective receipt during editing and persist it before
        /// publication without overlapping mutable borrows of that driver.
        ///
        /// The cursor is unspecified; editors must seek as needed. The format
        /// hint is supplied separately and never changes the held File's native
        /// identity. A replaced private artifact name refuses preparation even
        /// when the editor successfully changed its original held object.
        /// Errors/unwinds before publication preserve the original; editors must
        /// bound their own allocations, growth and cooperative cancellation.
        pub fn prepare_file<S: StateSink>(
            &self,
            original: BoundMedia,
            extension: &str,
            growth: u64,
            sink: &mut S,
            observer: &mut dyn Observer,
            edit: impl FnOnce(&mut File, &mut S) -> io::Result<()>,
        ) -> Result<Prepared> {
            self.prepare_internal(
                (None, extension, None),
                original,
                growth,
                sink,
                observer,
                |candidate, _, sink| edit(candidate, sink),
            )
        }
        pub(crate) fn prepare_file_id<S: StateSink>(
            &self,
            binding: (&str, &str, &std::sync::atomic::AtomicBool),
            original: BoundMedia,
            growth: u64,
            sink: &mut S,
            observer: &mut dyn Observer,
            edit: impl FnOnce(&mut File, &mut S) -> io::Result<()>,
        ) -> Result<Prepared> {
            self.prepare_internal(
                (Some(binding.0), binding.1, Some(binding.2)),
                original,
                growth,
                sink,
                observer,
                |candidate, _, sink| edit(candidate, sink),
            )
        }
        fn prepare_internal<S: StateSink>(
            &self,
            binding: (Option<&str>, &str, Option<&std::sync::atomic::AtomicBool>),
            mut original: BoundMedia,
            growth: u64,
            sink: &mut S,
            observer: &mut dyn Observer,
            edit: impl FnOnce(&mut File, &Path, &mut S) -> io::Result<()>,
        ) -> Result<Prepared> {
            let (requested_id, extension, cancel) = binding;
            self.validate()?;
            original.validate()?;
            original.exclude_recovery_objects()?;
            if original.logical.starts_with(&self.0.path) {
                return Err(Error::Invalid("original cannot be a recovery artifact"));
            }
            if !valid_hint(extension) {
                return Err(Error::Invalid("format extension hint"));
            }
            let len = original.file.metadata()?.len();
            if len > self.0.limits.media_bytes {
                return Err(Error::Capacity);
            }
            let maximum = len
                .checked_add(growth)
                .filter(|n| *n <= self.0.limits.media_bytes)
                .ok_or(Error::Capacity)?;
            let reservation = len
                .checked_add(maximum)
                .and_then(|n| n.checked_add(OVERHEAD))
                .ok_or(Error::Capacity)?;
            // Only the Executor-owned held-file adapter waits for initial
            // admission. No directory, candidate, callback or media effect has
            // happened yet; all public primitive APIs remain nonblocking.
            let started = std::time::Instant::now();
            let working = loop {
                if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst)) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "native preparation cancelled",
                    )
                    .into());
                }
                match self.0.budget.try_acquire(Charge {
                    transient: WORKING_BYTES,
                    jobs: 1,
                    ..Default::default()
                }) {
                    Err(crate::resources::AdmissionError::Busy)
                        if cancel.is_some()
                            && started.elapsed() < std::time::Duration::from_millis(500) =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    result => break result?,
                }
            };
            let id = match requested_id {
                Some(id) if valid_id(id) => id.to_owned(),
                Some(_) => return Err(Error::Invalid("operation identity")),
                None => new_id()?,
            };
            {
                let mut entries = self.0.entries.lock().unwrap_or_else(|p| p.into_inner());
                let used: u64 = entries.values().sum();
                if entries.len() >= self.0.limits.entries
                    || entries.contains_key(&id)
                    || used
                        .checked_add(reservation)
                        .is_none_or(|n| n > self.0.limits.store_bytes)
                {
                    return Err(Error::Capacity);
                }
                entries.insert(id.clone(), reservation);
            }
            if let Err(error) = mkdirat(
                &self.0.dir,
                OsStr::new(&id),
                Mode::from_bits_truncate(0o700),
            ) {
                self.0
                    .entries
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                return Err(io::Error::from(error).into());
            }
            let dir = child_dir(&self.0.dir, OsStr::new(&id))?;
            protect(&dir)?;
            let record = Record {
                id,
                phase: Phase::Preparing,
                logical: original
                    .logical
                    .to_str()
                    .ok_or(Error::Invalid("original path"))?
                    .into(),
                hint: extension.into(),
                identity: original.identity,
                directory_identity: object_identity(&dir)?,
                artifacts: Vec::new(),
                reservation,
                before: Fingerprint {
                    len,
                    sha256: [0; 32],
                },
                after: None,
            };
            let mut prepared = Prepared {
                store: self.clone(),
                dir,
                record,
                original,
                working: Some(working),
                intent: false,
                owned: HashMap::new(),
            };
            prepared.state(Phase::Preparing, None, observer)?;
            observer.event(Event::EntryCreated)?;
            let initial = witness(&prepared.original.file)?;
            let mut preimage = prepared.create_owned("preimage")?;
            preallocate(&preimage, len)?;
            let before = copy(
                &mut prepared.original.file,
                &mut preimage,
                len,
                CopyKind::Preimage,
                observer,
                None,
            )?;
            sync(&preimage, SyncKind::Preimage, observer)?;
            if initial != witness(&prepared.original.file)? {
                return Err(Error::Changed);
            }
            let candidate_name = prepared.candidate_name();
            let mut candidate = prepared.create_owned(&candidate_name)?;
            preallocate(&candidate, maximum)?;
            if copy(
                &mut preimage,
                &mut candidate,
                len,
                CopyKind::Candidate,
                observer,
                None,
            )? != before
            {
                return Err(Error::Changed);
            }
            sync(&candidate, SyncKind::Candidate, observer)?;
            prepared.validate_entry()?;
            observer.event(Event::BeforeEdit)?;
            edit(
                &mut candidate,
                &fd_path(&prepared.dir).join(&candidate_name),
                sink,
            )?;
            // Fix the editor's output witness before any further callback can
            // change it. Do not promote callback-authored bytes into a newly
            // established Prepared fingerprint or a captured selective receipt.
            let edited = witness(&candidate)?;
            observer.event(Event::AfterEdit)?;
            if witness(&candidate)? != edited {
                return Err(Error::Changed);
            }
            let mut current = artifact(&prepared.dir, &candidate_name)?;
            if !same(&candidate, &current)? {
                return Err(Error::Changed);
            }
            let after = fingerprint(&mut current, maximum)?;
            sync(&candidate, SyncKind::Candidate, observer)?;
            original = prepared.original_clone()?;
            original.validate()?;
            if fingerprint(&mut original.file, self.0.limits.media_bytes)? != before {
                return Err(Error::Changed);
            }
            prepared.record.before = before;
            prepared.record.after = Some(after);
            prepared.state(Phase::Prepared, Some(sink), observer)?;
            prepared.validate_entry()?;
            if witness(&candidate)? != edited
                || !same(&candidate, &artifact(&prepared.dir, &candidate_name)?)?
            {
                return Err(Error::Changed);
            }
            Ok(prepared)
        }
        /// Bounded inspection only. A different identity or unmatched bytes
        /// never authorizes overwrite/deletion or automatic full-file restore.
        pub fn inspect(&self, id: &str, original: &mut BoundMedia) -> Result<Current> {
            let _working = self.0.budget.try_acquire(Charge {
                transient: WORKING_BYTES,
                jobs: 1,
                ..Default::default()
            })?;
            self.validate()?;
            if !valid_id(id) {
                return Err(Error::Invalid("entry id"));
            }
            let dir = child_dir(&self.0.dir, OsStr::new(id))?;
            let record = read_record(&dir, id)?;
            original.validate()?;
            if original.identity != record.identity
                || original.logical != Path::new(&record.logical)
            {
                return Ok(Current::DifferentIdentity);
            }
            if record.phase >= Phase::Prepared
                && (fingerprint(&mut artifact(&dir, "preimage")?, self.0.limits.media_bytes)?
                    != record.before
                    || fingerprint(
                        &mut artifact(&dir, &format!("candidate.{}", record.hint))?,
                        self.0.limits.media_bytes,
                    )? != record.after.ok_or(Error::Invalid("candidate proof"))?)
            {
                return Err(Error::Changed);
            }
            let current = fingerprint(&mut original.file, self.0.limits.media_bytes)?;
            if record.phase >= Phase::Prepared && current == record.before {
                Ok(Current::Preimage)
            } else if record.after == Some(current) {
                Ok(Current::Candidate)
            } else {
                Ok(Current::Ambiguous)
            }
        }
        /// Executor recovery may discard only a proved, unchanged preimage.
        /// All record/image/native namespace proofs are checked before cleanup.
        pub(crate) fn discard_preimage(&self, id: &str, mut original: BoundMedia) -> Result<()> {
            if self.inspect(id, &mut original)? != Current::Preimage {
                return Err(Error::Changed);
            }
            let record = self.record(id)?;
            let dir = child_dir(&self.0.dir, OsStr::new(id))?;
            validate_inventory(&dir, &record, false)?;
            let mut owned = HashMap::new();
            for (name, _) in &record.artifacts {
                owned.insert(name.clone(), artifact(&dir, name)?);
            }
            let mut prepared = Prepared {
                store: self.clone(),
                dir,
                record,
                original,
                working: None,
                intent: true,
                owned,
            };
            // intent=true keeps Drop non-destructive if explicit cleanup fails.
            prepared.original.validate()?;
            if fingerprint(&mut prepared.original.file, self.0.limits.media_bytes)?
                != prepared.record.before
            {
                return Err(Error::Changed);
            }
            prepared.cleanup()
        }
        /// Reconcile a published image after restart. This never republishes
        /// bytes; retirement still requires explicit durable-finish confirmation.
        pub fn resume_published(&self, id: &str, mut original: BoundMedia) -> Result<Published> {
            if !valid_id(id) {
                return Err(Error::Invalid("entry id"));
            }
            let record = self.record(id)?;
            if record.phase != Phase::Published
                || self.inspect(id, &mut original)? != Current::Candidate
            {
                return Err(Error::Changed);
            }
            let dir = child_dir(&self.0.dir, OsStr::new(id))?;
            let mut owned = HashMap::new();
            for (name, _) in &record.artifacts {
                owned.insert(name.clone(), artifact(&dir, name)?);
            }
            Ok(Published(Prepared {
                store: self.clone(),
                dir,
                record,
                original,
                working: None,
                intent: true,
                owned,
            }))
        }
    }
    /// Dropping before any Publishing intent cleans only validated artifacts.
    /// Once intent is attempted, Drop retains everything, even on panic/error.
    pub struct Prepared {
        store: Store,
        dir: File,
        record: Record,
        original: BoundMedia,
        working: Option<Lease>,
        intent: bool,
        owned: HashMap<String, File>,
    }
    impl Prepared {
        pub fn record(&self) -> &Record {
            &self.record
        }
        fn candidate_name(&self) -> String {
            format!("candidate.{}", self.record.hint)
        }
        fn create_owned(&mut self, name: &str) -> Result<File> {
            let file = create(&self.dir, name)?;
            self.owned.insert(name.into(), file.try_clone()?);
            Ok(file)
        }
        fn original_clone(&self) -> Result<BoundMedia> {
            self.original.duplicate()
        }
        fn validate_entry(&self) -> Result<()> {
            self.store.validate()?;
            private(&self.dir, true)?;
            require_protected(&self.dir)?;
            if !same(
                &self.dir,
                &child_dir(&self.store.0.dir, OsStr::new(&self.record.id))?,
            )? {
                return Err(Error::Changed);
            }
            Ok(())
        }
        fn state(
            &mut self,
            phase: Phase,
            sink: Option<&mut dyn StateSink>,
            observer: &mut dyn Observer,
        ) -> Result<()> {
            self.validate_entry()?;
            if phase >= Phase::Publishing {
                self.intent = true;
            }
            let mut next = self.record.clone();
            next.phase = phase;
            let mut manifest = self.create_owned(phase.name())?;
            next.artifacts = self
                .owned
                .iter()
                .map(|(name, file)| Ok((name.clone(), identity(file)?)))
                .collect::<Result<Vec<_>>>()?;
            next.artifacts.sort_by(|a, b| a.0.cmp(&b.0));
            let bytes = encode(&next);
            if bytes.len() > MANIFEST_LIMIT {
                return Err(Error::Invalid("manifest too large"));
            }
            manifest.write_all(&bytes)?;
            sync(&manifest, SyncKind::Manifest(phase), observer)?;
            sync(&self.dir, SyncKind::Directory, observer)?;
            sync(&self.store.0.dir, SyncKind::Directory, observer)?;
            self.record = next;
            observer.event(Event::StateDurable(phase))?;
            if let Some(sink) = sink {
                observer.event(Event::BeforeCallback(phase))?;
                sink.persist(&self.record)?;
                observer.event(Event::AfterCallback(phase))?;
            }
            Ok(())
        }
        pub fn publish(
            mut self,
            sink: &mut dyn StateSink,
            observer: &mut dyn Observer,
        ) -> Result<Published> {
            self.validate_entry()?;
            self.original.validate()?;
            self.original.exclude_recovery_objects()?;
            if read_record(&self.dir, &self.record.id)? != self.record {
                return Err(Error::Changed);
            }
            let before = self.record.before;
            let after = self.record.after.ok_or(Error::Invalid("candidate proof"))?;
            let maximum = self.store.0.limits.media_bytes;
            let preimage = ImageSeal::new(artifact(&self.dir, "preimage")?, before, maximum)?;
            let candidate_seal =
                ImageSeal::new(artifact(&self.dir, &self.candidate_name())?, after, maximum)?;
            let original_guard = self.original_clone()?;
            let original_before = ImageSeal::new(self.original.file.try_clone()?, before, maximum)?;
            let store = self.store.clone();
            let dir = self.dir.try_clone()?;
            let id = self.record.id.clone();
            let candidate_name = self.candidate_name();
            let base = || {
                store.validate()?;
                private(&dir, true)?;
                require_protected(&dir)?;
                if !same(&dir, &child_dir(&store.0.dir, OsStr::new(&id))?)? {
                    return Err(Error::Changed);
                }
                original_guard.validate()?;
                original_guard.exclude_recovery_objects()?;
                preimage.validate()?;
                candidate_seal.validate()?;
                // Named objects must still be those whose contents were sealed.
                if !same(&preimage.file, &artifact(&dir, "preimage")?)?
                    || !same(&candidate_seal.file, &artifact(&dir, &candidate_name)?)?
                {
                    return Err(Error::Changed);
                }
                Ok(())
            };
            let prewrite = || {
                base()?;
                original_before.validate()
            };
            {
                let mut observed = CheckedObserver {
                    inner: observer,
                    guard: &prewrite,
                };
                let mut acknowledged = CheckedSink {
                    inner: sink,
                    guard: &prewrite,
                };
                self.state(Phase::Publishing, Some(&mut acknowledged), &mut observed)?;
            }
            base()?;
            if read_record(&self.dir, &id)? != self.record {
                return Err(Error::Changed);
            }
            // Allocation may change the original's metadata. Re-prove its
            // expected bytes afterwards; image witnesses remain immutable.
            preallocate(&self.original.file, after.len)?;
            let allocated_original =
                ImageSeal::new(self.original.file.try_clone()?, before, maximum)?;
            base()?;
            let publishing = self.record.clone();
            let streaming = || {
                base()?;
                if read_record(&dir, &id)? != publishing {
                    return Err(Error::Changed);
                }
                Ok(())
            };
            let mut candidate = candidate_seal.file.try_clone()?;
            let mut output = self.original.file.try_clone()?;
            if copy(
                &mut candidate,
                &mut output,
                after.len,
                CopyKind::Publish,
                observer,
                Some(CopyProof {
                    guard: &streaming,
                    input: candidate_seal.witness,
                    output: allocated_original.witness,
                }),
            )? != after
            {
                return Err(Error::Changed);
            }
            base()?;
            let original_after = ImageSeal::new(self.original.file.try_clone()?, after, maximum)?;
            let postwrite = || {
                base()?;
                original_after.validate()
            };
            {
                let mut observed = CheckedObserver {
                    inner: observer,
                    guard: &postwrite,
                };
                sync(&self.original.file, SyncKind::Original, &mut observed)?;
                let mut acknowledged = CheckedSink {
                    inner: sink,
                    guard: &postwrite,
                };
                self.state(Phase::Published, Some(&mut acknowledged), &mut observed)?;
            }
            postwrite()?;
            if read_record(&self.dir, &id)? != self.record {
                return Err(Error::Changed);
            }
            self.working.take();
            Ok(Published(self))
        }
        fn cleanup(&mut self) -> Result<()> {
            self.validate_entry()?;
            // Refuse replacement links/hardlinks/unknown entries. Never use
            // remove_dir_all on a mutable artifact namespace.
            let mut names = Vec::new();
            for item in std::fs::read_dir(fd_path(&self.dir))? {
                let name = item?
                    .file_name()
                    .into_string()
                    .map_err(|_| Error::Invalid("cleanup name"))?;
                if names.len() >= 7
                    || (name != "preimage"
                        && name != self.candidate_name()
                        && !PHASES.iter().any(|p| name == p.name()))
                {
                    return Err(Error::Invalid("cleanup artifact"));
                }
                let file = artifact(&self.dir, &name)?;
                if !self
                    .owned
                    .get(&name)
                    .is_some_and(|owned| same(owned, &file).unwrap_or(false))
                {
                    return Err(Error::Changed);
                }
                names.push(name);
            }
            for name in names {
                unlinkat(&self.dir, name.as_str(), AtFlags::empty()).map_err(io::Error::from)?;
            }
            self.dir.sync_all()?;
            unlinkat(
                &self.store.0.dir,
                OsStr::new(&self.record.id),
                AtFlags::REMOVEDIR,
            )
            .map_err(io::Error::from)?;
            self.store.0.dir.sync_all()?;
            self.store
                .0
                .entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.record.id);
            Ok(())
        }
    }
    impl Drop for Prepared {
        fn drop(&mut self) {
            if !self.intent {
                let _ = self.cleanup();
            }
        }
    }
    pub struct Published(Prepared);
    impl Published {
        pub fn record(&self) -> &Record {
            &self.0.record
        }
        /// Confirmation must verify the caller's durable journal finish. The
        /// primitive separately revalidates original and both image contents.
        pub fn retire(
            mut self,
            confirmed: impl FnOnce(&Record) -> io::Result<()>,
            sink: &mut dyn StateSink,
            observer: &mut dyn Observer,
        ) -> Result<()> {
            observer.event(Event::BeforeRetire)?;
            confirmed(&self.0.record)?;
            self.0.validate_entry()?;
            self.0.original.validate()?;
            let maximum = self.0.store.0.limits.media_bytes;
            self.0.working = Some(self.0.store.0.budget.try_acquire(Charge {
                transient: WORKING_BYTES,
                jobs: 1,
                ..Default::default()
            })?);
            let preimage = ImageSeal::new(
                artifact(&self.0.dir, "preimage")?,
                self.0.record.before,
                maximum,
            )?;
            let expected = self
                .0
                .record
                .after
                .ok_or(Error::Invalid("candidate proof"))?;
            let candidate_name = self.0.candidate_name();
            let candidate =
                ImageSeal::new(artifact(&self.0.dir, &candidate_name)?, expected, maximum)?;
            let source = ImageSeal::new(self.0.original.file.try_clone()?, expected, maximum)?;
            let store = self.0.store.clone();
            let dir = self.0.dir.try_clone()?;
            let id = self.0.record.id.clone();
            let original = self.0.original_clone()?;
            let guard = || {
                store.validate()?;
                original.validate()?;
                source.validate()?;
                preimage.validate()?;
                candidate.validate()?;
                if !same(&dir, &child_dir(&store.0.dir, OsStr::new(&id))?)?
                    || !same(&preimage.file, &artifact(&dir, "preimage")?)?
                    || !same(&candidate.file, &artifact(&dir, &candidate_name)?)?
                {
                    return Err(Error::Changed);
                }
                Ok(())
            };
            {
                let mut observed = CheckedObserver {
                    inner: observer,
                    guard: &guard,
                };
                let mut acknowledged = CheckedSink {
                    inner: sink,
                    guard: &guard,
                };
                self.0
                    .state(Phase::Retired, Some(&mut acknowledged), &mut observed)?;
            }
            guard()?;
            self.0.working.take();
            self.0.store.finish_retirement(
                &self.0.record.id,
                &mut self.0.original,
                |_| Ok(()),
                observer,
            )
        }
    }
}
#[cfg(target_os = "linux")]
pub(crate) use linux::{file_identity, operation_id};
#[cfg(target_os = "linux")]
pub use linux::{BoundMedia, Prepared, Published, Store};

#[cfg(not(target_os = "linux"))]
pub struct Store;
#[cfg(not(target_os = "linux"))]
impl Store {
    pub fn open(_path: &Path, _limits: Limits, _budget: crate::resources::Budget) -> Result<Self> {
        Err(Error::Unsupported)
    }
    pub fn open_in(
        _parent: std::fs::File,
        _path: &Path,
        _limits: Limits,
        _budget: crate::resources::Budget,
    ) -> Result<Self> {
        Err(Error::Unsupported)
    }
}
