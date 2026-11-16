//! Durable, recoverable persistence for the settings JSON (#79).
//!
//! `settings.json` is user-authored data — saved transform chains and mask
//! presets as well as preferences — so it must not be written in place. A save
//! stages the new bytes in the same directory, writes and syncs them, and then
//! renames the staging file over the target: the target holds either its
//! previous bytes or the new ones, never a half write. A file that exists but
//! cannot be *read* is refused rather than replaced, because an I/O failure is
//! not corruption and intact bytes must not be treated as replaceable. A file
//! that is readable but does not parse as settings is copied to
//! `settings.json.corrupt-<unix-stamp>` — never over an earlier copy — before
//! the replacement, so the evidence survives the recovery.
//!
//! The durability this buys is the platform's: `rename` replaces atomically on
//! Unix and Windows, and `sync_all` flushes the file's contents before the
//! rename. The parent directory is synced afterwards on Unix, best effort, so
//! the new name is more likely to survive a crash; Windows has no equivalent
//! step and the rename is the durability point there. A completed save is
//! therefore safe against a torn write, not a promise about every power-loss
//! ordering between the syncs and the rename.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Serializes read/check/replace transactions shared by desktop settings and
/// external agents. Callers hold this only on blocking workers, never while
/// waiting for an asynchronous queue or network work.
pub(crate) static DOCUMENT_LOCK: Mutex<()> = Mutex::new(());

/// Why the settings could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// The file is there but could not be read: a permission, a sharing
    /// violation, a transient failure. Its bytes are intact.
    Unreadable(io::Error),
    /// The file was read but is not valid settings JSON.
    Malformed(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(error) => write!(f, "could not read the file: {error}"),
            Self::Malformed(detail) => write!(f, "the file is not valid settings JSON: {detail}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable(error) => Some(error),
            Self::Malformed(_) => None,
        }
    }
}

/// Why the settings could not be saved. Whatever the reason, the file that was
/// there is unchanged (a recovery copy may have been made).
#[derive(Debug)]
pub enum SaveError {
    /// Settings were not read in this process, so the values being written are
    /// built on defaults nobody asked to keep. Refused until a read succeeds.
    UnresolvedRead { path: PathBuf, detail: String },
    /// The file cannot be read now, so replacing it could discard settings
    /// nobody has seen.
    Unreadable { path: PathBuf, error: io::Error },
    /// Writing the replacement failed.
    Io(io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedRead { path, detail } => write!(
                f,
                "Saved settings could not be read earlier ({detail}), so saving now would replace \
                 them with defaults. The file at {} was left untouched; reopen Settings, then save \
                 again.",
                path.display()
            ),
            Self::Unreadable { path, error } => write!(
                f,
                "Could not read the saved settings at {} ({error}), so they were left untouched \
                 instead of being replaced. Reopen Settings, then save again.",
                path.display()
            ),
            Self::Io(error) => write!(
                f,
                "Could not save the settings: {error}. The previous settings were left in place."
            ),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnresolvedRead { .. } => None,
            Self::Unreadable { error, .. } | Self::Io(error) => Some(error),
        }
    }
}

impl From<io::Error> for SaveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Read the settings at `path`. `Ok(None)` means the file is not there: the
/// caller starts from defaults and a later save may create it. `Ok(Some(..))`
/// is a parsed file. An unreadable or unparsable file is an error, so the
/// caller can tell the two apart from "nothing was ever saved".
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ReadError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadError::Unreadable(error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| ReadError::Malformed(error.to_string()))
}

/// Write `settings` to `path`, replacing whatever is there without ever leaving
/// a truncated file. See the module documentation for the guarantee and the
/// recovery copy taken from a file that does not parse.
#[cfg(test)]
pub fn save<T>(path: &Path, settings: &T) -> Result<(), SaveError>
where
    T: Serialize + DeserializeOwned,
{
    save_with_schema::<T, T>(path, settings)
}

