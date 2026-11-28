//! Persistent undo journal.
//!
//! The one place where extra complexity is bought up front (architecture.md):
//! the journal must survive application restarts, record both tag writes and
//! renames, and allow rollback of a whole batch as a unit. The motivating
//! scenario: "renamed 8,000 files, closed the app, realized in the morning
//! the mask was wrong."

use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model::{
    CoverArt, CoverKind, Id3v2Revision, TagBlockContent, TagBlockKind, TagField, TagMap,
};
use crate::plan::{BlockChange, ChangePlan, CoverChange, FieldChange, FileChange};

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod native;

#[cfg(target_os = "linux")]
type NativeRecordIntent = native::Intent;
#[cfg(not(target_os = "linux"))]
type NativeRecordIntent = ();

fn stored_path(value: rusqlite::types::ValueRef<'_>) -> Result<(PathBuf, bool), rusqlite::Error> {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(|s| (PathBuf::from(s), false))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            }),
        ValueRef::Blob(bytes) => {
            let path = bytes
                .strip_prefix(b"TRX-NATIVE-PATH\x01")
                .filter(|p| p.len() <= 4096)
                .and_then(|p| std::str::from_utf8(p).ok())
                .filter(|s| !s.contains('\0'))
                .map(PathBuf::from)
                .filter(|p| {
                    p.is_absolute()
                        && !p
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                })
                .ok_or(rusqlite::Error::InvalidQuery)?;
            Ok((path, true))
        }
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

/// One load owns one image pool. Reference rows never select the BLOB again;
/// legacy inline rows are interned after reading so their plans also share data.
#[derive(Default)]
struct LoadedImages {
    by_id: std::collections::HashMap<i64, std::sync::Arc<[u8]>>,
    inline: std::collections::HashSet<std::sync::Arc<[u8]>>,
}
impl LoadedImages {
    fn intern(&mut self, bytes: Vec<u8>) -> std::sync::Arc<[u8]> {
        if let Some(existing) = self.inline.get(bytes.as_slice()) {
            return existing.clone();
        }
        let shared: std::sync::Arc<[u8]> = bytes.into();
        self.inline.insert(shared.clone());
        shared
    }
}

/// Identifier of an applied batch, stable across restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchId(pub i64);

/// History metadata with source paths only. Listing this never hydrates tag,
/// cover or block snapshots; those are fetched only for an actual Undo.
#[derive(Debug, Clone)]
pub struct BatchHeader {
    pub id: BatchId,
    pub description: String,
    pub message: Option<String>,
    pub applied_at: i64,
    pub interrupted: bool,
    pub paths: Vec<PathBuf>,
}

/// A batch that has been applied to disk, as recorded in the journal.
#[derive(Debug, Clone)]
pub struct AppliedBatch {
    /// Protected native history must never enter the legacy scalar Undo path.
    #[doc(hidden)]
    pub native_tags: bool,
    pub id: BatchId,
    /// Description inherited from the plan source (for the history UI).
    pub description: String,
    /// The same description as an opaque code-and-values descriptor (#268), so
    /// History can be rendered in the language of the day rather than in the
    /// one that happened to be active when the batch was applied. See
    /// [`ChangePlan::message`](crate::plan::ChangePlan::message).
    pub message: Option<String>,
    /// Unix timestamp of when the batch was applied.
    pub applied_at: i64,
    /// The executed plan; `old` values are what rollback restores.
    pub plan: ChangePlan,
    /// Directories this batch had to create to make room for its renames,
    /// shallowest first. Rollback removes exactly these (deepest first) so a
    /// reorganize can be undone without leaving empty folders behind — and
    /// without touching directories that already existed.
    #[allow(clippy::struct_field_names)]
    pub created_dirs: Vec<PathBuf>,
    /// Source parents Undo may need to recreate, deepest first (#153). The
    /// initial write-ahead record includes verified pruning candidates, before
    /// any removal can run. Stored batches can retain those candidates even
    /// when not all were removed; recreating an existing authorized parent is
    /// harmless and ensures files have folders to return to.
    #[allow(clippy::struct_field_names)]
    pub removed_dirs: Vec<PathBuf>,
    /// The roots this batch was authorized to write inside, canonicalized. Only
    /// interesting when a reorganize filed tracks outside the open library
    /// (#153): the destination was confirmed by the user at apply time, and
    /// recording it here is what lets undo still reach those files in a later
    /// session, when the caller knows nothing about that folder.
    pub allowed_roots: Vec<PathBuf>,
    /// Transfer evidence in a legacy-named collection: completed-copy hashes
    /// and write-ahead move object identities. Historical weak/missing stamps
    /// remain readable but cannot authorize deletion or an unproved move.
    pub copy_stamps: Vec<CopyStamp>,
    /// What each changed field read as once the batch was written (#658). Undo
    /// refuses a file whose fields have changed since, rather than writing the
    /// old values over somebody's later edit. Completed legacy batches with no
    /// values keep their original behavior. Missing outcomes in an interrupted
    /// batch are unknown; only already-restored fields are safe without them.
    pub applied_values: Vec<AppliedValue>,
    /// Before/after file identity for whole-block writes, including private frames.
    /// Empty for legacy batches. An unknown interrupted outcome fails closed.
    pub block_fingerprints: Vec<BlockFingerprint>,
    /// Recorded before its first write and never finished (#659): the process
    /// stopped part-way through the apply. Undoing it puts back what it reached.
    pub interrupted: bool,
    /// The ID3v2 revision each file it writes in place was in before the
    /// write (#666). An ordinary write saves in the revision the app is set
    /// to, so a 2.3 file comes out as 2.4; undo puts the recorded revision
    /// back. Empty for a batch recorded before this existed.
    pub id3v2_revisions: Vec<FileRevision>,
}

/// Whole-file SHA-256 identity around a block edit. The path is the original
/// plan path; transfers are resolved through the plan when checking Undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockFingerprint {
    pub path: PathBuf,
    pub before: Vec<u8>,
    pub after: Option<Vec<u8>>,
}

/// The ID3v2 revision one file was in before a batch wrote it (#666).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRevision {
    pub path: PathBuf,
    pub revision: Id3v2Revision,
}

/// One field of one file as a batch left it (#658).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedValue {
    /// Where the file was once the batch was written: its rename target if the
    /// batch moved it.
    pub path: PathBuf,
    pub field: TagField,
    /// `None` when the field read as absent.
    pub value: Option<String>,
}

/// Transfer evidence in the legacy-named copy-stamp table: streaming SHA-256
/// for copies, and write-ahead native object identity for moves. Size/mtime
/// alone never authorize ownership of an externally replaced destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyStamp {
    pub path: PathBuf,
    pub len: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Strong evidence for newly recorded copies. Legacy weak stamps remain
    /// readable, but cannot authorize deleting a destination.
    pub sha256: Option<Vec<u8>>,
    /// Durable removal intent. This private sibling path may hold a copy moved
    /// aside by an interrupted Undo; retry must inspect it before deleting.
    pub undo_path: Option<PathBuf>,
    /// Native object identity, including a birth generation. Both copies and
    /// moves require ownership of the original entry. Copies additionally need
    /// an unchanged digest; move content may legitimately change in place.
    pub identity: Option<Vec<u8>>,
    /// This legacy-named table also carries write-ahead move identities.
    pub moved: bool,
}

impl CopyStamp {
    /// The stamp of the file at `path` as it is now, or `None` when there is no
    /// such file or its time cannot be read.
    pub fn of(path: &Path) -> Option<Self> {
        if !std::fs::symlink_metadata(path).ok()?.is_file() {
            return None;
        }
        let mut file = crate::plan::open_bound_file(path, false, false).ok()?;
        Self::from_file(path, &mut file)
    }

    pub(crate) fn from_file(path: &Path, file: &mut std::fs::File) -> Option<Self> {
        use sha2::{Digest, Sha256};
        use std::io::{Read, Seek};
        file.rewind().ok()?;
        let metadata = file.metadata().ok()?;
        let since_epoch = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).ok()?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let after = file.metadata().ok()?;
        if metadata.len() != after.len()
            || metadata.modified().ok()? != after.modified().ok()?
            || same_file::Handle::from_file(file.try_clone().ok()?).ok()?
                != same_file::Handle::from_file(
                    crate::plan::open_bound_file(path, false, false).ok()?,
                )
                .ok()?
        {
            return None;
        }
        Some(Self {
            path: path.to_path_buf(),
            len: metadata.len(),
            modified_ns: i64::try_from(since_epoch.as_nanos()).ok()?,
            sha256: Some(hash.finalize().to_vec()),
            undo_path: None,
            identity: native_identity(file),
            moved: false,
        })
    }

    pub(crate) fn matches(&self, path: &Path) -> bool {
        !self.moved
            && self.sha256.as_ref().is_some_and(|hash| hash.len() == 32)
            && Self::of(path).is_some_and(|now| {
                now.len == self.len
                    && now.modified_ns == self.modified_ns
                    && now.sha256 == self.sha256
                    && self.identity.as_ref().is_some_and(|identity| {
                        NativeIdentity::decode(identity)
                            .is_some_and(|id| id.current_platform() && id.file_role(false))
                            && now.identity.as_ref() == Some(identity)
                    })
            })
    }

    pub(crate) fn for_move(source: &Path, target: &Path) -> Result<Self, JournalError> {
        #[cfg(test)]
        let _timer = crate::plan::move_metrics::Timer::new(crate::plan::move_metrics::Phase::Proof);
        let file = crate::plan::open_bound_entry(source)
            .map_err(|error| JournalError::Storage(error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| JournalError::Storage(error.to_string()))?;
        let identity = native_identity(&file).ok_or_else(|| {
            JournalError::NotRollbackable(
                "This filesystem cannot provide a durable move identity with a birth generation"
                    .into(),
            )
        })?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|time| i64::try_from(time.as_nanos()).ok())
            .unwrap_or(0);
        Ok(Self {
            path: target.to_path_buf(),
            len: metadata.len(),
            modified_ns,
            sha256: None,
            undo_path: None,
            identity: Some(identity),
            moved: true,
        })
    }
    pub(crate) fn matches_identity(&self, path: &Path) -> bool {
        #[cfg(test)]
        let _timer = crate::plan::move_metrics::Timer::new(crate::plan::move_metrics::Phase::Proof);
        self.moved
            && self.identity.as_ref().is_some_and(|expected| {
                crate::plan::open_bound_entry(path)
                    .ok()
                    .is_some_and(|file| native_file_matches(expected, &file))
            })
    }
}

/// Decode only the native identity encoding this implementation actually
/// produces. An opaque nonempty blob is not ownership evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeIdentity {
    Unix {
        device: u64,
        inode: u64,
        birth: i128,
        kind: u8,
    },
    Windows {
        volume: u64,
        file: [u8; 16],
        birth: i128,
        kind: u8,
    },
}
impl NativeIdentity {
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let value = if (bytes.starts_with(b"TRX-U1") || bytes.starts_with(b"TRX-U2"))
            && bytes.len() == 39
        {
            let device = u64::from_le_bytes(bytes[6..14].try_into().ok()?);
            let inode = u64::from_le_bytes(bytes[14..22].try_into().ok()?);
            let birth = i128::from_le_bytes(bytes[22..38].try_into().ok()?);
            let kind = if bytes.starts_with(b"TRX-U2") {
                match bytes[38] {
                    value @ 0..=2 => value,
                    _ => return None,
                }
            } else {
                // U1 false meant either a regular file OR a directory. Keep
                // it readable, but never promote it into typed authority.
                match bytes[38] {
                    0 => 255,
                    1 => 1,
                    _ => return None,
                }
            };
            if inode == 0 {
                return None;
            }
            Self::Unix {
                device,
                inode,
                birth,
                kind,
            }
        } else if (bytes.starts_with(b"TRX-W1") && bytes.len() == 46)
            || (bytes.starts_with(b"TRX-W2") && bytes.len() == 47)
        {
            let volume = u64::from_le_bytes(bytes[6..14].try_into().ok()?);
            let file: [u8; 16] = bytes[14..30].try_into().ok()?;
            let birth = i128::from_le_bytes(bytes[30..46].try_into().ok()?);
            if file == [0; 16] {
                return None;
            }
            let kind = if bytes.starts_with(b"TRX-W2") {
                match bytes[46] {
                    value @ 0..=2 => value,
                    _ => return None,
                }
            } else {
                255
            };
            Self::Windows {
                volume,
                file,
                birth,
                kind,
            }
        } else {
            return None;
        };
        let birth = match value {
            Self::Unix { birth, .. } | Self::Windows { birth, .. } => birth,
        };
        let nanos = birth.unsigned_abs();
        let duration = std::time::Duration::new(
            u64::try_from(nanos / 1_000_000_000).ok()?,
            (nanos % 1_000_000_000) as u32,
        );
        if birth >= 0 {
            std::time::UNIX_EPOCH.checked_add(duration)?;
        } else {
            std::time::UNIX_EPOCH.checked_sub(duration)?;
        }
        Some(value)
    }
    pub(crate) fn file_role(self, allow_link: bool) -> bool {
        match self {
            Self::Unix { kind: 0, .. } => true,
            Self::Unix { kind: 1, .. } => allow_link,
            Self::Windows { kind: 0, .. } => true,
            Self::Windows { kind: 1, .. } => allow_link,
            _ => false,
        }
    }
    pub(crate) fn directory_role(self) -> bool {
        matches!(
            self,
            Self::Unix { kind: 2, .. } | Self::Windows { kind: 2, .. }
        )
    }
    pub(crate) fn current_platform(self) -> bool {
        matches!(
            (cfg!(unix), self),
            (true, Self::Unix { .. }) | (false, Self::Windows { .. })
        )
    }
}

