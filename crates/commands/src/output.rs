//! Confined sidecar output. On Unix every operation is relative to a pinned
//! directory descriptor; replacing a directory path cannot redirect a write.

use std::io;
#[cfg(test)]
use std::io::Write;
#[cfg(unix)]
use std::path::Component;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A pinned destination directory; all publication stays relative to it.
struct Directory {
    #[cfg(unix)]
    fd: rustix::fd::OwnedFd,
    path: PathBuf,
    // Windows share modes pin each ancestor's name while outputs are staged
    // and published. Opening the final directory alone would leave its parents
    // replaceable with junctions/reparse points during a path-based write.
    #[cfg(windows)]
    _ancestors: Vec<std::fs::File>,
}

impl Directory {
    /// Pin a contained output directory before any file is staged.
    fn open(root: &Path, parent: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        let parent = parent.canonicalize()?;
        let relative = parent.strip_prefix(&root).map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output is outside the library",
            )
        })?;
        #[cfg(unix)]
        {
            use rustix::fs::{open, openat, Mode, OFlags};
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let mut fd = open(&root, flags, Mode::empty())?;
            for part in relative.components() {
                let Component::Normal(name) = part else {
                    return Err(io::Error::other("invalid output parent"));
                };
                fd = openat(&fd, name, flags, Mode::empty())?;
            }
            Ok(Self { fd, path: parent })
        }
        #[cfg(not(unix))]
        {
            let _ = relative;
            #[cfg(windows)]
            let ancestors = pin_windows_ancestors(&parent)?;
            Ok(Self {
                path: parent,
                #[cfg(windows)]
                _ancestors: ancestors,
            })
        }
    }

    /// Check an entry without following links; distinguish absent from unsafe.
    fn regular(&self, name: &std::ffi::OsStr) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use rustix::fs::{statat, AtFlags, FileType};
            match statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(s) if FileType::from_raw_mode(s.st_mode) == FileType::RegularFile => Ok(true),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output target is not a regular file",
                )),
                Err(rustix::io::Errno::NOENT) => Ok(false),
                Err(e) => Err(e.into()),
            }
        }
        #[cfg(not(unix))]
        {
            match std::fs::symlink_metadata(self.path.join(name)) {
                Ok(m) if m.is_file() && !m.file_type().is_symlink() => Ok(true),
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output target is not a regular file",
                )),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
            }
        }
    }
    /// Disposable low-level transport fixture.
    #[cfg(test)]
    fn temporary(&self) -> io::Result<(std::ffi::OsString, std::fs::File)> {
        for _ in 0..100 {
            let name = format!(
                ".tagrex-output-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            #[cfg(unix)]
            let file = rustix::fs::openat(
                &self.fd,
                name.as_str(),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::EXCL
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::RUSR
                    | rustix::fs::Mode::WUSR
                    | rustix::fs::Mode::RGRP
                    | rustix::fs::Mode::WGRP
                    | rustix::fs::Mode::ROTH
                    | rustix::fs::Mode::WOTH,
            )
            .map(std::fs::File::from)
            .map_err(io::Error::from);
            #[cfg(not(unix))]
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path.join(&name));
            match file {
                Ok(file) => return Ok((name.into(), file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve output file",
        ))
    }

    /// Exercise the same native no-replace transport as the production batch.
    #[cfg(test)]
    fn publish(&self, from: &std::ffi::OsStr, to: &std::ffi::OsStr) -> io::Result<()> {
        self.move_exclusive(from, self, to)
    }

    /// Fixture cleanup only; production uses verified private capture.
    #[cfg(test)]
    fn remove(&self, name: &std::ffi::OsStr) {
        #[cfg(unix)]
        {
            let _ = rustix::fs::unlinkat(&self.fd, name, rustix::fs::AtFlags::empty());
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(self.path.join(name));
        }
    }
}

/// Open a regular input beneath a chosen root using the same pinned-directory
/// boundary as publication. Never follow a final symlink, junction or FIFO.
pub(crate) fn read_under(root: &Path, relative: &Path) -> io::Result<std::fs::File> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "expected a relative path without parent traversal",
        ));
    }
    let parent = root.join(relative.parent().unwrap_or_else(|| Path::new("")));
    let directory = Directory::open(root, &parent)?;
    let name = relative
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing filename"))?;
    #[cfg(unix)]
    let file = {
        use rustix::fs::{openat, Mode, OFlags};
        std::fs::File::from(openat(
            &directory.fd,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?)
    };
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .custom_flags(0x0020_0000)
            .open(directory.path.join(name))?;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "input is a reparse point",
            ));
        }
        file
    };
    #[cfg(not(any(unix, windows)))]
    let file = {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "confined reads are not supported on this platform",
        ));
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input is not a regular file",
        ));
    }
    Ok(file)
}