/// Preserve unknown document slices while validating existing bytes against
/// the actual settings schema for corruption recovery.
pub(crate) fn save_with_schema<T: DeserializeOwned, S: Serialize>(
    path: &Path,
    settings: &S,
) -> Result<(), SaveError> {
    if let Some(detail) = unresolved_read_failure() {
        return Err(SaveError::UnresolvedRead {
            path: path.to_path_buf(),
            detail,
        });
    }
    // Follow a symlinked settings file first: the atomic replace must land on
    // the file the link points at, not turn the link into a regular file.
    let target = replace_target(path).map_err(|error| SaveError::Unreadable {
        path: path.to_path_buf(),
        error,
    })?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(SaveError::Io)?;
    let previous = match fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(SaveError::Unreadable {
                path: target,
                error,
            })
        }
    };
    if let Some(bytes) = &previous {
        // A file that does not parse as settings is what the caller saw as
        // defaults. Keep its bytes before they are replaced.
        if serde_json::from_slice::<T>(bytes).is_err() {
            let permissions = fs::metadata(&target).ok();
            preserve(&target, bytes, permissions.as_ref()).map_err(SaveError::Io)?;
        }
    }
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|error| SaveError::Io(io::Error::other(error)))?;
    write_replacing(&target, &json)
}

/// What the last read of the settings in this process found, when it did not
/// simply succeed. A transient failure at startup leaves the interface showing
/// defaults; saving those over a file that has since become readable would
/// discard settings nobody saw, so a save is refused until a read succeeds.
static READ_FAILURE: Mutex<Option<String>> = Mutex::new(None);

fn read_failure() -> std::sync::MutexGuard<'static, Option<String>> {
    READ_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Record that the settings could not be read, so a save is refused until one
/// succeeds.
pub(crate) fn note_read_failure(detail: String) {
    *read_failure() = Some(detail);
}

/// Record that the settings were read, or were not there, or were malformed in
/// a way the next save preserves: either way the caller's values are a usable
/// base for a save.
pub(crate) fn note_read_success() {
    *read_failure() = None;
}

/// The last read failure, when a save must not go ahead.
pub(crate) fn unresolved_read_failure() -> Option<String> {
    read_failure().clone()
}

/// The file a save must replace. A symlinked `settings.json` (a dotfiles
/// setup, say) is followed so the atomic replace lands on the file it points
/// at; a link that cannot be resolved is refused rather than replaced by a
/// regular file.
fn replace_target(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path),
        Ok(_) => Ok(path.to_path_buf()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(error),
    }
}

/// Reserve a staging file next to `target`, so the rename that replaces the
/// target stays on one filesystem. `create_new` makes the name ours alone; the
/// counter and the timestamp keep two saves (or two instances) apart.
fn reserve_staging(target: &Path) -> io::Result<(File, PathBuf)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let original = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the settings path has no file name",
        )
    })?;
    for _ in 0..100 {
        let mut name = OsString::from(".");
        name.push(original);
        name.push(format!(
            ".{}-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            nanos()
        ));
        let candidate = parent.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a temporary settings file",
    ))
}

