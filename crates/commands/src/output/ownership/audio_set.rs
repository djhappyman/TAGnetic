//! Streamed output sets share the existing pinned, no-replace publisher.
use super::{same_handles, Directory, Private, Stamp};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{atomic::Ordering, Arc};

pub struct AudioSet {
    private: Private,
    directory: Option<Directory>,
    destination: PathBuf,
    name: Option<String>,
    owned: BTreeMap<String, OwnedFile>,
}

const STAGED_SET: &str = "set";

pub struct AudioWitness(pub(super) Stamp, pub(super) same_file::Handle);

/// Only a folder created by a successful audio-set publication can be retired.
pub struct AudioPublishedFolder {
    parent: Arc<Directory>,
    directory: Directory,
    name: String,
}

impl AudioPublishedFolder {
    pub fn remove_if_empty(self) -> io::Result<()> {
        let name = OsStr::new(&self.name);
        let identity = same_file::Handle::from_file(self.directory.directory_file()?)?;
        let current = self.parent.open_child_dir(name)?;
        if !same_handles(
            &identity,
            &same_file::Handle::from_file(current.directory_file()?)?,
        )? || std::fs::read_dir(current.actual_path()?)?.next().is_some()
        {
            return Ok(());
        }
        drop(current);
        let private = Private::create(self.parent.clone(), &mut Vec::new(), &mut false)?;
        let captured = match private.capture(&self.parent, name) {
            Ok(path) => path,
            Err(error) => {
                let _ = private.remove_empty();
                return Err(error);
            }
        };
        let remove = (|| {
            let directory = private.dir.open_child_dir(&captured)?;
            let actual = same_file::Handle::from_file(directory.directory_file()?)?;
            if !same_handles(&identity, &actual)? {
                return Err(io::Error::other("The output folder changed"));
            }
            drop(actual);
            drop(directory);
            drop(identity);
            drop(self.directory);
            private.dir.unlink_private(&captured, true)
        })();
        if let Err(error) = remove {
            private
                .dir
                .move_exclusive(&captured, &self.parent, name)
                .map_err(|restore| {
                    io::Error::other(format!(
                        "{error}; {restore}. Check {} for the retained folder.",
                        private.dir.path.display()
                    ))
                })?;
        }
        private.forget(false, &captured);
        private.remove_empty()
    }
}

struct OwnedFile {
    handle: Option<same_file::Handle>,
    verified: Option<Stamp>,
}

impl OwnedFile {
    fn open_checked(&self, directory: &Directory, name: &OsStr) -> io::Result<File> {
        let current = directory.open_regular(name, false)?;
        let actual = same_file::Handle::from_file(current.try_clone()?)?;
        if !self.matches(&actual)? {
            return Err(io::Error::other("A staged audio file changed"));
        }
        Ok(current)
    }

    fn matches(&self, actual: &same_file::Handle) -> io::Result<bool> {
        if let Some(handle) = &self.handle {
            return same_handles(handle, actual);
        }
        #[cfg(windows)]
        {
            let file = actual.as_file();
            Ok(self.verified == Some(Stamp::of(file, &file.metadata()?)?))
        }
        #[cfg(not(windows))]
        Ok(false)
    }
}

pub struct AudioDestination(Arc<Directory>);

impl AudioDestination {
    pub fn open(path: &Path) -> io::Result<Self> {
        let directory = Directory::open(path, path)?;
        directory.ensure_exclusive_rename()?;
        Ok(Self(Arc::new(directory)))
    }

    pub fn available_bytes(&self) -> io::Result<u64> {
        #[cfg(unix)]
        {
            let volume = rustix::fs::fstatvfs(&self.0.fd)?;
            Ok(volume.f_bavail.saturating_mul(volume.f_frsize))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
            let _guard = self.0.operation_guard()?;
            let wide: Vec<_> = self.0.path.as_os_str().encode_wide().chain([0]).collect();
            let mut available = 0;
            // SAFETY: the path is NUL terminated and the result borrows a live u64.
            if unsafe {
                GetDiskFreeSpaceExW(
                    wide.as_ptr(),
                    &mut available,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(available)
        }
        #[cfg(not(any(unix, windows)))]
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
        && matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
        && tagrex_core::mask::portable_file_name(name, true) == name
}

impl AudioSet {
    pub fn validate_name(name: &str) -> io::Result<()> {
        if valid_component(name) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose a portable output name",
            ))
        }
    }