/// Windows name-surrogate reparse tags redirect a path; cloud placeholders do not.
/// https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags
#[cfg(any(windows, test))]
fn name_surrogate_tag(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

/// Pin every ancestor against rename without requiring directory-list access.
#[cfg(windows)]
fn pin_windows_ancestors(parent: &Path) -> io::Result<Vec<std::fs::File>> {
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
    };
    // Keep ownership in std::fs::File; only the reparse-tag query borrows its
    // raw handle for one Windows SDK call.
    // Metadata-only opens do not participate in Windows sharing checks. Ask
    // for traverse/execute access too, so omitting FILE_SHARE_DELETE actually
    // prevents renames until these handles are dropped.
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/8c0e3f4f-0729-49f4-a14d-7f7add593819
    // FILE_TRAVERSE shares the FILE_EXECUTE bit, which participates in the
    // documented sharing check but does not require listing ancestor contents.
    const FILE_TRAVERSE: u32 = 0x20;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let mut handles = Vec::new();
    let mut path = PathBuf::new();
    for component in parent.components() {
        path.push(component.as_os_str());
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(FILE_TRAVERSE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)?;
        let metadata = handle.metadata()?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "output ancestor is not a plain directory",
            ));
        }
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            let mut info = FILE_ATTRIBUTE_TAG_INFO {
                FileAttributes: 0,
                ReparseTag: 0,
            };
            // Query the pinned handle, not a second path lookup. Cloud-file tags
            // do not redirect names; symlinks and junctions have the surrogate bit.
            // SAFETY: the live handle is borrowed; info is writable for exactly
            // its declared size and the API retains neither pointer.
            if unsafe {
                GetFileInformationByHandleEx(
                    handle.as_raw_handle(),
                    FileAttributeTagInfo,
                    (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                    std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if name_surrogate_tag(info.ReparseTag) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "output ancestor redirects its name",
                ));
            }
        }
        handles.push(handle);
    }
    Ok(handles)
}

// Deterministic fixture-only interleaving at the same point an external process
// can replace a newly published output. Production has no fault environment.
#[cfg(test)]
type PublicationHook = Box<dyn FnMut(&Path)>;
#[cfg(test)]
thread_local! { static PUBLICATION_HOOK: std::cell::RefCell<Option<PublicationHook>> = const { std::cell::RefCell::new(None) }; }

mod ownership;
pub use ownership::OutputError;