/// Compare decoded file identities, so the explicitly typed U1 link variant
/// remains compatible with U2. Ambiguous legacy non-link values never match.
pub(crate) fn native_file_matches(expected: &[u8], file: &std::fs::File) -> bool {
    let Some(expected) = NativeIdentity::decode(expected)
        .filter(|identity| identity.current_platform() && identity.file_role(true))
    else {
        return false;
    };
    native_identity(file)
        .and_then(|bytes| NativeIdentity::decode(&bytes))
        .is_some_and(|actual| actual.file_role(true) && actual == expected)
}

pub(crate) const MAX_MOVE_STAGE_ROWS: usize = 262_144;
pub(crate) const MAX_MOVE_STAGE_BYTES: usize = 128 * 1024 * 1024;
pub(crate) fn move_stage_bytes(stage: &MoveStage) -> usize {
    [&stage.target, &stage.from, &stage.to, &stage.path]
        .iter()
        .fold(
            stage
                .identity
                .len()
                .saturating_add(stage.directory_identity.len()),
            |sum, path| sum.saturating_add(path.as_os_str().len()),
        )
}

pub(crate) fn native_identity(file: &std::fs::File) -> Option<Vec<u8>> {
    let metadata = file.metadata().ok()?;
    let born = metadata.created().ok()?;
    let nanos = match born.duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => i128::try_from(value.as_nanos()).ok()?,
        Err(value) => -i128::try_from(value.duration().as_nanos()).ok()?,
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let kind = if metadata.file_type().is_symlink() {
            1
        } else if metadata.is_file() {
            0
        } else if metadata.is_dir() {
            2
        } else {
            return None;
        };
        let mut identity = b"TRX-U2".to_vec();
        identity.extend(metadata.dev().to_le_bytes());
        identity.extend(metadata.ino().to_le_bytes());
        identity.extend(nanos.to_le_bytes());
        identity.push(kind);
        Some(identity)
    }
    #[cfg(windows)]
    {
        use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
        };
        let mut info = MaybeUninit::<FILE_ID_INFO>::zeroed();
        // SAFETY: a valid live File handle and correctly sized writable output.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } == 0
        {
            return None;
        }
        // SAFETY: the successful API initialized FILE_ID_INFO.
        let info = unsafe { info.assume_init() };
        if info.FileId.Identifier == [0; 16] {
            return None;
        }
        let kind = if metadata.file_type().is_symlink() {
            1
        } else if metadata.is_file() {
            0
        } else if metadata.is_dir() {
            2
        } else {
            return None;
        };
        let mut identity = b"TRX-W2".to_vec();
        identity.extend(info.VolumeSerialNumber.to_le_bytes());
        identity.extend(info.FileId.Identifier);
        identity.extend(nanos.to_le_bytes());
        identity.push(kind);
        Some(identity)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = nanos;
        None
    }
}

/// Whether `table` already has `column`. Used to make the `copied` migration
/// (#153) idempotent — `ALTER TABLE ADD COLUMN` has no `IF NOT EXISTS`, and
/// `open` runs on every start.
/// Whether the file at `path` is a damaged SQLite database: not a database at
/// all, or one whose pages fail SQLite's own check. A file that cannot be
/// opened or is locked is not called damaged, so it is never moved aside.
fn is_damaged_database(path: &Path) -> bool {
    use rusqlite::{ErrorCode, OpenFlags};
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    // A locked healthy journal must not stall recovery classification on the
    // dependency's longer default busy wait, or be mistaken for corruption.
    if conn
        .busy_timeout(std::time::Duration::from_millis(500))
        .is_err()
    {
        return false;
    }
    match conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)) {
        Ok(verdict) => verdict != "ok",
        Err(rusqlite::Error::SqliteFailure(failure, _)) => matches!(
            failure.code,
            ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
        ),
        Err(_) => false,
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Reserve the whole recovery namespace, including SQLite companions. A plain
/// rename onto a timestamp name overwrites an earlier recovery on Unix. A newly
/// created directory makes collisions explicit and keeps all four files together.
fn reserve_recovery_directory(path: &Path) -> Result<PathBuf, JournalError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    for attempt in 0..1024 {
        let suffix = if attempt == 0 {
            format!(".corrupt-{stamp}")
        } else {
            format!(".corrupt-{stamp}-{attempt}")
        };
        let directory = with_suffix(path, &suffix);
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        match builder.create(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(JournalError::Storage(error.to_string())),
        }
    }
    Err(JournalError::Storage(
        "Could not reserve a name for the damaged journal recovery files".into(),
    ))
}

fn quarantine_damaged_database(path: &Path) -> Result<PathBuf, JournalError> {
    quarantine_damaged_database_with(path, |from, to| std::fs::rename(from, to))
}