    pub fn begin(destination: &AudioDestination, name: &str) -> io::Result<Self> {
        Self::validate_name(name)?;
        Self::create_stage(destination, Some(name))
    }

    pub fn begin_flat(destination: &AudioDestination) -> io::Result<Self> {
        Self::create_stage(destination, None)
    }

    fn create_stage(destination: &AudioDestination, name: Option<&str>) -> io::Result<Self> {
        let directory = destination.0.clone();
        let destination = directory.actual_path()?;
        if destination != directory.path || !directory.logical_current(&directory)? {
            return Err(io::Error::other(
                "The destination changed. Review the export again.",
            ));
        }
        if let Some(name) = name {
            require_absent(&destination.join(name))?;
        }
        let mut orphaned = Vec::new();
        let mut unlocated = false;
        let private =
            Private::create(directory, &mut orphaned, &mut unlocated).map_err(|error| {
                if orphaned.is_empty() && !unlocated {
                    error
                } else {
                    io::Error::other(format!(
                        "{error}. Inspect staging near {}",
                        destination.display()
                    ))
                }
            })?;
        let directory = private
            .dir
            .child(OsStr::new(STAGED_SET), &mut false)
            .map_err(|error| {
                let path = private
                    .path()
                    .unwrap_or_else(|_| destination.join(&private.name));
                io::Error::other(format!(
                    "{error}. Check the staging location at {}.",
                    path.display()
                ))
            })?;
        Ok(Self {
            private,
            directory: Some(directory),
            destination,
            name: name.map(str::to_owned),
            owned: BTreeMap::new(),
        })
    }

    pub fn staging_path(&self) -> io::Result<PathBuf> {
        self.staging_directory()?.actual_path()
    }

    fn staging_directory(&self) -> io::Result<&Directory> {
        self.directory
            .as_ref()
            .ok_or_else(|| io::Error::other("The staging folder was retired"))
    }

    pub fn recovery_path(&self) -> io::Result<PathBuf> {
        self.private.path()
    }

    pub fn create_file(&mut self, name: &str) -> io::Result<File> {
        Self::validate_name(name)?;
        let file = self
            .staging_directory()?
            .open_regular(OsStr::new(name), true)?;
        let handle = same_file::Handle::from_file(file.try_clone()?)?;
        self.owned.insert(
            name.into(),
            OwnedFile {
                handle: Some(handle),
                verified: None,
            },
        );
        Ok(file)
    }

    pub fn checked_file(&self, name: &str) -> io::Result<File> {
        let owned = self
            .owned
            .get(name)
            .ok_or_else(|| io::Error::other("Output was not created by this export"))?;
        owned.open_checked(self.staging_directory()?, OsStr::new(name))
    }

    pub fn witness(file: &File) -> io::Result<AudioWitness> {
        Ok(AudioWitness(
            Stamp::of(file, &file.metadata()?)?,
            same_file::Handle::from_file(file.try_clone()?)?,
        ))
    }

    pub fn mark_verified(
        &mut self,
        name: &str,
        verified: &File,
        start: AudioWitness,
    ) -> io::Result<()> {
        let current = self.checked_file(name)?;
        let expected = same_file::Handle::from_file(verified.try_clone()?)?;
        let actual = same_file::Handle::from_file(current.try_clone()?)?;
        if !same_handles(&expected, &actual)? || Self::witness(&current)?.0 != start.0 {
            return Err(io::Error::other("Staged audio changed during verification"));
        }
        let owned = self
            .owned
            .get_mut(name)
            .ok_or_else(|| io::Error::other("Output is not owned"))?;
        #[cfg(windows)]
        {
            // Flush while the writable handle is still open, then read the
            // witness after its close. NTFS may defer a write time update.
            let writer = owned
                .handle
                .as_ref()
                .ok_or_else(|| io::Error::other("Missing output writer"))?;
            writer.as_file().sync_all()?;
            owned.handle = Some(actual);
        }
        owned.verified = Some(Stamp::of(verified, &verified.metadata()?)?);
        Ok(())
    }

