//! Private profile storage shared by the service and its local connector.
//! File creation protects the empty file before any secret bytes are written.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

#[cfg(windows)]
mod windows_security;

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Service state must be private to the current user",
    )
}

pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(denied());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                // SAFETY: geteuid has no arguments or memory access requirements.
                if metadata.uid() != unsafe { libc::geteuid() } {
                    return Err(denied());
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(windows)]
            windows_security::create_private_dir_all(path)?;
            #[cfg(not(windows))]
            fs::create_dir_all(path)?;
        }
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    windows_security::set_private(path)?;
    verify_private(path, true)
}

pub fn verify_private(path: &Path, directory: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(denied());
    }
    verify_metadata(&metadata)?;
    #[cfg(windows)]
    windows_security::verify_private(path)?;
    Ok(())
}

fn verify_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no arguments or memory access requirements.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(denied());
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open the named object, never its target.
        options.custom_flags(0x0020_0000);
    }
    options
}

fn verify_opened(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(denied());
    }
    verify_metadata(&metadata)?;
    #[cfg(windows)]
    windows_security::verify_opened(file)?;
    Ok(())
}

pub fn create_private_file(path: &Path) -> io::Result<File> {
    verify_private(path.parent().ok_or_else(denied)?, true)?;
    #[cfg(windows)]
    let file = windows_security::create_private_file(path)?;
    #[cfg(not(windows))]
    let file = options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    verify_opened(&file)?;
    Ok(file)
}

pub fn open_private_read(path: &Path) -> io::Result<File> {
    verify_private(path, false)?;
    let file = options().read(true).open(path)?;
    verify_opened(&file)?;
    Ok(file)
}

/// Opens a stable private lock inode without truncation or replacement. The
/// caller retains its file lock for the whole service/worker lifetime.
pub fn open_private_lock(path: &Path) -> io::Result<File> {
    match create_private_file(path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            verify_private(path, false)?;
            let file = options().read(true).write(true).open(path)?;
            verify_opened(&file)?;
            Ok(file)
        }
        Err(error) => Err(error),
    }
}

/// Atomic private replacement with a file sync before rename and directory
/// sync where supported. Existing symlinks and foreign/broadly-readable files
/// are refused, rather than read or overwritten through their target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_publication(path, bytes, || {})
}

/// Reports when the new file becomes visible, including replacements whose
/// subsequent directory sync reports a durability error.
pub(super) fn write_atomic_with_publication(
    path: &Path,
    bytes: &[u8],
    published: impl FnOnce(),
) -> io::Result<()> {
    let parent = path.parent().ok_or_else(denied)?;
    verify_private(parent, true)?;
    match fs::symlink_metadata(path) {
        Ok(_) => verify_private(path, false)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random)
        .map_err(|_| io::Error::other("Cannot create private temporary identity"))?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = parent.join(format!(".atomic-{suffix}"));
    let result = (|| {
        let mut file = create_private_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        published();
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Read no more than the supplied bound, even if the file changes after stat.
pub fn read_bounded(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let file = open_private_read(path)?;
    let mut bytes = Vec::new();
    file.take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Private state exceeds its size limit",
        ));
    }
    Ok(bytes)
}

#[cfg(windows)]
pub fn current_user_sid() -> io::Result<String> {
    windows_security::current_sid()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn private_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        ensure_private_dir(&private).unwrap();
        (root, private)
    }

    #[test]
    fn atomic_roundtrip_replaces_only_the_destination() {
        let (_root, dir) = private_dir();
        let file = dir.join("state.json");
        write_atomic(&file, b"first").unwrap();
        let mut published = false;
        write_atomic_with_publication(&file, b"second", || published = true).unwrap();
        assert!(published);
        assert_eq!(read_bounded(&file, 6).unwrap(), b"second");
        assert!(read_bounded(&file, 5).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn publication_precedes_the_parent_sync() {
        let (root, dir) = private_dir();
        let moved = root.path().join("moved");
        let file = dir.join("state.json");
        let error = write_atomic_with_publication(&file, b"published", || {
            // Make the subsequent directory sync fail after a real file rename.
            fs::rename(&dir, &moved).unwrap();
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            read_bounded(&moved.join("state.json"), 16).unwrap(),
            b"published"
        );
    }

    #[test]
    fn stable_lock_file_is_not_truncated_on_reopen() {
        let (_root, dir) = private_dir();
        let path = dir.join("owner.lock");
        open_private_lock(&path)
            .unwrap()
            .write_all(b"owner")
            .unwrap();
        drop(open_private_lock(&path).unwrap());
        assert_eq!(read_bounded(&path, 16).unwrap(), b"owner");
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_and_broad_files_are_refused() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (_root, dir) = private_dir();
        let actual = dir.join("actual");
        create_private_file(&actual)
            .unwrap()
            .write_all(b"untouched")
            .unwrap();
        let link = dir.join("link");
        symlink(&actual, &link).unwrap();
        assert!(open_private_read(&link).is_err());
        assert!(write_atomic(&link, b"bad").is_err());
        assert_eq!(fs::read(&actual).unwrap(), b"untouched");
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open_private_read(&actual).is_err());
        assert!(write_atomic(&actual, b"bad").is_err());
    }
}