fn quarantine_damaged_database_with(
    path: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<PathBuf, JournalError> {
    let main_stamp = CopyStamp::of(path).ok_or_else(|| {
        JournalError::Storage("Damaged journal must remain a stable regular file".into())
    })?;
    // Validate companion kinds first. A directory/link is not a SQLite-owned
    // file; never silently move it, skip it, or start a fresh database beside it.
    let mut companions = Vec::new();
    for suffix in ["-journal", "-wal", "-shm"] {
        let from = with_suffix(path, suffix);
        match std::fs::symlink_metadata(&from) {
            Ok(metadata) if metadata.file_type().is_file() => companions.push((from, suffix)),
            Ok(_) => {
                return Err(JournalError::Storage(format!(
                    "Cannot recover journal: SQLite companion is not a regular file: {}",
                    from.display()
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(JournalError::Storage(error.to_string())),
        }
    }
    let directory = reserve_recovery_directory(path)?;
    let aside = directory.join(
        path.file_name()
            .ok_or_else(|| JournalError::Storage("Journal path has no file name".into()))?,
    );
    // Preserve a complete, flushed group before moving ANY original. A crash
    // between the companion moves and main move otherwise splits evidence
    // across the old directory and an incomplete reservation.
    let sources: Vec<_> = std::iter::once((path.to_path_buf(), String::new(), main_stamp.clone()))
        .chain(
            companions
                .iter()
                .map(|(from, suffix)| {
                    let stamp = CopyStamp::of(from).ok_or_else(|| {
                        JournalError::Storage(format!(
                            "Unstable SQLite companion: {}",
                            from.display()
                        ))
                    })?;
                    Ok((from.clone(), suffix.to_string(), stamp))
                })
                .collect::<Result<Vec<_>, JournalError>>()?,
        )
        .collect();
    let mut snapshots = Vec::new();
    let snapshot_result = (|| -> std::io::Result<()> {
        for (source, suffix, stamp) in &sources {
            let name = aside
                .file_name()
                .ok_or_else(|| std::io::Error::other("Missing journal file name"))?;
            let mut snapshot_name = std::ffi::OsString::from("snapshot-");
            snapshot_name.push(name);
            let snapshot = with_suffix(&directory.join(snapshot_name), suffix);
            let mut input = std::fs::File::open(source)?;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&snapshot)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            let observed = CopyStamp::of(&snapshot)
                .ok_or_else(|| std::io::Error::other("Snapshot could not be verified"))?;
            if observed.len != stamp.len
                || observed.sha256 != stamp.sha256
                || !stamp.matches(source)
            {
                return Err(std::io::Error::other(
                    "Journal group changed during recovery snapshot",
                ));
            }
            snapshots.push(snapshot);
        }
        let mut manifest = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("recovery-group.txt"))?;
        use std::io::Write;
        writeln!(manifest, "Complete pre-move SQLite recovery group; snapshot-* files are the original group.\nSource: {path:?}")?;
        manifest.sync_all()?;
        #[cfg(unix)]
        {
            std::fs::File::open(&directory)?.sync_all()?;
            if let Some(parent) = directory.parent() {
                crate::plan::sync_directory(parent)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = snapshot_result {
        return Err(JournalError::Storage(format!("Could not snapshot damaged journal group; sources unchanged; retained directory {}: {error}", directory.display())));
    }
    let mut moved = Vec::new();
    let result = (|| -> std::io::Result<()> {
        // The main database moves LAST. A failed companion move must not leave
        // a missing database path which the next open could create and recover
        // using a leftover hot journal from the damaged database.
        for (from, suffix) in companions {
            let to = with_suffix(&aside, suffix);
            rename(&from, &to)?;
            moved.push((from, to));
        }
        if !main_stamp.matches(path) {
            return Err(std::io::Error::other(
                "Journal changed before recovery move",
            ));
        }
        rename(path, &aside)
    })();
    if let Err(error) = result {
        let mut restore_errors = Vec::new();
        for (from, to) in moved.into_iter().rev() {
            // Hard-link creation never overwrites a new entry at the original
            // name. Unsupported hard links leave evidence in the reservation.
            let restore = std::fs::hard_link(&to, &from).and_then(|()| std::fs::remove_file(&to));
            if let Err(restore) = restore {
                restore_errors.push(format!("{}: {restore}", to.display()));
            }
        }
        // Remove only our empty reservation. Any evidence that could not be
        // restored stays inside it and its location is included in the error.
        if restore_errors.is_empty()
            && sources
                .iter()
                .all(|(source, _, stamp)| stamp.matches(source))
        {
            for snapshot in &snapshots {
                let _ = std::fs::remove_file(snapshot);
            }
            let _ = std::fs::remove_file(directory.join("recovery-group.txt"));
            let _ = std::fs::remove_dir(&directory);
        }
        return Err(JournalError::Storage(format!(
            "Could not set aside damaged journal: {error}; recovery directory: {}; companion restore failures: {}",
            directory.display(), restore_errors.join(", ")
        )));
    }
    if !main_stamp.matches(&aside) {
        let _ = crate::plan::rename_noreplace(&aside, path);
        return Err(JournalError::Storage(format!(
            "Journal identity changed during recovery; retained recovery directory: {}",
            directory.display()
        )));
    }
    if !sources
        .iter()
        .all(|(_, suffix, stamp)| stamp.matches(&with_suffix(&aside, suffix)))
    {
        return Err(JournalError::Storage(format!(
            "Journal companions changed during recovery; complete snapshots retained in {}",
            directory.display()
        )));
    }
    // Both complete groups now exist. Remove only redundant, verified copies;
    // a crash during this cleanup still leaves the complete original group.
    for snapshot in snapshots {
        let _ = std::fs::remove_file(snapshot);
    }
    let _ = std::fs::remove_file(directory.join("recovery-group.txt"));
    Ok(aside)
}

fn column_exists(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> Result<bool, JournalError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_move_stage(stage: &MoveStage) -> Result<(), JournalError> {
    if [&stage.target, &stage.from, &stage.to, &stage.path]
        .iter()
        .any(|path| path.to_str().is_none())
        || !NativeIdentity::decode(&stage.identity)
            .is_some_and(|identity| identity.current_platform() && identity.file_role(true))
        || !NativeIdentity::decode(&stage.directory_identity)
            .is_some_and(|identity| identity.current_platform() && identity.directory_role())
    {
        return Err(JournalError::NotRollbackable(
            "move recovery requires Unicode paths and native identities".into(),
        ));
    }
    Ok(())
}

/// Immutable write-ahead intent for one private, reversible move stage.
/// Rows survive outcome updates and are deleted only with their batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveStage {
    pub target: PathBuf,
    pub from: PathBuf,
    pub to: PathBuf,
    pub path: PathBuf,
    pub directory_identity: Vec<u8>,
    pub identity: Vec<u8>,
}

fn validate_copy_checkpoint(
    stamp: &CopyStamp,
    previous: Option<&CopyStamp>,
) -> Result<(), JournalError> {
    if stamp.path.to_str().is_none()
        || stamp.path.as_os_str().len() > 131_072
        || stamp.len > i64::MAX as u64
        || stamp.moved
        || stamp.undo_path.is_some()
        || !stamp.sha256.as_ref().is_some_and(|hash| hash.len() == 32)
        || !stamp
            .identity
            .as_deref()
            .and_then(NativeIdentity::decode)
            .is_some_and(|id| id.current_platform() && id.file_role(false))
        || previous.is_some_and(|old| {
            old.path != stamp.path
                || old.moved
                || old.undo_path.is_some()
                || old.len > i64::MAX as u64
                || !old.sha256.as_ref().is_some_and(|hash| hash.len() == 32)
                || old.identity != stamp.identity
        })
    {
        return Err(JournalError::NotRollbackable(
            "copy checkpoint requires a bounded strong identity of the same regular file".into(),
        ));
    }
    Ok(())
}

fn approved_copy_targets(plan: &ChangePlan, path: &Path) -> usize {
    plan.changes
        .iter()
        .filter(|change| change.copy)
        .map(|change| {
            usize::from(change.rename_to.as_deref() == Some(path) && change.path != path)
                + change
                    .sidecar_renames
                    .iter()
                    .filter(|(from, to)| to == path && from != to)
                    .count()
        })
        .sum()
}

pub trait UndoJournal {
    #[cfg(target_os = "linux")]
    #[doc(hidden)]
    fn native_tags(&mut self) -> Option<&mut dyn native::NativeJournal> {
        None
    }

    /// Explicit opt-in: completed-copy evidence must survive before callbacks.
    /// Unsupported adapters are refused before recording or copying anything.
    fn supports_copy_checkpoints(&self) -> bool {
        false
    }

    /// Atomically checkpoint one completed, flushed copy bound to the original
    /// plan. `previous` is the exact earlier checkpoint for a metadata refresh;
    /// it cannot change ownership or overwrite a removal intent. An identical
    /// retry is idempotent. This does not finish the enclosing batch.
    fn checkpoint_copy(
        &mut self,
        _id: BatchId,
        _stamp: &CopyStamp,
        _previous: Option<&CopyStamp>,
    ) -> Result<(), JournalError> {
        Err(JournalError::NotRollbackable(
            "journal does not support completed-copy checkpoints".into(),
        ))
    }

    /// Persist the exact private directory and native object identity before
    /// any source-name mutation. Foreign implementations must opt in.
    fn prepare_move_stage(&mut self, _id: BatchId, _stage: &MoveStage) -> Result<(), JournalError> {
        Err(JournalError::NotRollbackable(
            "journal does not support durable move staging".into(),
        ))
    }
    fn move_stages(&self, _id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        Ok(Vec::new())
    }

    /// Persist the destination of a reversible, no-clobber copy-removal move
    /// before any filesystem change. A foreign journal must opt into durability.
    fn prepare_copy_removal(
        &mut self,
        _id: BatchId,
        _target: &Path,
        _undo_path: &Path,
    ) -> Result<(), JournalError> {
        Err(JournalError::NotRollbackable(
            "journal does not support durable copy removal".into(),
        ))
    }
    /// Persist a batch and return the id the journal assigned to it.
    ///
    /// The journal owns id assignment (like a database autoincrement) so ids
    /// stay unique across application restarts; `batch.id` on the way in is
    /// ignored. The caller should adopt the returned id.
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError>;

    /// All recorded batches, newest first.
    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError>;

    /// One batch by id, or `None` when there is no such batch. A journal that
    /// can fetch a single batch should say so: undoing one batch has no business
    /// loading every other (#449).
    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        Ok(self.batches()?.into_iter().find(|batch| batch.id == id))
    }

    /// Finish a batch recorded before its write began (#659): add what only the
    /// end of the write knows (the folders it created and removed, the copy
    /// stamps, the values it left) and clear its `interrupted` mark.
    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError>;

    /// Preserve execution facts after a failed rollback. The original record
    /// must survive even if this best-effort update fails.
    fn retain_interrupted(
        &mut self,
        _id: BatchId,
        _batch: &AppliedBatch,
    ) -> Result<(), JournalError> {
        Ok(())
    }

    /// Roll back a whole batch as a unit.
    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError>;
}

/// In-memory journal: keeps recorded batches in a `Vec` for the lifetime of
/// the process, nothing more. It exists so the transaction pipeline
/// ([`Executor`](crate::plan::Executor)) can be built and tested end to end
/// before the persistent [`SqliteJournal`] lands. It deliberately does **not**
/// satisfy the cross-restart durability requirement in architecture.md — that
/// is `SqliteJournal`'s job.
#[derive(Debug, Default)]
pub struct VecJournal {
    batches: Vec<AppliedBatch>,
    next_id: i64,
    move_stages: std::collections::HashMap<i64, Vec<MoveStage>>,
}

impl VecJournal {
    pub fn new() -> Self {
        Self {
            batches: Vec::new(),
            next_id: 1,
            move_stages: std::collections::HashMap::new(),
        }
    }
}

impl UndoJournal for VecJournal {
    fn supports_copy_checkpoints(&self) -> bool {
        true
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        stamp: &CopyStamp,
        previous: Option<&CopyStamp>,
    ) -> Result<(), JournalError> {
        validate_copy_checkpoint(stamp, previous)?;
        let batch = self
            .batches
            .iter_mut()
            .find(|batch| batch.id == id)
            .ok_or(JournalError::UnknownBatch(id))?;
        if approved_copy_targets(&batch.plan, &stamp.path) != 1 {
            return Err(JournalError::NotRollbackable(
                "copy checkpoint is not one approved copy destination".into(),
            ));
        }
        let matching: Vec<_> = batch
            .copy_stamps
            .iter()
            .enumerate()
            .filter(|(_, old)| old.path == stamp.path)
            .map(|(index, _)| index)
            .collect();
        match matching.as_slice() {
            [] if previous.is_none() => batch.copy_stamps.push(stamp.clone()),
            [index] if batch.copy_stamps[*index] == *stamp => {}
            [index] if previous == Some(&batch.copy_stamps[*index]) => {
                batch.copy_stamps[*index] = stamp.clone()
            }
            _ => {
                return Err(JournalError::NotRollbackable(
                    "copy checkpoint conflicts with persisted evidence".into(),
                ))
            }
        }
        Ok(())
    }

    fn prepare_move_stage(&mut self, id: BatchId, stage: &MoveStage) -> Result<(), JournalError> {
        validate_move_stage(stage)?;
        if !self
            .batches
            .iter()
            .find(|batch| batch.id == id)
            .is_some_and(|batch| {
                batch.copy_stamps.iter().any(|stamp| {
                    stamp.path == stage.target
                        && stamp.moved
                        && stamp.identity.as_ref() == Some(&stage.identity)
                })
            })
        {
            return Err(JournalError::NotRollbackable(
                "move stage requires original persisted ownership".into(),
            ));
        }
        let stages = self.move_stages.entry(id.0).or_default();
        if !stages.contains(stage) {
            stages.push(stage.clone());
        }
        Ok(())
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        Ok(self.move_stages.get(&id.0).cloned().unwrap_or_default())
    }

    fn prepare_copy_removal(
        &mut self,
        id: BatchId,
        target: &Path,
        undo_path: &Path,
    ) -> Result<(), JournalError> {
        if target.to_str().is_none() || undo_path.to_str().is_none() {
            return Err(JournalError::NotRollbackable(
                "Copy recovery paths must be Unicode".into(),
            ));
        }
        let batch = self
            .batches
            .iter_mut()
            .find(|batch| batch.id == id)
            .ok_or(JournalError::UnknownBatch(id))?;
        let mut matching = batch
            .copy_stamps
            .iter_mut()
            .filter(|stamp| stamp.path == target);
        let stamp = matching.next().ok_or_else(|| {
            JournalError::NotRollbackable("copy removal requires persisted identity".into())
        })?;
        if matching.next().is_some()
            || stamp.moved
            || !stamp.sha256.as_ref().is_some_and(|hash| hash.len() == 32)
            || stamp
                .undo_path
                .as_deref()
                .is_some_and(|path| path != undo_path)
        {
            return Err(JournalError::NotRollbackable(
                "copy removal requires one strong unambiguous persisted identity".into(),
            ));
        }
        stamp.undo_path = Some(undo_path.to_path_buf());
        Ok(())
    }
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        let id = BatchId(self.next_id);
        self.next_id += 1;
        let mut stored = batch.clone();
        stored.id = id;
        self.batches.push(stored);
        Ok(id)
    }

    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        // Newest first, per the trait contract.
        Ok(self.batches.iter().rev().cloned().collect())
    }

    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        let stored = self
            .batches
            .iter_mut()
            .find(|stored| stored.id == id)
            .ok_or(JournalError::UnknownBatch(id))?;
        *stored = AppliedBatch {
            id,
            interrupted: false,
            plan: stored.plan.clone(),
            ..batch.clone()
        };
        Ok(())
    }

    fn retain_interrupted(
        &mut self,
        id: BatchId,
        batch: &AppliedBatch,
    ) -> Result<(), JournalError> {
        let stored = self
            .batches
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(JournalError::UnknownBatch(id))?;
        *stored = AppliedBatch {
            id,
            interrupted: true,
            plan: stored.plan.clone(),
            ..batch.clone()
        };
        Ok(())
    }

    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        self.move_stages.remove(&id.0);
        let before = self.batches.len();
        self.batches.retain(|batch| batch.id != id);
        if self.batches.len() == before {
            return Err(JournalError::UnknownBatch(id));
        }
        Ok(())
    }
}

/// SQLite-backed journal, stored next to the application config.
///
/// This is the durable one: a batch survives an application restart, so the
/// "renamed 8,000 files, closed the app, realized in the morning the mask was
/// wrong" scenario stays recoverable. The plan is normalized across three
/// tables (`batches` -> `file_changes` -> `field_changes`) so old/new values
/// per field and old/new paths per file are first-class rows, not an opaque
/// blob.
///
/// Paths are stored as UTF-8 text via `to_string_lossy`; non-UTF-8 paths
/// (rare on the target platforms) would not round-trip byte-for-byte. Left as
/// a known limitation for now.
pub struct SqliteJournal {
    #[cfg(target_os = "linux")]
    native_profile: Option<native::Profile>,
    conn: rusqlite::Connection,
}

impl SqliteJournal {
    fn finish_with_validation(
        &mut self,
        id: BatchId,
        batch: &AppliedBatch,
        validate: impl FnOnce(&Self) -> Result<(), JournalError>,
    ) -> Result<(), JournalError> {
        ensure_unicode_batch(batch)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let affected = tx.execute(
            "UPDATE batches SET interrupted = 0 WHERE id = ?1",
            rusqlite::params![id.0],
        )?;
        if affected == 0 {
            return Err(JournalError::UnknownBatch(id));
        }
        clear_outcome(&tx, id.0)?;
        insert_outcome(&tx, id.0, batch)?;
        validate(self)?;
        tx.commit()?;
        Ok(())
    }

    fn record_with_native(
        &mut self,
        batch: &AppliedBatch,
        intents: Option<&[NativeRecordIntent]>,
    ) -> Result<BatchId, JournalError> {
        ensure_unicode_batch(batch)?;
        #[cfg(target_os = "linux")]
        if intents.is_some() {
            self.native_profile
                .as_ref()
                .ok_or_else(|| native::invalid("journal profile missing"))?
                .validate()?;
        }
        if batch.native_tags != intents.is_some() {
            return Err(JournalError::NotRollbackable(
                "native record requires its initial exact intents".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        if let Some(intents) = intents {
            let roots_digest = native::root_digest(&batch.allowed_roots);
            if intents.len() != batch.plan.changes.len()
                || intents.iter().zip(&batch.plan.changes).any(|(i, c)| {
                    i.digest != native::file_digest(c) || i.roots_digest != roots_digest
                })
            {
                return Err(native::invalid("native initial plan binding"));
            }
        }

        let mut image_ids = ImageIds::new();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO batches (description, message, applied_at, interrupted) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                batch.description,
                batch.message,
                batch.applied_at,
                batch.interrupted
            ],
        )?;
        let batch_id = tx.last_insert_rowid();

        // Reuse SQL statements across each file's rows in the same durable
        // transaction. SQLite still runs the same inserts in the same order.
        for (ordinal, change) in batch.plan.changes.iter().enumerate() {
            tx.prepare_cached(
                "INSERT INTO file_changes (batch_id, path, rename_to, copied) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(rusqlite::params![
                batch_id,
                {
                    #[cfg(target_os = "linux")]
                    {
                        if intents.is_some() {
                            rusqlite::types::Value::Blob(native::marked_path(&change.path))
                        } else {
                            rusqlite::types::Value::Text(change.path.to_string_lossy().into_owned())
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        rusqlite::types::Value::Text(change.path.to_string_lossy().into_owned())
                    }
                },
                change
                    .rename_to
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                change.copy,
            ])?;
            let file_change_id = tx.last_insert_rowid();
            #[cfg(target_os = "linux")]
            if let Some(intents) = intents {
                let intent = &intents[ordinal];
                tx.prepare_cached("INSERT INTO native_tag_files(file_change_id,batch_id,ordinal,identity,digest,intents,operation_id,roots_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?.execute(rusqlite::params![file_change_id,batch_id,ordinal as i64,intent.identity.as_slice(),intent.digest.as_slice(),intent.bytes,intent.operation_id,intent.roots_digest.as_slice()])?;
            }
            #[cfg(not(target_os = "linux"))]
            let _ = ordinal;

            for field_change in &change.tag_changes {
                tx.prepare_cached(
                    "INSERT INTO field_changes (file_change_id, field, old_value, new_value) \
                     VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(rusqlite::params![
                    file_change_id,
                    field_change.field.to_storage_key(),
                    field_change.old,
                    field_change.new,
                ])?;
            }

            // Both whole sides of the image set (#56), in order. The legacy
            // `cover_changes` table held one old/new pair and is no longer
            // written — only read, so a journal from an older build still undoes.
            if let Some(cover) = &change.cover_change {
                for (side, images) in [("old", &cover.old), ("new", &cover.new)] {
                    for image in images {
                        tx.prepare_cached(
                            "INSERT INTO cover_images \
                             (file_change_id, side, kind, description, mime, data, blob_id) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        )?
                        .execute(rusqlite::params![
                            file_change_id,
                            side,
                            image.kind.to_storage_key(),
                            image.description,
                            image.mime,
                            "tagrex:pooled-artwork:upgrade-required",
                            intern_image(&tx, &image.data, &mut image_ids)?,
                        ])?;
                    }
                }
            }

            for (from, to) in &change.sidecar_renames {
                tx.prepare_cached(
                    "INSERT INTO sidecar_renames (file_change_id, from_path, to_path) \
                     VALUES (?1, ?2, ?3)",
                )?
                .execute(rusqlite::params![
                    file_change_id,
                    from.to_string_lossy(),
                    to.to_string_lossy(),
                ])?;
            }

            // Both whole sides of every block change (#47, #205), field rows and
            // picture rows. A stripped block's snapshot is the only copy left
            // once it is off the file, so nothing here is stored by reference.
            // `has_old`/`has_new` are separate from the row counts because a
            // block that exists and holds nothing is not the same as no block.
            for block_change in &change.block_changes {
                tx.prepare_cached(
                    "INSERT INTO block_changes \
                     (file_change_id, kind, revision, old_revision, old_bytes, has_old, has_new, \
                     text_to_unicode) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )?
                .execute(rusqlite::params![
                    file_change_id,
                    block_change.kind.to_storage_key(),
                    block_change.revision.map(Id3v2Revision::to_storage_key),
                    block_change.old_revision.map(Id3v2Revision::to_storage_key),
                    block_change.old_bytes,
                    block_change.old.is_some(),
                    block_change.new.is_some(),
                    block_change.text_to_unicode,
                ])?;
                let block_change_id = tx.last_insert_rowid();
                let sides = [("old", &block_change.old), ("new", &block_change.new)];
                for (side, content) in sides {
                    let Some(content) = content else { continue };
                    for (field, value) in &content.tags {
                        tx.prepare_cached(
                            "INSERT INTO block_change_fields \
                             (block_change_id, side, field, value) VALUES (?1, ?2, ?3, ?4)",
                        )?
                        .execute(rusqlite::params![
                            block_change_id,
                            side,
                            field.to_storage_key(),
                            value
                        ])?;
                    }
                    for image in &content.covers {
                        tx.prepare_cached(
                            "INSERT INTO block_change_images \
                             (block_change_id, side, kind, description, mime, data, blob_id) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        )?
                        .execute(rusqlite::params![
                            block_change_id,
                            side,
                            image.kind.to_storage_key(),
                            image.description,
                            image.mime,
                            "tagrex:pooled-artwork:upgrade-required",
                            intern_image(&tx, &image.data, &mut image_ids)?,
                        ])?;
                    }
                }
            }
        }

        // Known before the first write, unlike the outcome below, so recorded
        // here and never again on `finish`.
        for file in &batch.id3v2_revisions {
            tx.prepare_cached(
                "INSERT INTO id3v2_revisions (batch_id, path, revision) VALUES (?1, ?2, ?3)",
            )?
            .execute(rusqlite::params![
                batch_id,
                file.path.to_string_lossy(),
                file.revision.to_storage_key(),
            ])?;
        }

        insert_outcome(&tx, batch_id, batch)?;

        #[cfg(target_os = "linux")]
        if let Some(intents) = intents {
            tx.execute("INSERT INTO native_tag_batches(batch_id,expected_count,plan_digest,intent_digest) VALUES(?1,?2,?3,?4)", rusqlite::params![batch_id,intents.len() as i64,native::plan_digest(batch).as_slice(),native::intent_digest(intents.iter()).as_slice()])?;
        }
        if intents.is_some() {
            tx.execute("INSERT INTO native_tag_schema(id,signature) VALUES(1,'tagrex-native-flac-journal-v2') ON CONFLICT(id) DO NOTHING",[])?;
            tx.pragma_update(None, "user_version", 2)?;
        }
        tx.commit()?;
        Ok(BatchId(batch_id))
    }

    #[cfg(test)]
    pub(crate) fn test_limit_to_current_pages(&self) -> Result<(), JournalError> {
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        self.conn.pragma_update(None, "max_page_count", pages)?;
        Ok(())
    }
    /// [`open`](Self::open), and when the file there is a damaged database, set
    /// it aside and start a fresh journal in its place.
    ///
    /// The journal is shared by every library, so a damaged one — a crash or a
    /// full disk part-way through a write, a sync tool's half copy — made every
    /// folder refuse to open, with nothing to do about it short of finding and
    /// deleting the file by hand. Only damage does this: a journal that is
    /// locked by another instance, or that cannot be read for want of
    /// permission, is an error as before, since moving a healthy history aside
    /// would lose it. The damaged file is renamed, not deleted, so it can still
    /// be recovered; its new path comes back so the caller can say so.
    pub fn open_or_recover(path: &Path) -> Result<(Self, Option<PathBuf>), JournalError> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(JournalError::Storage(
                    "Journal path must be a regular file".into(),
                ))
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(JournalError::Storage(error.to_string()))
            }
            _ => {}
        }
        let error = match Self::open(path) {
            Ok(journal) => return Ok((journal, None)),
            Err(error) => error,
        };
        if !is_damaged_database(path) {
            return Err(error);
        }
        let aside = quarantine_damaged_database(path)?;
        let fresh = Self::open(path).map_err(|error| {
            JournalError::Storage(format!(
                "Damaged journal preserved at {}; could not create replacement: {error}",
                aside.display()
            ))
        })?;
        Ok((fresh, Some(aside)))
    }

    /// Open (creating if needed) the journal database at `path` and run
    /// migrations.
    pub fn open(path: &Path) -> Result<Self, JournalError> {
        const VERSION: i64 = 2;
        #[cfg(target_os = "linux")]
        let native_profile = native::Profile::capture(path)?;
        let mut storage = rusqlite::Connection::open(path)?;
        storage.busy_timeout(std::time::Duration::from_millis(500))?;
        let version: i64 = storage.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if !(0..=VERSION).contains(&version) {
            return Err(JournalError::Storage(format!(
                "Unsupported journal schema version {version}; supported through {VERSION}"
            )));
        }
        let native_schema: bool = storage.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_schema')", [], |r| r.get(0))?;
        let native_ledger: bool = storage.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_files')", [], |r| r.get(0))?;
        let signature = if native_schema {
            storage
                .query_row(
                    "SELECT signature FROM native_tag_schema WHERE id=1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
        } else {
            None
        };
        let native_headers: bool = storage.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_batches')", [], |r|r.get(0))?;
        let has_native_headers = native_headers
            && storage.query_row("SELECT EXISTS(SELECT 1 FROM native_tag_batches)", [], |r| {
                r.get::<_, bool>(0)
            })?;
        let has_native_history = native_ledger
            && storage.query_row("SELECT EXISTS(SELECT 1 FROM native_tag_files)", [], |r| {
                r.get::<_, bool>(0)
            })?;
        if (version == 2 || has_native_history || has_native_headers || signature.is_some())
            && (version != 2 || signature.as_deref() != Some("tagrex-native-flac-journal-v2"))
        {
            return Err(JournalError::Storage(
                "Unrecognized native journal schema state".into(),
            ));
        }
        if signature.is_some()
            && (!native_headers
                || !native_ledger
                || !column_exists(&storage, "native_tag_files", "recovery_role")?)
        {
            return Err(JournalError::Storage(
                "Incomplete native journal schema".into(),
            ));
        }
        // ON DELETE CASCADE below depends on this being enabled per connection.
        storage.pragma_update(None, "foreign_keys", true)?;
        let transaction = storage.transaction()?;
        let conn = &*transaction;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS batches (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 description TEXT NOT NULL,
                 applied_at  INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS file_changes (
                 id        INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id  INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path      TEXT NOT NULL,
                 rename_to TEXT
             );
             CREATE TABLE IF NOT EXISTS field_changes (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 field          TEXT NOT NULL,
                 old_value      TEXT,
                 new_value      TEXT
             );
             CREATE TABLE IF NOT EXISTS created_dirs (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cover_changes (
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 old_mime       TEXT,
                 old_data       BLOB,
                 new_mime       TEXT,
                 new_data       BLOB
             );
             CREATE TABLE IF NOT EXISTS sidecar_renames (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 from_path      TEXT NOT NULL,
                 to_path        TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS cover_images (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 side           TEXT NOT NULL,
                 kind           TEXT NOT NULL,
                 description    TEXT NOT NULL,
                 mime           TEXT NOT NULL,
                 data           BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS removed_dirs (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS allowed_roots (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_removals (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 kind           TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_removal_fields (
                 id               INTEGER PRIMARY KEY AUTOINCREMENT,
                 block_removal_id INTEGER NOT NULL REFERENCES block_removals(id) ON DELETE CASCADE,
                 field            TEXT NOT NULL,
                 value            TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_removal_images (
                 id               INTEGER PRIMARY KEY AUTOINCREMENT,
                 block_removal_id INTEGER NOT NULL REFERENCES block_removals(id) ON DELETE CASCADE,
                 kind             TEXT NOT NULL,
                 description      TEXT NOT NULL,
                 mime             TEXT NOT NULL,
                 data             BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_changes (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_change_id INTEGER NOT NULL REFERENCES file_changes(id) ON DELETE CASCADE,
                 kind           TEXT NOT NULL,
                 revision       TEXT,
                 old_revision   TEXT,
                 old_bytes      BLOB,
                 has_old        INTEGER NOT NULL,
                 has_new        INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_change_fields (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 block_change_id INTEGER NOT NULL REFERENCES block_changes(id) ON DELETE CASCADE,
                 side            TEXT NOT NULL,
                 field           TEXT NOT NULL,
                 value           TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS block_change_images (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 block_change_id INTEGER NOT NULL REFERENCES block_changes(id) ON DELETE CASCADE,
                 side            TEXT NOT NULL,
                 kind            TEXT NOT NULL,
                 description     TEXT NOT NULL,
                 mime            TEXT NOT NULL,
                 data            BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS copy_stamps (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id    INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path        TEXT NOT NULL,
                 len         INTEGER NOT NULL,
                 modified_ns INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS applied_values (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path     TEXT NOT NULL,
                 field    TEXT NOT NULL,
                 value    TEXT
             );
             CREATE TABLE IF NOT EXISTS block_fingerprints (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path TEXT NOT NULL,
                 before_hash BLOB NOT NULL CHECK(length(before_hash) = 32),
                 after_hash BLOB CHECK(after_hash IS NULL OR length(after_hash) = 32)
             );
             CREATE TABLE IF NOT EXISTS id3v2_revisions (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
                 path     TEXT NOT NULL,
                 revision TEXT NOT NULL
             );",
        )?;
        // `copied` (#153) arrived after the table did, and a journal from an
        // older build is a real thing to open — a whole point of this file is
        // surviving restarts. `CREATE TABLE IF NOT EXISTS` cannot add it, so add
        // the column only when it is missing; every existing row is a move.
        if !column_exists(conn, "file_changes", "copied")? {
            conn.execute_batch(
                "ALTER TABLE file_changes ADD COLUMN copied INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        // The translatable descriptor (#268), added the same way. A row written
        // before it holds NULL and keeps showing the English it was written
        // with — there is nothing to recover a code from, and matching the
        // prose to guess one would be inventing what a batch was.
        if !column_exists(conn, "batches", "message")? {
            conn.execute_batch("ALTER TABLE batches ADD COLUMN message TEXT")?;
        }
        // The write-ahead mark (#659), added the same way. Every existing row
        // was recorded after its write had finished.
        if !column_exists(conn, "batches", "interrupted")? {
            conn.execute_batch(
                "ALTER TABLE batches ADD COLUMN interrupted INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        // The rewrite of a block's text in Unicode (#698), added the same way.
        // Every existing row is some other change.
        if !column_exists(conn, "block_changes", "text_to_unicode")? {
            conn.execute_batch(
                "ALTER TABLE block_changes ADD COLUMN text_to_unicode INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        // New snapshots share immutable image bytes across files and batches.
        // Inline rows from older journals remain readable; migration never
        // rewrites or discards their recovery material. New rows keep a TEXT
        // sentinel in the old BLOB column so an older reader fails closed;
        // it must never interpret pooled bytes as an empty original image.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS journal_image_blobs (id INTEGER PRIMARY KEY AUTOINCREMENT, hash INTEGER NOT NULL, size INTEGER NOT NULL, data BLOB NOT NULL); CREATE INDEX IF NOT EXISTS idx_journal_image_blobs_hash ON journal_image_blobs(hash, size);")?;
        for table in ["cover_images", "block_change_images"] {
            if !column_exists(conn, table, "blob_id")? {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN blob_id INTEGER REFERENCES journal_image_blobs(id)"))?;
            }
            conn.execute_batch(&format!(
                "CREATE INDEX IF NOT EXISTS idx_{table}_blob ON {table}(blob_id)"
            ))?;
        }
        // Every child table is read and deleted by its parent's id, and SQLite
        // does not index a foreign key by itself. Without these, each of the
        // lookups below — one per file, several per file — scanned its whole
        // table, so loading or rolling back a batch cost the square of the
        // journal's size: minutes for a few batches of 20,000 files (#449). An
        // existing journal gets them on its next open.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_file_changes_batch ON file_changes(batch_id);
             CREATE INDEX IF NOT EXISTS idx_file_changes_copy_target ON file_changes(batch_id,copied,rename_to);
             CREATE INDEX IF NOT EXISTS idx_sidecar_copy_target ON sidecar_renames(to_path,file_change_id);
             CREATE INDEX IF NOT EXISTS idx_field_changes_file ON field_changes(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_cover_changes_file ON cover_changes(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_cover_images_file ON cover_images(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_sidecar_renames_file ON sidecar_renames(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_block_changes_file ON block_changes(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_block_change_fields_block ON block_change_fields(block_change_id);
             CREATE INDEX IF NOT EXISTS idx_block_change_images_block ON block_change_images(block_change_id);
             CREATE INDEX IF NOT EXISTS idx_block_removals_file ON block_removals(file_change_id);
             CREATE INDEX IF NOT EXISTS idx_block_removal_fields_block ON block_removal_fields(block_removal_id);
             CREATE INDEX IF NOT EXISTS idx_block_removal_images_block ON block_removal_images(block_removal_id);
             CREATE INDEX IF NOT EXISTS idx_created_dirs_batch ON created_dirs(batch_id);
             CREATE INDEX IF NOT EXISTS idx_removed_dirs_batch ON removed_dirs(batch_id);
             CREATE INDEX IF NOT EXISTS idx_allowed_roots_batch ON allowed_roots(batch_id);
             CREATE INDEX IF NOT EXISTS idx_copy_stamps_batch ON copy_stamps(batch_id);
             CREATE INDEX IF NOT EXISTS idx_copy_stamps_batch_path ON copy_stamps(batch_id,path);
             CREATE INDEX IF NOT EXISTS idx_applied_values_batch ON applied_values(batch_id);
             CREATE INDEX IF NOT EXISTS idx_block_fingerprints_batch ON block_fingerprints(batch_id);
             CREATE INDEX IF NOT EXISTS idx_id3v2_revisions_batch ON id3v2_revisions(batch_id);",
        )?;
        for (column, sql) in [
            ("sha256", "ALTER TABLE copy_stamps ADD COLUMN sha256 BLOB"),
            (
                "undo_path",
                "ALTER TABLE copy_stamps ADD COLUMN undo_path TEXT",
            ),
            ("identity","ALTER TABLE copy_stamps ADD COLUMN identity BLOB"),
            ("moved","ALTER TABLE copy_stamps ADD COLUMN moved INTEGER NOT NULL DEFAULT 0 CHECK(moved IN (0,1))"),
        ] {
            if !column_exists(conn, "copy_stamps", column)? {
                conn.execute_batch(sql)?;
            }
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS move_stages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,
            target TEXT NOT NULL, source TEXT NOT NULL, destination TEXT NOT NULL,
            path TEXT NOT NULL, directory_identity BLOB NOT NULL, identity BLOB NOT NULL,
            UNIQUE(batch_id,path)
        ); CREATE INDEX IF NOT EXISTS idx_move_stages_batch ON move_stages(batch_id);",
        )?;
        conn.pragma_update(None, "user_version", version.max(1))?;
        #[cfg(target_os = "linux")]
        native::schema(conn)?;
        transaction.commit()?;
        Ok(Self {
            conn: storage,
            #[cfg(target_os = "linux")]
            native_profile,
        })
    }
}

impl UndoJournal for SqliteJournal {
    #[cfg(target_os = "linux")]
    fn native_tags(&mut self) -> Option<&mut dyn native::NativeJournal> {
        if self.native_profile.is_some() {
            Some(self)
        } else {
            None
        }
    }
    fn supports_copy_checkpoints(&self) -> bool {
        true
    }
    fn checkpoint_copy(
        &mut self,
        id: BatchId,
        stamp: &CopyStamp,
        previous: Option<&CopyStamp>,
    ) -> Result<(), JournalError> {
        validate_copy_checkpoint(stamp, previous)?;
        let tx = self.conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM batches WHERE id=?1)",
            [id.0],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(JournalError::UnknownBatch(id));
        }
        let approved: i64 = tx.query_row("SELECT (SELECT count(*) FROM file_changes WHERE batch_id=?1 AND copied=1 AND rename_to=?2 AND path<>?2) + (SELECT count(*) FROM sidecar_renames s INDEXED BY idx_sidecar_copy_target CROSS JOIN file_changes f ON f.id=s.file_change_id WHERE f.batch_id=?1 AND f.copied=1 AND s.to_path=?2 AND s.from_path<>?2)", rusqlite::params![id.0, stamp.path.to_string_lossy()], |row| row.get(0))?;
        if approved != 1 {
            return Err(JournalError::NotRollbackable(
                "copy checkpoint is not one approved copy destination".into(),
            ));
        }
        // LIMIT and per-cell bounds also keep adversarial persisted rows bounded.
        let mut statement = tx.prepare("SELECT id,len,modified_ns,CASE WHEN length(sha256)<=32 THEN sha256 END,CASE WHEN undo_path IS NULL OR length(CAST(undo_path AS BLOB))<=131072 THEN undo_path ELSE 'invalid' END,CASE WHEN length(identity)<=64 THEN identity END,moved FROM copy_stamps WHERE batch_id=?1 AND path=?2 LIMIT 2")?;
        let mut rows = statement.query(rusqlite::params![id.0, stamp.path.to_string_lossy()])?;
        let old = rows
            .next()?
            .map(|row| -> Result<_, rusqlite::Error> {
                Ok((
                    row.get::<_, i64>(0)?,
                    CopyStamp {
                        path: stamp.path.clone(),
                        len: u64::try_from(row.get::<_, i64>(1)?)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, -1))?,
                        modified_ns: row.get(2)?,
                        sha256: row.get(3)?,
                        undo_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                        identity: row.get(5)?,
                        moved: row.get(6)?,
                    },
                ))
            })
            .transpose()?;
        if rows.next()?.is_some() {
            return Err(JournalError::NotRollbackable(
                "duplicate copy checkpoint evidence".into(),
            ));
        }
        drop(rows);
        drop(statement);
        if let Some((row_id, old)) = old {
            if old != *stamp {
                if previous != Some(&old) {
                    return Err(JournalError::NotRollbackable(
                        "copy checkpoint conflicts with persisted evidence".into(),
                    ));
                }
                tx.execute(
                    "UPDATE copy_stamps SET len=?2,modified_ns=?3,sha256=?4 WHERE id=?1",
                    rusqlite::params![row_id, stamp.len as i64, stamp.modified_ns, stamp.sha256],
                )?;
            }
        } else {
            if previous.is_some() {
                return Err(JournalError::NotRollbackable(
                    "prior copy checkpoint is missing".into(),
                ));
            }
            tx.execute("INSERT INTO copy_stamps(batch_id,path,len,modified_ns,sha256,identity,moved) VALUES(?1,?2,?3,?4,?5,?6,0)",rusqlite::params![id.0,stamp.path.to_string_lossy(),stamp.len as i64,stamp.modified_ns,stamp.sha256,stamp.identity])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn prepare_move_stage(&mut self, id: BatchId, stage: &MoveStage) -> Result<(), JournalError> {
        validate_move_stage(stage)?;
        let tx = self.conn.transaction()?;
        let count:i64=tx.query_row("SELECT count(*) FROM copy_stamps WHERE batch_id=?1 AND path=?2 AND moved=1 AND identity=?3",rusqlite::params![id.0,stage.target.to_string_lossy(),stage.identity],|row|row.get(0))?;
        if count != 1 {
            return Err(JournalError::NotRollbackable(
                "move stage requires one persisted native ownership proof".into(),
            ));
        }
        tx.execute("INSERT INTO move_stages(batch_id,target,source,destination,path,directory_identity,identity) VALUES(?1,?2,?3,?4,?5,?6,?7)",rusqlite::params![id.0,stage.target.to_string_lossy(),stage.from.to_string_lossy(),stage.to.to_string_lossy(),stage.path.to_string_lossy(),stage.directory_identity,stage.identity])?;
        tx.commit()?;
        Ok(())
    }
    fn move_stages(&self, id: BatchId) -> Result<Vec<MoveStage>, JournalError> {
        let mut statement=self.conn.prepare("SELECT CASE WHEN length(CAST(target AS BLOB))<=131072 THEN target END, CASE WHEN length(CAST(source AS BLOB))<=131072 THEN source END, CASE WHEN length(CAST(destination AS BLOB))<=131072 THEN destination END, CASE WHEN length(CAST(path AS BLOB))<=131072 THEN path END, CASE WHEN length(directory_identity)<=64 THEN directory_identity END, CASE WHEN length(identity)<=64 THEN identity END FROM move_stages WHERE batch_id=?1 ORDER BY id LIMIT ?2")?;
        let mut rows =
            statement.query(rusqlite::params![id.0, (MAX_MOVE_STAGE_ROWS + 1) as i64])?;
        let mut stages = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next()? {
            let stage = MoveStage {
                target: PathBuf::from(row.get::<_, String>(0)?),
                from: PathBuf::from(row.get::<_, String>(1)?),
                to: PathBuf::from(row.get::<_, String>(2)?),
                path: PathBuf::from(row.get::<_, String>(3)?),
                directory_identity: row.get(4)?,
                identity: row.get(5)?,
            };
            bytes = bytes.saturating_add(move_stage_bytes(&stage));
            if stages.len() >= MAX_MOVE_STAGE_ROWS || bytes > MAX_MOVE_STAGE_BYTES {
                return Err(JournalError::NotRollbackable(
                    "move recovery collection exceeds its resource budget".into(),
                ));
            }
            stages.push(stage);
        }
        Ok(stages)
    }

    fn prepare_copy_removal(
        &mut self,
        id: BatchId,
        target: &Path,
        undo_path: &Path,
    ) -> Result<(), JournalError> {
        if target.to_str().is_none() || undo_path.to_str().is_none() {
            return Err(JournalError::NotRollbackable(
                "Copy recovery paths must be Unicode".into(),
            ));
        }
        let tx = self.conn.transaction()?;
        let existing: i64 = tx.query_row(
            "SELECT count(*) FROM copy_stamps WHERE batch_id=?1 AND path=?2",
            rusqlite::params![id.0, target.to_string_lossy()],
            |row| row.get(0),
        )?;
        if existing != 1 {
            return Err(JournalError::NotRollbackable(
                "copy removal requires one unambiguous persisted identity".into(),
            ));
        }
        let count = tx.execute("UPDATE copy_stamps SET undo_path=?3 WHERE batch_id=?1 AND path=?2 AND moved=0 AND length(sha256)=32 AND (undo_path IS NULL OR undo_path=?3)", rusqlite::params![id.0, target.to_string_lossy(), undo_path.to_string_lossy()])?;
        if count != 1 {
            return Err(JournalError::NotRollbackable(
                "copy removal requires one strong persisted identity".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }
    fn record(&mut self, batch: &AppliedBatch) -> Result<BatchId, JournalError> {
        self.record_with_native(batch, None)
    }

    fn batches(&self) -> Result<Vec<AppliedBatch>, JournalError> {
        let mut batch_stmt = self.conn.prepare(
            "SELECT id, description, message, applied_at, interrupted FROM batches \
                 ORDER BY id DESC",
        )?;
        let rows = batch_stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;

        let mut batches = Vec::new();
        for row in rows {
            let (id, description, message, applied_at, interrupted) = row?;
            let mut batch = self.load_batch(id, description, message, applied_at)?;
            batch.interrupted = interrupted;
            batches.push(batch);
        }
        Ok(batches)
    }

    fn batch(&self, id: BatchId) -> Result<Option<AppliedBatch>, JournalError> {
        let row = self
            .conn
            .query_row(
                "SELECT description, message, applied_at, interrupted FROM batches WHERE id = ?1",
                rusqlite::params![id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        row.map(|(description, message, applied_at, interrupted)| {
            let mut batch = self.load_batch(id.0, description, message, applied_at)?;
            batch.interrupted = interrupted;
            Ok(batch)
        })
        .transpose()
    }

    fn finish(&mut self, id: BatchId, batch: &AppliedBatch) -> Result<(), JournalError> {
        self.finish_with_validation(id, batch, |_| Ok(()))
    }

    fn retain_interrupted(
        &mut self,
        id: BatchId,
        batch: &AppliedBatch,
    ) -> Result<(), JournalError> {
        ensure_unicode_batch(batch)?;
        let tx = self.conn.transaction()?;
        if tx.execute(
            "UPDATE batches SET interrupted = 1 WHERE id = ?1",
            rusqlite::params![id.0],
        )? == 0
        {
            return Err(JournalError::UnknownBatch(id));
        }
        // Replace outcome rows, rather than duplicating facts recorded before
        // the write began. Snapshot payloads are left untouched.
        clear_outcome(&tx, id.0)?;
        insert_outcome(&tx, id.0, batch)?;
        tx.commit()?;
        Ok(())
    }

    fn rollback(&mut self, id: BatchId) -> Result<(), JournalError> {
        // Children cascade away via the foreign keys enabled in `open`.
        let tx = self.conn.transaction()?;
        let affected = tx.execute("DELETE FROM batches WHERE id = ?1", rusqlite::params![id.0])?;
        if affected == 0 {
            return Err(JournalError::UnknownBatch(id));
        }
        tx.execute("DELETE FROM journal_image_blobs WHERE NOT EXISTS (SELECT 1 FROM cover_images WHERE blob_id = journal_image_blobs.id) AND NOT EXISTS (SELECT 1 FROM block_change_images WHERE blob_id = journal_image_blobs.id)", [])?;
        tx.commit()?;
        Ok(())
    }
}

fn check_native_header_counts(conn: &rusqlite::Connection) -> Result<(), JournalError> {
    let table: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_batches')", [], |r|r.get(0))?;
    if !table {
        let ledger: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_files')", [], |r|r.get(0))?;
        if ledger
            && conn.query_row("SELECT EXISTS(SELECT 1 FROM native_tag_files)", [], |r| {
                r.get::<_, bool>(0)
            })?
        {
            return Err(JournalError::NotRollbackable(
                "native independent batch marker missing".into(),
            ));
        }
        if conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM file_changes WHERE typeof(path)='blob')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(JournalError::NotRollbackable(
                "native independent batch marker missing".into(),
            ));
        }
        return Ok(());
    }
    // Validate the native projection once, rather than issuing owner-count
    // lookups for each legacy header in a large history.
    let invalid: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM native_tag_batches h
            LEFT JOIN (SELECT batch_id,count(*) AS count,coalesce(sum(typeof(path)='blob'),0) AS marked FROM file_changes GROUP BY batch_id) f ON f.batch_id=h.batch_id
            LEFT JOIN (SELECT batch_id,count(*) AS count FROM native_tag_files GROUP BY batch_id) n ON n.batch_id=h.batch_id
            WHERE h.expected_count<1 OR h.expected_count>8192 OR h.expected_count<>coalesce(f.count,0) OR h.expected_count<>coalesce(f.marked,0) OR h.expected_count<>coalesce(n.count,0)
            UNION ALL SELECT 1 FROM native_tag_files n LEFT JOIN native_tag_batches h ON h.batch_id=n.batch_id WHERE h.batch_id IS NULL
            UNION ALL SELECT 1 FROM file_changes f LEFT JOIN native_tag_batches h ON h.batch_id=f.batch_id WHERE typeof(f.path)='blob' AND h.batch_id IS NULL
        )", [], |r|r.get(0))?;
    if invalid {
        return Err(JournalError::NotRollbackable(
            "native immutable batch owner collection missing".into(),
        ));
    }
    Ok(())
}

fn native_batch_count(conn: &rusqlite::Connection, id: i64) -> Result<Option<usize>, JournalError> {
    let table: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_batches')", [], |r| r.get(0))?;
    let expected: Option<i64> = if table {
        conn.query_row(
            "SELECT expected_count FROM native_tag_batches WHERE batch_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    let ledger: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_files')",
        [],
        |r| r.get(0),
    )?;
    let native: i64 = if ledger {
        conn.query_row(
            "SELECT count(*) FROM native_tag_files WHERE batch_id=?1",
            [id],
            |r| r.get(0),
        )?
    } else {
        0
    };
    let (files, marked): (i64, i64) = conn.query_row(
        "SELECT count(*),coalesce(sum(typeof(path)='blob'),0) FROM file_changes WHERE batch_id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    match expected {
        Some(count)
            if (1..=8192).contains(&count)
                && count == files
                && count == marked
                && count == native =>
        {
            Ok(Some(count as usize))
        }
        None if native == 0 && marked == 0 => Ok(None),
        _ => Err(JournalError::NotRollbackable(
            "native immutable batch owner collection missing".into(),
        )),
    }
}

fn ensure_unicode_batch(batch: &AppliedBatch) -> Result<(), JournalError> {
    let paths = batch
        .created_dirs
        .iter()
        .chain(&batch.removed_dirs)
        .chain(&batch.allowed_roots)
        .chain(batch.plan.changes.iter().flat_map(|change| {
            std::iter::once(&change.path)
                .chain(change.rename_to.as_ref())
                .chain(
                    change
                        .sidecar_renames
                        .iter()
                        .flat_map(|(from, to)| [from, to]),
                )
        }))
        .chain(
            batch
                .copy_stamps
                .iter()
                .flat_map(|stamp| std::iter::once(&stamp.path).chain(stamp.undo_path.as_ref())),
        )
        .chain(batch.applied_values.iter().map(|value| &value.path))
        .chain(batch.block_fingerprints.iter().map(|value| &value.path))
        .chain(batch.id3v2_revisions.iter().map(|value| &value.path));
    if paths.into_iter().any(|path| path.to_str().is_none()) {
        return Err(JournalError::NotRollbackable(
            "Paths must be Unicode for lossless journal identity".into(),
        ));
    }
    Ok(())
}

/// Both completion and interrupted-outcome retention replace facts atomically.
/// The initial record already contains roots/pruning intent; appending those
/// again grows recovery state and makes retrying finish non-idempotent.
fn clear_outcome(tx: &rusqlite::Transaction<'_>, batch_id: i64) -> Result<(), JournalError> {
    for table in [
        "created_dirs",
        "removed_dirs",
        "allowed_roots",
        "copy_stamps",
        "applied_values",
        "block_fingerprints",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE batch_id = ?1"),
            rusqlite::params![batch_id],
        )?;
    }
    Ok(())
}

/// Hash narrows the lookup only. Exact byte equality decides reuse, so even
/// a collision cannot replace an Undo snapshot with another image. A future
/// std hasher change can reduce reuse across versions, never correctness.
type ImageIds = std::collections::HashMap<(*const u8, usize), i64>;
fn intern_image(
    tx: &rusqlite::Transaction<'_>,
    bytes: &std::sync::Arc<[u8]>,
    ids: &mut ImageIds,
) -> Result<i64, JournalError> {
    // Every pointer key remains owned by the immutably borrowed batch until
    // record() returns. Repeated shared artwork avoids rehashing/querying its
    // complete bytes per file; distinct allocations still use exact SQL equality.
    let identity = (bytes.as_ptr(), bytes.len());
    if let Some(id) = ids.get(&identity) {
        return Ok(*id);
    }
    use rusqlite::OptionalExtension;
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    let hash = hash.finish() as i64;
    let existing = tx.query_row("SELECT id FROM journal_image_blobs WHERE hash = ?1 AND size = ?2 AND data = ?3 LIMIT 1", rusqlite::params![hash,bytes.len() as i64,bytes], |row| row.get(0)).optional()?;
    let id = if let Some(id) = existing {
        id
    } else {
        tx.execute(
            "INSERT INTO journal_image_blobs (hash,size,data) VALUES (?1,?2,?3)",
            rusqlite::params![hash, bytes.len() as i64, bytes],
        )?;
        tx.last_insert_rowid()
    };
    ids.insert(identity, id);
    Ok(id)
}

/// The rows only the end of a batch's write knows (#659): the folders it
/// created and removed, its copy stamps (#604) and the values it left (#658).
fn insert_outcome(
    tx: &rusqlite::Transaction<'_>,
    batch_id: i64,
    batch: &AppliedBatch,
) -> Result<(), JournalError> {
    for dir in &batch.created_dirs {
        tx.prepare_cached("INSERT INTO created_dirs (batch_id, path) VALUES (?1, ?2)")?
            .execute(rusqlite::params![batch_id, dir.to_string_lossy()])?;
    }
    for dir in &batch.removed_dirs {
        tx.prepare_cached("INSERT INTO removed_dirs (batch_id, path) VALUES (?1, ?2)")?
            .execute(rusqlite::params![batch_id, dir.to_string_lossy()])?;
    }
    for root in &batch.allowed_roots {
        tx.prepare_cached("INSERT INTO allowed_roots (batch_id, path) VALUES (?1, ?2)")?
            .execute(rusqlite::params![batch_id, root.to_string_lossy()])?;
    }
    for stamp in &batch.copy_stamps {
        tx.prepare_cached("INSERT INTO copy_stamps (batch_id, path, len, modified_ns, sha256, undo_path, identity, moved) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?.execute(rusqlite::params![
                batch_id,
                stamp.path.to_string_lossy(),
                i64::try_from(stamp.len).unwrap_or(i64::MAX),
                stamp.modified_ns,
                stamp.sha256,
                stamp.undo_path.as_ref().map(|path| path.to_string_lossy()),
                stamp.identity,stamp.moved,
            ],
        )?;
    }

    for fingerprint in &batch.block_fingerprints {
        tx.prepare_cached("INSERT INTO block_fingerprints (batch_id,path,before_hash,after_hash) VALUES (?1,?2,?3,?4)")?.execute(rusqlite::params![batch_id, fingerprint.path.to_string_lossy(), fingerprint.before, fingerprint.after])?;
    }
    for applied in &batch.applied_values {
        tx.prepare_cached(
            "INSERT INTO applied_values (batch_id, path, field, value) \
                 VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(rusqlite::params![
            batch_id,
            applied.path.to_string_lossy(),
            applied.field.to_storage_key(),
            applied.value,
        ])?;
    }
    Ok(())
}

impl SqliteJournal {
    /// Fetch history in one metadata-only query. Path membership stays exact
    /// (`Path::starts_with`, not SQL wildcard/prefix matching), including empty
    /// batches and symlink-root spellings supported by the command layer.
    pub fn headers(&self, roots: &[PathBuf]) -> Result<Vec<BatchHeader>, JournalError> {
        check_native_header_counts(&self.conn)?;
        let mut statement = self.conn.prepare("SELECT b.id, b.description, b.message, b.applied_at, b.interrupted, CASE WHEN typeof(f.path)='blob' AND length(f.path)>8192 THEN zeroblob(0) ELSE f.path END FROM batches b LEFT JOIN file_changes f ON f.batch_id = b.id ORDER BY b.id DESC, f.id")?;
        let mut rows = statement.query([])?;
        let mut headers: Vec<BatchHeader> = Vec::new();
        while let Some(row) = rows.next()? {
            let id = BatchId(row.get(0)?);
            if headers.last().is_none_or(|h| h.id != id) {
                headers.push(BatchHeader {
                    id,
                    description: row.get(1)?,
                    message: row.get(2)?,
                    applied_at: row.get(3)?,
                    interrupted: row.get(4)?,
                    paths: Vec::new(),
                });
            }
            if !matches!(row.get_ref(5)?, rusqlite::types::ValueRef::Null) {
                let (path, _) = stored_path(row.get_ref(5)?)?;
                headers
                    .last_mut()
                    .expect("header inserted above")
                    .paths
                    .push(path);
            }
        }
        headers.retain(|header| {
            header
                .paths
                .iter()
                .all(|path| roots.iter().any(|root| path.starts_with(root)))
        });
        Ok(headers)
    }

    /// One of the batch's plain path lists, in insertion order. `table` is a
    /// literal from this module, never anything a caller supplies.
    fn load_paths(&self, table: &str, batch_id: i64) -> Result<Vec<PathBuf>, JournalError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT path FROM {table} WHERE batch_id = ?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(rusqlite::params![batch_id], |row| {
            row.get::<_, String>(0).map(PathBuf::from)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// A batch row with everything that hangs off it.
    fn load_batch(
        &self,
        id: i64,
        description: String,
        message: Option<String>,
        applied_at: i64,
    ) -> Result<AppliedBatch, JournalError> {
        let expected_native = native_batch_count(&self.conn, id)?;
        let (changes, native_tags) = self.load_file_changes(id, &mut LoadedImages::default())?;
        if native_tags != expected_native.is_some() {
            return Err(JournalError::NotRollbackable(
                "native independent batch marker mismatch".into(),
            ));
        }
        let table:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='native_tag_files')",[],|r|r.get(0))?;
        let native_count: i64 = if table {
            self.conn.query_row(
                "SELECT count(*) FROM native_tag_files WHERE batch_id=?1",
                [id],
                |r| r.get(0),
            )?
        } else {
            0
        };
        if native_tags != (native_count > 0)
            || (native_tags && native_count != changes.len() as i64)
        {
            return Err(JournalError::NotRollbackable(
                "native history marker/intent mismatch".into(),
            ));
        }
        #[cfg(not(target_os = "linux"))]
        if native_tags {
            return Err(JournalError::NotRollbackable(
                "protected native history requires Linux recovery capabilities".into(),
            ));
        }

        let batch = AppliedBatch {
            native_tags,
            id: BatchId(id),
            description: description.clone(),
            message: message.clone(),
            applied_at,
            plan: ChangePlan {
                description,
                message,
                changes,
                // Apply-time only: what it caused is in `removed_dirs`.
                prune_empty_dirs: false,
            },
            created_dirs: self.load_paths("created_dirs", id)?,
            removed_dirs: self.load_paths("removed_dirs", id)?,
            allowed_roots: self.load_paths("allowed_roots", id)?,
            copy_stamps: self.load_copy_stamps(id)?,
            applied_values: self.load_applied_values(id)?,
            block_fingerprints: self.load_block_fingerprints(id)?,
            id3v2_revisions: self.load_id3v2_revisions(id)?,
            // Set by the caller, which read the batch row.
            interrupted: false,
        };
        #[cfg(target_os = "linux")]
        if native_tags {
            native::check_batch_header(&self.conn, &batch)?;
        }
        Ok(batch)
    }

    fn load_copy_stamps(&self, batch_id: i64) -> Result<Vec<CopyStamp>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, len, modified_ns, sha256, undo_path, identity, moved FROM copy_stamps WHERE batch_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![batch_id], |row| {
            Ok(CopyStamp {
                path: PathBuf::from(row.get::<_, String>(0)?),
                len: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                modified_ns: row.get::<_, i64>(2)?,
                sha256: row.get(3)?,
                undo_path: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                identity: row.get(5)?,
                moved: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn load_block_fingerprints(
        &self,
        batch_id: i64,
    ) -> Result<Vec<BlockFingerprint>, JournalError> {
        let mut statement = self.conn.prepare("SELECT path,before_hash,after_hash FROM block_fingerprints WHERE batch_id = ?1 ORDER BY id")?;
        let rows = statement.query_map(rusqlite::params![batch_id], |row| {
            Ok(BlockFingerprint {
                path: PathBuf::from(row.get::<_, String>(0)?),
                before: row.get(1)?,
                after: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn load_applied_values(&self, batch_id: i64) -> Result<Vec<AppliedValue>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, field, value FROM applied_values WHERE batch_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![batch_id], |row| {
            Ok(AppliedValue {
                path: PathBuf::from(row.get::<_, String>(0)?),
                field: TagField::from_storage_key(&row.get::<_, String>(1)?),
                value: row.get::<_, Option<String>>(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// A revision this build does not know is left out: undo cannot write it.
    fn load_id3v2_revisions(&self, batch_id: i64) -> Result<Vec<FileRevision>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT path, revision FROM id3v2_revisions WHERE batch_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![batch_id], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                row.get::<_, String>(1)?,
            ))
        })?;
        let mut revisions = Vec::new();
        for row in rows {
            let (path, key) = row?;
            if let Some(revision) = Id3v2Revision::from_storage_key(&key) {
                revisions.push(FileRevision { path, revision });
            }
        }
        Ok(revisions)
    }

    fn load_file_changes(
        &self,
        batch_id: i64,
        images: &mut LoadedImages,
    ) -> Result<(Vec<FileChange>, bool), JournalError> {
        // Probe the indexed child tables while reading the file row. Ordinary
        // tag/rename batches skip opening separate empty result sets for each
        // sidecar, artwork and tag-block table. Legacy rows count as present.
        let mut stmt = self.conn.prepare(
            "SELECT f.id, CASE WHEN typeof(f.path)='blob' AND length(f.path)>8192 THEN NULL ELSE f.path END,
                    f.rename_to, f.copied,
                    EXISTS(SELECT 1 FROM field_changes c WHERE c.file_change_id=f.id),
                    EXISTS(SELECT 1 FROM cover_images c WHERE c.file_change_id=f.id) OR
                        EXISTS(SELECT 1 FROM cover_changes c WHERE c.file_change_id=f.id),
                    EXISTS(SELECT 1 FROM sidecar_renames c WHERE c.file_change_id=f.id),
                    EXISTS(SELECT 1 FROM block_changes c WHERE c.file_change_id=f.id) OR
                        EXISTS(SELECT 1 FROM block_removals c WHERE c.file_change_id=f.id)
               FROM file_changes f WHERE f.batch_id = ?1 ORDER BY f.id",
        )?;
        let mut field_stmt = self.conn.prepare(
            "SELECT field, old_value, new_value FROM field_changes \
             WHERE file_change_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![batch_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                stored_path(row.get_ref(1)?)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })?;

        let mut changes = Vec::new();
        let mut marker = None;
        for row in rows {
            let (
                file_change_id,
                (path, native),
                rename_to,
                copied,
                fields,
                covers,
                sidecars,
                blocks,
            ) = row?;
            if marker.is_some_and(|old| old != native) {
                return Err(JournalError::NotRollbackable(
                    "mixed native path markers".into(),
                ));
            }
            marker = Some(native);
            changes.push(FileChange {
                path,
                tag_changes: if fields {
                    Self::load_field_changes(&mut field_stmt, file_change_id)?
                } else {
                    Vec::new()
                },
                cover_change: if covers {
                    self.load_cover_change(file_change_id, images)?
                } else {
                    None
                },
                rename_to: rename_to.map(PathBuf::from),
                copy: copied,
                sidecar_renames: if sidecars {
                    self.load_sidecar_renames(file_change_id)?
                } else {
                    Vec::new()
                },
                block_changes: if blocks {
                    self.load_block_changes(file_change_id, images)?
                } else {
                    Vec::new()
                },
            });
        }
        Ok((changes, marker.unwrap_or(false)))
    }

    fn load_sidecar_renames(
        &self,
        file_change_id: i64,
    ) -> Result<Vec<(PathBuf, PathBuf)>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT from_path, to_path FROM sidecar_renames \
             WHERE file_change_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The whole-block changes one file change carries (#47, #205), both sides
    /// of each.
    ///
    /// A row whose `kind` this build has no name for is skipped rather than
    /// guessed at: it comes from a newer build, and undoing it would mean
    /// writing a block this one cannot address. The rest of the batch still
    /// rolls back. A file change with no rows here falls back to the legacy
    /// removal-only tables, so a batch recorded before conversion landed can
    /// still be undone.
    fn load_block_changes(
        &self,
        file_change_id: i64,
        images: &mut LoadedImages,
    ) -> Result<Vec<BlockChange>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, revision, old_revision, old_bytes, has_old, has_new, \
             text_to_unicode FROM block_changes WHERE file_change_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<Vec<u8>>>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })?;

        let mut changes = Vec::new();
        let mut any = false;
        for row in rows {
            let (id, kind, revision, old_revision, old_bytes, has_old, has_new, text_to_unicode) =
                row?;
            any = true;
            let Some(kind) = TagBlockKind::from_storage_key(&kind) else {
                continue;
            };
            let mut side = |side: &str, present: bool| -> Result<_, JournalError> {
                if !present {
                    return Ok(None);
                }
                Ok(Some(TagBlockContent {
                    tags: self.load_block_change_fields(id, side)?,
                    covers: self.load_block_change_images(id, side, images)?,
                }))
            };
            changes.push(BlockChange {
                kind,
                revision: revision
                    .as_deref()
                    .and_then(Id3v2Revision::from_storage_key),
                old_revision: old_revision
                    .as_deref()
                    .and_then(Id3v2Revision::from_storage_key),
                old_bytes,
                old: side("old", has_old)?,
                new: side("new", has_new)?,
                text_to_unicode,
            });
        }
        if any {
            return Ok(changes);
        }
        self.load_legacy_block_removals(file_change_id, images)
    }

    fn load_block_change_fields(&self, id: i64, side: &str) -> Result<TagMap, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT field, value FROM block_change_fields \
             WHERE block_change_id = ?1 AND side = ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![id, side], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut tags = TagMap::new();
        for row in rows {
            let (field, value) = row?;
            tags.insert(TagField::from_storage_key(&field), value);
        }
        Ok(tags)
    }

    fn load_image_bytes(
        &self,
        row: &rusqlite::Row<'_>,
        data_column: usize,
        id_column: usize,
        images: &mut LoadedImages,
    ) -> Result<std::sync::Arc<[u8]>, rusqlite::Error> {
        if let Some(id) = row.get::<_, Option<i64>>(id_column)? {
            if let Some(bytes) = images.by_id.get(&id) {
                return Ok(bytes.clone());
            }
            let bytes = self.conn.query_row(
                "SELECT data FROM journal_image_blobs WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            let bytes = images.intern(bytes);
            images.by_id.insert(id, bytes.clone());
            Ok(bytes)
        } else {
            Ok(images.intern(row.get::<_, Vec<u8>>(data_column)?))
        }
    }

    fn load_block_change_images(
        &self,
        id: i64,
        side: &str,
        images: &mut LoadedImages,
    ) -> Result<Vec<CoverArt>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, description, mime, data, blob_id FROM block_change_images \
             WHERE block_change_id = ?1 AND side = ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![id, side], |row| {
            Ok(CoverArt {
                kind: CoverKind::from_storage_key(&row.get::<_, String>(0)?),
                description: row.get::<_, String>(1)?,
                mime: row.get::<_, String>(2)?,
                data: self.load_image_bytes(row, 3, 4, images)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// A batch recorded when a block change could only ever be a removal. Read
    /// only, never written: the tables stay so a pending undo from that build
    /// still works.
    fn load_legacy_block_removals(
        &self,
        file_change_id: i64,
        images: &mut LoadedImages,
    ) -> Result<Vec<BlockChange>, JournalError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind FROM block_removals WHERE file_change_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut removals = Vec::new();
        for row in rows {
            let (removal_id, kind) = row?;
            let Some(kind) = TagBlockKind::from_storage_key(&kind) else {
                continue;
            };
            let mut fields = self.conn.prepare(
                "SELECT field, value FROM block_removal_fields \
                 WHERE block_removal_id = ?1 ORDER BY id",
            )?;
            let mut tags = TagMap::new();
            for row in fields.query_map(rusqlite::params![removal_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (field, value) = row?;
                tags.insert(TagField::from_storage_key(&field), value);
            }
            let mut image_statement = self.conn.prepare(
                "SELECT kind, description, mime, data FROM block_removal_images \
                 WHERE block_removal_id = ?1 ORDER BY id",
            )?;
            let covers = image_statement
                .query_map(rusqlite::params![removal_id], |row| {
                    Ok(CoverArt {
                        kind: CoverKind::from_storage_key(&row.get::<_, String>(0)?),
                        description: row.get::<_, String>(1)?,
                        mime: row.get::<_, String>(2)?,
                        data: images.intern(row.get::<_, Vec<u8>>(3)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            removals.push(BlockChange::removal(kind, TagBlockContent { tags, covers }));
        }
        Ok(removals)
    }

    /// The image-set change for one file change (#56), or `None` when it carries
    /// none. Reads the current `cover_images` rows; a file change with none of
    /// those falls back to the legacy single-pair `cover_changes` row, so a
    /// batch recorded before the set model can still be rolled back.
    fn load_cover_change(
        &self,
        file_change_id: i64,
        images: &mut LoadedImages,
    ) -> Result<Option<CoverChange>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT side, kind, description, mime, data, blob_id FROM cover_images \
             WHERE file_change_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CoverArt {
                    kind: CoverKind::from_storage_key(&row.get::<_, String>(1)?),
                    description: row.get::<_, String>(2)?,
                    mime: row.get::<_, String>(3)?,
                    data: self.load_image_bytes(row, 4, 5, images)?,
                },
            ))
        })?;
        let mut change = CoverChange::default();
        let mut any = false;
        for row in rows {
            let (side, image) = row?;
            any = true;
            if side == "old" {
                change.old.push(image);
            } else {
                change.new.push(image);
            }
        }
        if any {
            return Ok(Some(change));
        }
        self.load_legacy_cover_change(file_change_id, images)
    }

    fn load_legacy_cover_change(
        &self,
        file_change_id: i64,
        images: &mut LoadedImages,
    ) -> Result<Option<CoverChange>, JournalError> {
        let mut stmt = self.conn.prepare(
            "SELECT old_mime, old_data, new_mime, new_data FROM cover_changes \
             WHERE file_change_id = ?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
            ))
        })?;
        let Some(row) = rows.next() else {
            return Ok(None);
        };
        let (old_mime, old_data, new_mime, new_data) = row?;
        // A pre-#56 pair: one front cover, or none on that side.
        let mut cover = |mime: Option<String>, data: Option<Vec<u8>>| match (mime, data) {
            (Some(mime), Some(data)) => vec![CoverArt {
                mime,
                data: images.intern(data),
                kind: CoverKind::Front,
                description: String::new(),
            }],
            _ => Vec::new(),
        };
        Ok(Some(CoverChange {
            old: cover(old_mime, old_data),
            new: cover(new_mime, new_data),
        }))
    }

    fn load_field_changes(
        stmt: &mut rusqlite::Statement<'_>,
        file_change_id: i64,
    ) -> Result<Vec<FieldChange>, JournalError> {
        let rows = stmt.query_map(rusqlite::params![file_change_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;

        let mut field_changes = Vec::new();
        for row in rows {
            let (field, old, new) = row?;
            field_changes.push(FieldChange {
                field: TagField::from_storage_key(&field),
                old,
                new,
            });
        }
        Ok(field_changes)
    }
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal storage error: {0}")]
    Storage(String),
    #[error("unknown batch id: {0:?}")]
    UnknownBatch(BatchId),
    #[error("batch cannot be rolled back: {0}")]
    NotRollbackable(String),
}

impl From<rusqlite::Error> for JournalError {
    fn from(err: rusqlite::Error) -> Self {
        JournalError::Storage(err.to_string())
    }
}

#[cfg(test)]
mod recovery_fault_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-quarantine-fault-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn companion_or_main_move_failure_restores_original_recovery_files() {
        for fail_at in 1..=4 {
            let fixture = Fixture::new();
            let db = fixture.0.join("journal.sqlite");
            for suffix in ["", "-journal", "-wal", "-shm"] {
                std::fs::write(with_suffix(&db, suffix), format!("source {suffix}")).unwrap();
            }
            let mut call = 0;
            let error = quarantine_damaged_database_with(&db, |from, to| {
                call += 1;
                if call == fail_at {
                    return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
                }
                std::fs::rename(from, to)
            })
            .unwrap_err();
            assert!(error.to_string().contains("Could not set aside"));
            for suffix in ["", "-journal", "-wal", "-shm"] {
                assert_eq!(
                    std::fs::read_to_string(with_suffix(&db, suffix)).unwrap(),
                    format!("source {suffix}")
                );
            }
            assert_eq!(
                std::fs::read_dir(&fixture.0).unwrap().count(),
                4,
                "empty reservations are cleaned, evidence is retained"
            );
        }
    }

    #[test]
    fn failed_companion_restore_preserves_evidence_and_reports_its_location() {
        let fixture = Fixture::new();
        let db = fixture.0.join("journal.sqlite");
        std::fs::write(&db, b"damaged source").unwrap();
        std::fs::write(with_suffix(&db, "-journal"), b"irreplaceable journal").unwrap();
        std::fs::write(with_suffix(&db, "-wal"), b"irreplaceable WAL").unwrap();
        let mut call = 0;
        let error = quarantine_damaged_database_with(&db, |from, to| {
            call += 1;
            if call > 1 {
                std::fs::create_dir(with_suffix(&db, "-journal")).unwrap();
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert_eq!(std::fs::read(&db).unwrap(), b"damaged source");
        assert_eq!(
            std::fs::read(with_suffix(&db, "-wal")).unwrap(),
            b"irreplaceable WAL"
        );
        let directory = std::fs::read_dir(&fixture.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .contains(".corrupt-")
            })
            .unwrap();
        assert_eq!(
            std::fs::read(directory.join("journal.sqlite-journal")).unwrap(),
            b"irreplaceable journal"
        );
        assert!(with_suffix(&db, "-journal").is_dir());
        assert!(error.to_string().contains(directory.to_str().unwrap()));
        assert!(error.to_string().contains("journal.sqlite-journal"));
    }

    #[test]
    fn sqlite_full_during_snapshot_rolls_back_the_whole_record() {
        let fixture = Fixture::new();
        let mut journal = SqliteJournal::open(&fixture.0.join("journal.sqlite")).unwrap();
        let page_count: i64 = journal
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        journal
            .conn
            .pragma_update(None, "max_page_count", page_count)
            .unwrap();
        let mut batch = AppliedBatch {
            native_tags: false,
            id: BatchId(0),
            description: "SQLITE_FULL canary".into(),
            message: None,
            applied_at: 0,
            plan: ChangePlan {
                changes: vec![FileChange {
                    path: fixture.0.join("not-written.flac"),
                    tag_changes: vec![FieldChange {
                        field: TagField::Comment,
                        old: None,
                        new: Some("x".repeat(1024 * 1024)),
                    }],
                    ..FileChange::default()
                }],
                ..ChangePlan::default()
            },
            created_dirs: vec![],
            removed_dirs: vec![],
            allowed_roots: vec![fixture.0.clone()],
            copy_stamps: vec![],
            applied_values: vec![],
            block_fingerprints: vec![],
            interrupted: true,
            id3v2_revisions: vec![],
        };
        let error = journal.record(&batch).unwrap_err();
        assert!(error.to_string().contains("full"), "{error}");
        assert!(journal.batches().unwrap().is_empty());
        let fields: i64 = journal
            .conn
            .query_row("SELECT COUNT(*) FROM field_changes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fields, 0);
        journal
            .conn
            .pragma_update(None, "max_page_count", 1_073_741_823i64)
            .unwrap();
        batch.plan.changes[0].tag_changes[0].new = Some("small retry".into());
        let id = journal.record(&batch).unwrap();
        assert!(journal.batch(id).unwrap().unwrap().interrupted);
    }

    #[test]
    fn quarantine_move_child() {
        let Some(root) = std::env::var_os("TAGREX_QUARANTINE_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let stop: usize = std::env::var("TAGREX_QUARANTINE_TEST_STOP")
            .unwrap()
            .parse()
            .unwrap();
        let mut moved = 0;
        quarantine_damaged_database_with(&root.join("journal.sqlite"), |from, to| {
            std::fs::rename(from, to)?;
            moved += 1;
            if moved == stop {
                std::process::exit(83);
            }
            Ok(())
        })
        .unwrap();
        panic!("quarantine crash checkpoint was never reached");
    }

    #[test]
    fn quarantine_crash_always_retains_one_complete_group() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        for stop in 1..=4 {
            let fixture = Fixture::new();
            let db = fixture.0.join("journal.sqlite");
            for suffix in ["", "-journal", "-wal", "-shm"] {
                std::fs::write(
                    with_suffix(&db, suffix),
                    format!("original recovery {suffix}"),
                )
                .unwrap();
            }
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "journal::recovery_fault_tests::quarantine_move_child",
                    "--nocapture",
                ])
                .env("TAGREX_QUARANTINE_TEST_ROOT", &fixture.0)
                .env("TAGREX_QUARANTINE_TEST_STOP", stop.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(status.code(), Some(83));
                    break;
                }
                if Instant::now() > deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("quarantine child timed out");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let directory = std::fs::read_dir(&fixture.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.is_dir())
                .unwrap();
            assert!(directory.join("recovery-group.txt").is_file());
            for suffix in ["", "-journal", "-wal", "-shm"] {
                assert_eq!(
                    std::fs::read_to_string(
                        directory.join(format!("snapshot-journal.sqlite{suffix}"))
                    )
                    .unwrap(),
                    format!("original recovery {suffix}")
                );
            }
        }
    }

    #[test]
    fn replacement_at_the_main_quarantine_move_refuses_fresh_creation() {
        let fixture = Fixture::new();
        let db = fixture.0.join("journal.sqlite");
        std::fs::write(&db, b"original damaged bytes").unwrap();
        let error = quarantine_damaged_database_with(&db, |from, to| {
            if from == db {
                std::fs::write(from, b"new unrelated database bytes").unwrap();
            }
            std::fs::rename(from, to)
        })
        .unwrap_err();
        assert!(error.to_string().contains("identity changed"));
        assert_eq!(std::fs::read(&db).unwrap(), b"new unrelated database bytes");
        let directory = std::fs::read_dir(&fixture.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        assert_eq!(
            std::fs::read(directory.join("snapshot-journal.sqlite")).unwrap(),
            b"original damaged bytes"
        );
    }
}

#[cfg(test)]
#[path = "journal_copy_checkpoint_tests.rs"]
mod copy_checkpoint_tests;