    fn verify_staging(&self, files: &[String]) -> io::Result<()> {
        if files.is_empty() || self.private.parent.actual_path()? != self.destination {
            return Err(io::Error::other("Destination changed before publication"));
        }
        let path = self.staging_path()?;
        if path != self.private.path()?.join(STAGED_SET) {
            return Err(io::Error::other("Staging was moved before publication"));
        }
        let expected: BTreeSet<_> = files.iter().map(OsStr::new).collect();
        if expected.len() != files.len() {
            return Err(io::Error::other("Duplicate output names"));
        }
        let entries = std::fs::read_dir(&path)?
            .map(|entry| Ok(entry?.file_name()))
            .collect::<io::Result<BTreeSet<_>>>()?;
        if expected
            .iter()
            .copied()
            .ne(entries.iter().map(|name| name.as_os_str()))
        {
            return Err(io::Error::other("Staging contains an unexpected file"));
        }
        for file in files {
            let current = self.checked_file(file)?;
            let stamp = Stamp::of(&current, &current.metadata()?)?;
            if self.owned.get(file).and_then(|owned| owned.verified) != Some(stamp) {
                return Err(io::Error::other("A staged file changed after verification"));
            }
            #[cfg(unix)]
            self.owned[file]
                .handle
                .as_ref()
                .ok_or_else(|| io::Error::other("Missing output writer"))?
                .as_file()
                .sync_all()?;
        }
        #[cfg(unix)]
        self.staging_directory()?.directory_file()?.sync_all()?;
        Ok(())
    }