/// Write the bytes to a staging file, keep the target's permissions, sync, and
/// rename over the target. The target is only touched by the rename, and the
/// staging file is removed if anything before it fails.
fn write_replacing(target: &Path, bytes: &[u8]) -> Result<(), SaveError> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let permissions = fs::metadata(target).ok();
    let (mut file, staging) = reserve_staging(target)?;
    let written = (|| {
        file.write_all(bytes)?;
        if let Some(metadata) = &permissions {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = written {
        discard(&staging);
        return Err(SaveError::Io(error));
    }
    if let Err(error) = fs::rename(&staging, target) {
        discard(&staging);
        return Err(SaveError::Io(error));
    }
    sync_parent(parent);
    Ok(())
}

/// Copy bytes that are not valid settings to a recovery file next to `target`,
/// never overwriting an earlier recovery copy. The copy is synced before the
/// caller replaces the original, so the evidence is on disk first.
fn preserve(path: &Path, bytes: &[u8], permissions: Option<&fs::Metadata>) -> io::Result<PathBuf> {
    preserve_with_stamp(path, bytes, permissions, unix_stamp())
}

fn preserve_with_stamp(
    path: &Path,
    bytes: &[u8],
    permissions: Option<&fs::Metadata>,
    stamp: u64,
) -> io::Result<PathBuf> {
    for attempt in 0..100 {
        let candidate = aside_path(path, stamp, attempt);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(mut file) => {
                let written = (|| {
                    if let Some(metadata) = permissions {
                        file.set_permissions(metadata.permissions())?;
                    }
                    file.write_all(bytes)?;
                    file.sync_all()
                })();
                if let Err(error) = written {
                    discard(&candidate);
                    return Err(error);
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a name for the unreadable settings copy",
    ))
}

fn aside_path(path: &Path, stamp: u64, attempt: u32) -> PathBuf {
    let mut name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("settings.json"));
    name.push(format!(".corrupt-{stamp}"));
    if attempt > 0 {
        name.push(format!("-{attempt}"));
    }
    path.with_file_name(name)
}

/// Remove a staging or recovery file we created. A failure to remove it must
/// not hide the error that got us here; the settings file is safe either way.
fn discard(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Best-effort durability of the directory entry the rename created. Unix
/// only: elsewhere there is no portable directory to sync, and the rename's
/// own atomicity is what is promised. A failure is ignored — the data is
/// already in the renamed file; this only helps the name survive a crash.
#[cfg(unix)]
fn sync_parent(parent: &Path) {
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) {}

/// Seconds since the Unix epoch, for a recovery copy's name. A clock before
/// the epoch makes the name less informative, not wrong.
fn unix_stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

/// Tests share the process-wide read failure, so every test that saves holds
/// this and starts from a clean slate; otherwise the test that deliberately
/// records a failure could refuse another test's save.
#[cfg(test)]
pub(crate) fn serialise_save_tests() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    note_read_success();
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        value: u32,
    }

    /// A directory of its own under the system temp dir (which honours
    /// `TMPDIR`), removed when the test ends, panic or not.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "tagrex-settings-{name}-{}-{}",
                std::process::id(),
                nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.join("settings.json")
        }

        fn names(&self) -> Vec<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        }

        fn staging(&self) -> Vec<String> {
            self.names()
                .into_iter()
                .filter(|name| name.ends_with(".tmp"))
                .collect()
        }

        fn recovery_copies(&self) -> Vec<String> {
            self.names()
                .into_iter()
                .filter(|name| name.contains(".corrupt-"))
                .collect()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_file_reads_as_none() {
        let scratch = Scratch::new("missing");
        assert!(matches!(read::<Sample>(&scratch.path()), Ok(None)));
    }

    #[test]
    fn saving_round_trips_and_leaves_no_staging_file() {
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("roundtrip");
        let path = scratch.path();

        save(&path, &Sample { value: 7 }).unwrap();

        assert_eq!(read::<Sample>(&path).unwrap(), Some(Sample { value: 7 }));
        assert!(
            scratch.staging().is_empty(),
            "a staging file was left behind: {:?}",
            scratch.staging()
        );
    }

    #[test]
    fn saving_replaces_the_previous_file() {
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("overwrite");
        let path = scratch.path();

        save(&path, &Sample { value: 1 }).unwrap();
        save(&path, &Sample { value: 2 }).unwrap();

        assert_eq!(read::<Sample>(&path).unwrap(), Some(Sample { value: 2 }));
        assert!(scratch.staging().is_empty());
    }

    #[test]
    fn a_file_that_is_not_settings_is_kept_before_it_is_replaced() {
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("malformed");
        let path = scratch.path();
        // Invalid UTF-8 and invalid JSON: the recovery copy keeps the bytes.
        let evidence: &[u8] = b"{ \"value\": \xff oops";

        fs::write(&path, evidence).unwrap();
        assert!(matches!(
            read::<Sample>(&path),
            Err(ReadError::Malformed(_))
        ));

        save(&path, &Sample { value: 3 }).unwrap();

        assert_eq!(read::<Sample>(&path).unwrap(), Some(Sample { value: 3 }));
        let copies = scratch.recovery_copies();
        assert_eq!(copies.len(), 1, "one recovery copy is expected: {copies:?}");
        assert_eq!(fs::read(scratch.0.join(&copies[0])).unwrap(), evidence);
        assert!(scratch.staging().is_empty());
    }

    #[test]
    fn an_earlier_recovery_copy_is_never_overwritten() {
        let scratch = Scratch::new("aside");
        let path = scratch.path();
        let first = aside_path(&path, 1_700_000_000, 0);
        fs::write(&first, b"the earlier copy").unwrap();

        let used = preserve_with_stamp(&path, b"new evidence", None, 1_700_000_000).unwrap();
        let third = preserve_with_stamp(&path, b"third", None, 1_700_000_000).unwrap();

        assert_ne!(used, first);
        assert_ne!(third, first);
        assert_ne!(third, used);
        assert_eq!(fs::read(&first).unwrap(), b"the earlier copy");
        assert_eq!(fs::read(&used).unwrap(), b"new evidence");
    }

    #[test]
    fn a_settings_path_that_cannot_be_read_is_not_replaced() {
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("unreadable");
        let path = scratch.path();
        // A directory where the file should be: readable metadata, unreadable
        // contents, and certainly not something to rename aside.
        fs::create_dir_all(&path).unwrap();

        let error = save(&path, &Sample { value: 1 }).unwrap_err();

        assert!(matches!(error, SaveError::Unreadable { .. }), "{error:?}");
        assert!(path.is_dir(), "the path was replaced");
        assert!(
            scratch.recovery_copies().is_empty(),
            "an I/O failure was treated as corruption"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_leaves_the_previous_bytes_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("readonly");
        let path = scratch.path();
        save(&path, &Sample { value: 1 }).unwrap();
        let before = fs::read(&path).unwrap();

        fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o500)).unwrap();
        // Root ignores a directory's write bit; without that guarantee the
        // check proves nothing, so the test stands down rather than lie.
        let blocked = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(scratch.0.join(".write-probe"))
            .is_err();
        let error = save(&path, &Sample { value: 2 });
        fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o700)).unwrap();
        if !blocked {
            return;
        }

        assert!(error.is_err(), "the save unexpectedly succeeded");
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(scratch.staging().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_the_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("mode");
        let path = scratch.path();
        save(&path, &Sample { value: 1 }).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        save(&path, &Sample { value: 2 }).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_settings_file_stays_a_link() {
        use std::os::unix::fs::symlink;
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("symlink");
        let real = scratch.0.join("dotfiles-settings.json");
        let link = scratch.path();
        fs::write(&real, br#"{"value":1}"#).unwrap();
        symlink(&real, &link).unwrap();

        save(&link, &Sample { value: 2 }).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link was replaced by a regular file"
        );
        assert_eq!(read::<Sample>(&link).unwrap(), Some(Sample { value: 2 }));
        assert!(scratch.staging().is_empty());
    }

    #[test]
    fn a_save_after_a_failed_read_is_refused() {
        let _serial = serialise_save_tests();
        let scratch = Scratch::new("unresolved");
        let path = scratch.path();
        save(&path, &Sample { value: 1 }).unwrap();
        let before = fs::read(&path).unwrap();

        note_read_failure("permission denied".to_string());
        let error = save(&path, &Sample { value: 2 }).unwrap_err();
        note_read_success();

        assert!(
            matches!(error, SaveError::UnresolvedRead { .. }),
            "{error:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