/// Stage and validate the entire batch before publication. Ownership failures
/// retain evidence and return typed recovery requirements.
pub(crate) fn write_batch(
    root: &Path,
    outputs: &[(PathBuf, Vec<u8>)],
    overwrite: bool,
) -> Result<(), OutputError> {
    ownership::write_batch(root, outputs, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(label: &str) -> PathBuf {
        let base = std::env::var_os("TAGREX_OUTPUT_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let p = base.join(format!(
            "tagrex-output-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    // macOS may create AppleDouble resource-fork companions on FAT/exFAT.
    fn output_entries(root: &Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .filter(|entry| {
                !entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("._")
            })
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn missing_safe_capture_primitive_refuses_instead_of_checked_rename() {
        let root = root("unsupported-capture");
        let dir = Directory::open(&root, &root).unwrap();
        let (temporary, mut file) = dir.temporary().unwrap();
        file.write_all(b"staged output").unwrap();
        drop(file);
        let result = ownership::native_result(Err(rustix::io::Errno::NOTSUP));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert_eq!(
            std::fs::read(root.join(&temporary)).unwrap(),
            b"staged output"
        );
        assert!(!root.join("cover.jpg").exists());
        std::fs::write(root.join("cover.jpg"), b"original").unwrap();
        assert_eq!(
            dir.publish(&temporary, std::ffi::OsStr::new("cover.jpg"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(root.join("cover.jpg")).unwrap(), b"original");
        dir.remove(&temporary);
        drop(dir);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_publish_refuses_existing_and_dangling_links() {
        let root = root("fallback-links");
        let dir = Directory::open(&root, &root).unwrap();
        let (temporary, file) = dir.temporary().unwrap();
        drop(file);
        std::fs::write(root.join("original"), b"untouched").unwrap();
        for (name, target) in [("live", "original"), ("dangling", "missing")] {
            std::os::unix::fs::symlink(target, root.join(name)).unwrap();
            assert!(dir.publish(&temporary, std::ffi::OsStr::new(name)).is_err());
            assert!(std::fs::symlink_metadata(root.join(name))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert_eq!(std::fs::read(root.join("original")).unwrap(), b"untouched");
        dir.remove(&temporary);
        drop(dir);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn native_no_replace_errors_never_fall_back_to_replacing_rename() {
        for error in [
            rustix::io::Errno::EXIST,
            rustix::io::Errno::ACCESS,
            rustix::io::Errno::IO,
        ] {
            assert_eq!(
                ownership::native_result(Err(error))
                    .unwrap_err()
                    .raw_os_error(),
                Some(error.raw_os_error())
            );
        }
        for error in [
            rustix::io::Errno::NOSYS,
            rustix::io::Errno::INVAL,
            rustix::io::Errno::NOTSUP,
            rustix::io::Errno::OPNOTSUPP,
        ] {
            assert_eq!(
                ownership::native_result(Err(error)).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
        ownership::native_result(Ok(())).unwrap();
    }

    #[test]
    fn rollback_preserves_replacement_after_prior_output_publication() {
        let root = root("foreign-published-output");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let preserved = root.join("captured-first.txt");
        let (callback_first, callback_second, callback_preserved) =
            (first.clone(), second.clone(), preserved.clone());
        PUBLICATION_HOOK.with(|hook| {
            hook.replace(Some(Box::new(move |published| {
                if published == callback_first {
                    std::fs::rename(&callback_first, &callback_preserved).unwrap();
                    std::fs::write(&callback_first, b"external replacement").unwrap();
                    std::fs::write(&callback_second, b"racing target").unwrap();
                }
            })))
        });
        let outcome = write_batch(
            &root,
            &[
                (first.clone(), b"owned first".to_vec()),
                (second.clone(), b"owned second".to_vec()),
            ],
            false,
        );
        PUBLICATION_HOOK.with(|hook| hook.replace(None));
        assert!(outcome.is_err());
        assert_eq!(std::fs::read(&preserved).unwrap(), b"owned first");
        assert_eq!(
            std::fs::read(&first).ok().as_deref(),
            Some(b"external replacement".as_slice()),
            "rollback deleted an unowned replacement: {outcome:?}"
        );
        assert_eq!(std::fs::read(&second).unwrap(), b"racing target");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn held_candidate_identity_survives_growth_and_refuses_an_identical_replacement() {
        let root = root("held-growth");
        let path = root.join("candidate.txt");
        let original = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let held = same_file::Handle::from_file(original).unwrap();
        let mut writer = held.as_file();
        writer.write_all(b"owned bytes").unwrap();
        writer.sync_all().unwrap();
        let named = same_file::Handle::from_path(&path).unwrap();
        assert!(ownership::same_handles(&held, &named).unwrap());
        drop(named);
        let moved = root.join("moved-original.txt");
        std::fs::rename(&path, &moved).unwrap();
        std::fs::write(&path, b"owned bytes").unwrap();
        let foreign = same_file::Handle::from_path(&path).unwrap();
        assert!(!ownership::same_handles(&held, &foreign).unwrap());
        drop(foreign);
        drop(held);
        assert_eq!(std::fs::read(&moved).unwrap(), b"owned bytes");
        assert_eq!(std::fs::read(&path).unwrap(), b"owned bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_volume_capability_matches_native_transport_and_preflights_publication() {
        let root = root("volume-capability");
        let dir = Directory::open(&root, &root).unwrap();
        let (temporary, mut file) = dir.temporary().unwrap();
        file.write_all(b"transport probe").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let native = dir.publish(&temporary, std::ffi::OsStr::new("transport-probe.txt"));
        let capability = dir.ensure_exclusive_rename();
        match native {
            Ok(()) => {
                capability.unwrap();
                dir.remove(std::ffi::OsStr::new("transport-probe.txt"));
                let target = root.join("published.txt");
                write_batch(&root, &[(target.clone(), b"published".to_vec())], false).unwrap();
                assert_eq!(std::fs::read(target).unwrap(), b"published");
            }
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                assert_eq!(capability.unwrap_err().kind(), io::ErrorKind::Unsupported);
                dir.remove(&temporary);
                let target = root.join("original.txt");
                let untouched = root.join("unrelated.txt");
                let absent = root.join("new.txt");
                std::fs::write(&target, b"original bytes").unwrap();
                std::fs::write(&untouched, b"unrelated bytes").unwrap();
                for overwrite in [false, true] {
                    let result = write_batch(
                        &root,
                        &[
                            (target.clone(), b"replacement".to_vec()),
                            (absent.clone(), b"new output".to_vec()),
                        ],
                        overwrite,
                    )
                    .unwrap_err();
                    assert_eq!(result.kind(), io::ErrorKind::Unsupported);
                    assert!(result.recovery_paths().is_empty());
                    assert_eq!(std::fs::read(&target).unwrap(), b"original bytes");
                    assert_eq!(std::fs::read(&untouched).unwrap(), b"unrelated bytes");
                    assert!(!absent.exists());
                    assert_eq!(output_entries(&root), 2, "refusal left staging artifacts");
                }
            }
            Err(error) => panic!("Native transport probe failed unexpectedly: {error}"),
        }
        drop(dir);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publishes_new_files_and_replaces_existing_files_without_leftovers() {
        let root = root("publish");
        let target = root.join("cover.jpg");
        write_batch(&root, &[(target.clone(), b"first".to_vec())], false).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        assert!(write_batch(&root, &[(target.clone(), b"refused".to_vec())], false).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"first");
        write_batch(&root, &[(target.clone(), b"replacement".to_vec())], true).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"replacement");
        assert_eq!(output_entries(&root), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_preserves_a_target_that_appears_after_staging() {
        let root = root("racing-target");
        let dir = Directory::open(&root, &root).unwrap();
        let (temporary, mut file) = dir.temporary().unwrap();
        file.write_all(b"staged").unwrap();
        drop(file);
        let target = std::ffi::OsStr::new("cover.jpg");
        std::fs::write(root.join(target), b"racing writer").unwrap();
        assert!(dir.publish(&temporary, target).is_err());
        assert_eq!(std::fs::read(root.join(target)).unwrap(), b"racing writer");
        dir.remove(&temporary);
        drop(dir);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_name_surrogate_tags_redirect_windows_paths() {
        assert!(name_surrogate_tag(0xa000_000c)); // symlink
        assert!(name_surrogate_tag(0xa000_0003)); // junction/mount point
        assert!(!name_surrogate_tag(0x9000_001a)); // cloud
        assert!(!name_surrogate_tag(0x9000_f01a)); // cloud F
        assert!(!name_surrogate_tag(0)); // ordinary directory
    }

    #[cfg(windows)]
    #[test]
    fn traverse_only_parent_can_be_pinned_without_listing() {
        let root = root("traverse-only");
        let child = root.join("child");
        std::fs::create_dir(&child).unwrap();
        let script = r#"
            $ErrorActionPreference = 'Stop'
            $acl = Get-Acl -LiteralPath $env:TAGREX_PIN_TEST_PARENT
            $user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
            $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($user, [System.Security.AccessControl.FileSystemRights]::ListDirectory, [System.Security.AccessControl.AccessControlType]::Deny)
            $acl.AddAccessRule($rule)
            Set-Acl -LiteralPath $env:TAGREX_PIN_TEST_PARENT -AclObject $acl
        "#;
        let status = std::process::Command::new("powershell.exe")
            // Cargo may itself run under PowerShell 7, whose module paths
            // cannot be loaded by the Windows PowerShell 5.1 child.
            .env_remove("PSModulePath")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("TAGREX_PIN_TEST_PARENT", &root)
            .status()
            .unwrap();
        assert!(status.success());
        let result = (|| {
            assert!(std::fs::read_dir(&root).is_err());
            let pinned = Directory::open(&root, &child)?;
            let (name, file) = pinned.temporary()?;
            drop(file);
            pinned.publish(&name, std::ffi::OsStr::new("cover.jpg"))?;
            Ok::<_, io::Error>(())
        })();
        // Restore only this test directory's inherited ACL before cleanup.
        let status = std::process::Command::new("icacls.exe")
            .arg(&root)
            .args(["/reset", "/Q"])
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(root).unwrap();
        result.unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn pinned_windows_ancestors_cannot_be_renamed_until_publication_finishes() {
        let root = root("windows-pins");
        let album = root.join("album");
        let disc = album.join("disc");
        std::fs::create_dir_all(&disc).unwrap();
        let pinned = Directory::open(&root, &disc).unwrap();
        assert!(std::fs::rename(&disc, album.join("replacement-disc")).is_err());
        assert!(std::fs::rename(&album, root.join("replacement")).is_err());
        let moved_root = root.with_extension("moved");
        assert!(std::fs::rename(&root, &moved_root).is_err());

        // Pinning names must not prevent publishing files beneath them.
        let (temporary, mut file) = pinned.temporary().unwrap();
        file.write_all(b"cover").unwrap();
        drop(file);
        pinned
            .publish(&temporary, std::ffi::OsStr::new("cover.jpg"))
            .unwrap();
        pinned.remove(&temporary);
        drop(pinned);

        std::fs::rename(&disc, album.join("replacement-disc")).unwrap();
        std::fs::rename(&album, root.join("replacement")).unwrap();
        std::fs::rename(&root, &moved_root).unwrap();
        assert_eq!(
            std::fs::read(moved_root.join("replacement/replacement-disc/cover.jpg")).unwrap(),
            b"cover"
        );
        std::fs::remove_dir_all(moved_root).unwrap();
    }
    #[test]
    fn a_later_invalid_target_leaves_the_entire_batch_untouched() {
        let root = root("batch");
        std::fs::write(root.join("first.jpg"), b"original").unwrap();
        std::fs::create_dir(root.join("second.jpg")).unwrap();
        assert!(write_batch(
            &root,
            &[
                (root.join("first.jpg"), b"changed".to_vec()),
                (root.join("second.jpg"), vec![1])
            ],
            true
        )
        .is_err());
        assert_eq!(std::fs::read(root.join("first.jpg")).unwrap(), b"original");
        assert_eq!(output_entries(&root), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn pinned_parent_cannot_be_redirected_by_a_directory_symlink() {
        let root = root("parent-race");
        let inner = root.join("album");
        let outside = root.join("outside");
        std::fs::create_dir(&inner).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let dir = Directory::open(&root, &inner).unwrap();
        std::fs::rename(&inner, root.join("original-album")).unwrap();
        std::os::unix::fs::symlink(&outside, &inner).unwrap();
        let (temp, mut f) = dir.temporary().unwrap();
        f.write_all(b"image").unwrap();
        drop(f);
        dir.publish(&temp, std::ffi::OsStr::new("cover.jpg"))
            .unwrap();
        dir.remove(&temp);
        assert!(!outside.join("cover.jpg").exists());
        assert_eq!(
            std::fs::read(root.join("original-album/cover.jpg")).unwrap(),
            b"image"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod adversarial_tests;

#[cfg(test)]
mod independent_review_tests;