    pub fn publish(
        &mut self,
        files: &[String],
        cancelled: impl FnOnce() -> bool,
    ) -> io::Result<PathBuf> {
        let name = self
            .name
            .as_deref()
            .ok_or_else(|| io::Error::other("Missing output folder name"))?;
        self.verify_staging(files)?;
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Audio export stopped",
            ));
        }
        let staged_identity =
            same_file::Handle::from_file(self.staging_directory()?.directory_file()?)?;
        let current = self.private.dir.open_child_dir(OsStr::new(STAGED_SET))?;
        let current_identity = same_file::Handle::from_file(current.directory_file()?)?;
        if !same_handles(&staged_identity, &current_identity)? {
            return Err(io::Error::other("Staging identity changed"));
        }
        drop(current_identity);
        drop(current);
        // NTFS refuses to rename a directory with open children, even with
        // delete sharing. Retain the verified file IDs and change stamps.
        #[cfg(windows)]
        for owned in self.owned.values_mut() {
            owned.handle.take();
        }
        self.private.dir.move_exclusive(
            OsStr::new(STAGED_SET),
            &self.private.parent,
            OsStr::new(name),
        )?;
        let published = self.destination.join(name);
        let verified = (|| {
            let current = self.private.parent.open_child_dir(OsStr::new(name))?;
            let actual = same_file::Handle::from_file(current.directory_file()?)?;
            if !same_handles(&staged_identity, &actual)? || current.actual_path()? != published {
                return Err(io::Error::other(
                    "Published directory does not match the staged output",
                ));
            }
            let result = self.verify_contents(files, &current, &published);
            // Even if verification finds interference, this set is already
            // visible. Leave it available for inspection and retire the empty
            // private directory.
            let _ = self.private.remove_empty();
            result
        })();
        if let Err(error) = verified {
            return Err(io::Error::other(format!(
                "{error}. Inspect the output at {}.",
                published.display()
            )));
        }
        // The output is visible at this point. A directory sync failure must
        // not describe the job as unpublished or invite a second write.
        #[cfg(unix)]
        let _ = self
            .private
            .parent
            .directory_file()
            .and_then(|parent| parent.sync_all());
        Ok(published)
    }

    pub fn published_witnesses(
        &self,
        files: &[String],
        path: &Path,
    ) -> io::Result<Vec<(PathBuf, AudioWitness)>> {
        let name = self
            .name
            .as_deref()
            .ok_or_else(|| io::Error::other("Missing output folder name"))?;
        let directory = self.private.parent.open_child_dir(OsStr::new(name))?;
        if directory.actual_path()? != path {
            return Err(io::Error::other(
                "The output folder moved after publication",
            ));
        }
        files
            .iter()
            .map(|name| {
                let file = directory.open_regular(OsStr::new(name), false)?;
                let stamp = Self::witness(&file)?;
                let owned = self
                    .owned
                    .get(name)
                    .ok_or_else(|| io::Error::other("Missing output witness"))?;
                if owned.verified != Some(stamp.0) || !owned.matches(&stamp.1)? {
                    return Err(io::Error::other("An output changed after publication"));
                }
                Ok((path.join(name), stamp))
            })
            .collect()
    }

    pub fn published_folder_witness(&self) -> io::Result<AudioPublishedFolder> {
        let name = self
            .name
            .as_ref()
            .ok_or_else(|| io::Error::other("Missing output folder name"))?;
        let directory = self.private.parent.open_child_dir(OsStr::new(name))?;
        let from = same_file::Handle::from_file(self.staging_directory()?.directory_file()?)?;
        let to = same_file::Handle::from_file(directory.directory_file()?)?;
        if !same_handles(&from, &to)? {
            return Err(io::Error::other(
                "The output folder changed after publication",
            ));
        }
        Ok(AudioPublishedFolder {
            parent: self.private.parent.clone(),
            directory,
            name: name.clone(),
        })
    }

    /// Publish a verified batch beside its sources. Directory entries appear
    /// one at a time. A partial publication leaves the reported files intact.
    pub fn publish_files(&mut self, files: &[String]) -> io::Result<Vec<(PathBuf, AudioWitness)>> {
        self.verify_staging(files)?;
        for name in files {
            require_absent(&self.destination.join(name))?;
        }
        let mut published = Vec::with_capacity(files.len());
        let mut moved = Vec::new();
        let result = (|| {
            for name in files {
                let expected = same_file::Handle::from_file(self.checked_file(name)?)?;
                let component = OsStr::new(name);
                let path = self.destination.join(name);
                self.staging_directory()?.move_exclusive(
                    component,
                    &self.private.parent,
                    component,
                )?;
                self.owned.remove(name);
                moved.push(path);
                let file = self
                    .private
                    .parent
                    .open_regular(component, false)
                    .map_err(|error| {
                        io::Error::other(format!("Could not reopen the published file: {error}"))
                    })?;
                let actual = same_file::Handle::from_file(file)?;
                if !same_handles(&expected, &actual)? {
                    return Err(io::Error::other("An output changed during publication"));
                }
                published.push((
                    self.destination.join(name),
                    Self::witness(actual.as_file())?,
                ));
            }
            #[cfg(unix)]
            self.private.parent.directory_file()?.sync_all()?;
            self.discard().map_err(|error| {
                io::Error::other(format!("Could not clean up audio staging: {error}"))
            })
        })();
        match result {
            Ok(()) => Ok(published),
            Err(error) if moved.is_empty() => Err(error),
            Err(error) => Err(io::Error::other(format!(
                "{error}. Check the published audio at {}.",
                moved
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    fn verify_contents(
        &self,
        files: &[String],
        directory: &Directory,
        path: &Path,
    ) -> io::Result<()> {
        let expected: BTreeSet<_> = files.iter().map(OsStr::new).collect();
        let entries = std::fs::read_dir(path)?
            .map(|entry| Ok(entry?.file_name()))
            .collect::<io::Result<BTreeSet<_>>>()?;
        if expected
            .iter()
            .copied()
            .ne(entries.iter().map(|name| name.as_os_str()))
        {
            return Err(io::Error::other(
                "The output set contains an unexpected file",
            ));
        }
        for name in files {
            let owned = &self.owned[name];
            let current = directory.open_regular(OsStr::new(name), false)?;
            let witness = Stamp::of(&current, &current.metadata()?)?;
            let actual = same_file::Handle::from_file(current)?;
            if !owned.matches(&actual)? || owned.verified != Some(witness) {
                return Err(io::Error::other(
                    "A file changed while publishing the output set",
                ));
            }
        }
        Ok(())
    }

    pub fn discard(&mut self) -> io::Result<()> {
        if self.private.removed.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(dir) = &self.directory else {
            return self.private.remove_empty();
        };
        let mut failure = None;
        self.owned.retain(|name, owned| {
            let discard = || -> io::Result<()> {
                let name = OsStr::new(name);
                let candidate = same_file::Handle::from_file(owned.open_checked(dir, name)?)?;
                let captured = self.private.capture(dir, name)?;
                let captured_matches = self
                    .private
                    .dir
                    .open_regular(&captured, false)
                    .and_then(same_file::Handle::from_file)
                    .and_then(|actual| same_handles(&candidate, &actual));
                if !matches!(captured_matches, Ok(true)) {
                    if self
                        .private
                        .dir
                        .move_exclusive(&captured, dir, name)
                        .is_ok()
                    {
                        self.private.forget(false, &captured);
                    }
                    return Err(io::Error::other("An unexpected staged file was retained"));
                }
                self.private.dir.unlink_private(&captured, false)?;
                self.private.forget(false, &captured);
                Ok(())
            };
            if let Err(error) = discard() {
                failure.get_or_insert(error);
                true
            } else {
                false
            }
        });
        match failure {
            Some(error) => Err(error),
            None => {
                let name = OsStr::new(STAGED_SET);
                let captured = self.private.capture(&self.private.dir, name)?;
                let identity = same_file::Handle::from_file(dir.directory_file()?)?;
                let actual = self
                    .private
                    .dir
                    .open_child_dir(&captured)
                    .and_then(|directory| {
                        same_file::Handle::from_file(directory.directory_file()?)
                    });
                if !actual.is_ok_and(|handle| same_handles(&identity, &handle).unwrap_or(false)) {
                    if self
                        .private
                        .dir
                        .move_exclusive(&captured, &self.private.dir, name)
                        .is_ok()
                    {
                        self.private.forget(false, &captured);
                    }
                    return Err(io::Error::other(
                        "The changed staging directory was retained",
                    ));
                }
                // Windows keeps a removed directory pending until its last
                // handle closes. Release it before retiring the private parent.
                drop(identity);
                self.directory.take();
                self.private.dir.unlink_private(&captured, true)?;
                self.private.forget(false, &captured);
                self.private.remove_empty()
            }
        }
    }
}

fn require_absent(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        )),
    }
}

pub fn write_new_cue(path: &Path, text: String) -> Result<(), super::OutputError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    super::write_batch(parent, &[(path.to_path_buf(), text.into_bytes())], false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixtures() -> tempfile::TempDir {
        match std::env::var_os("TAGREX_OUTPUT_TEST_ROOT") {
            Some(path) => tempfile::tempdir_in(path).unwrap(),
            None => tempfile::tempdir().unwrap(),
        }
    }

    #[test]
    fn audio_set_respects_volume_capability() {
        let root = fixtures();
        let path = root.path().canonicalize().unwrap();
        let destination = match AudioDestination::open(&path) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                assert!(std::fs::read_dir(&path).unwrap().next().is_none());
                return;
            }
            Err(error) => panic!("{error}"),
        };
        let mut stage = AudioSet::begin(&destination, "Tracks").unwrap();
        let mut file = stage.create_file("01.mp3").unwrap();
        file.write_all(b"owned output fixture").unwrap();
        drop(file);
        let held = stage.checked_file("01.mp3").unwrap();
        stage
            .mark_verified("01.mp3", &held, AudioSet::witness(&held).unwrap())
            .unwrap();
        drop(held);
        let files = ["01.mp3".into()];
        assert!(stage.publish(&files, || true).is_err());
        stage.discard().unwrap();
        assert!(std::fs::read_dir(&path).unwrap().next().is_none());

        let mut stage = AudioSet::begin(&destination, "Tracks").unwrap();
        let mut file = stage.create_file("01.mp3").unwrap();
        file.write_all(b"owned output fixture").unwrap();
        drop(file);
        let held = stage.checked_file("01.mp3").unwrap();
        stage
            .mark_verified("01.mp3", &held, AudioSet::witness(&held).unwrap())
            .unwrap();
        drop(held);
        let published = stage.publish(&files, || false).unwrap();
        assert_eq!(
            std::fs::read(published.join("01.mp3")).unwrap(),
            b"owned output fixture"
        );
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
    }

    #[test]
    fn audio_set_never_removes_an_unowned_candidate() {
        let root = fixtures();
        let path = root.path().canonicalize().unwrap();
        let Ok(destination) = AudioDestination::open(&path) else {
            return;
        };
        let mut stage = AudioSet::begin(&destination, "Tracks").unwrap();
        let mut file = stage.create_file("01.mp3").unwrap();
        file.write_all(b"owned").unwrap();
        drop(file);
        let held = stage.checked_file("01.mp3").unwrap();
        stage
            .mark_verified("01.mp3", &held, AudioSet::witness(&held).unwrap())
            .unwrap();
        drop(held);
        let staged = stage.staging_path().unwrap();
        std::fs::rename(staged.join("01.mp3"), staged.join("Owned.mp3")).unwrap();
        std::fs::write(staged.join("01.mp3"), b"foreign").unwrap();
        assert!(stage.publish(&["01.mp3".into()], || false).is_err());
        assert!(stage.discard().is_err());
        assert_eq!(std::fs::read(staged.join("01.mp3")).unwrap(), b"foreign");
        assert_eq!(std::fs::read(staged.join("Owned.mp3")).unwrap(), b"owned");
        assert!(!path.join("Tracks").exists());
    }
}
