//! Backup, retire and restore recordings through the guarded audio publisher.
//!
//! The recovery manifest and checked copies reach the backup volume before any
//! public source name moves. An interrupted operation retains the manifest and
//! all completed backups. The in-app Undo keeps its ownership proofs in memory.
use super::audio_set::AudioWitness;
use super::{hex, nonce, same_handles, Directory, Private, Stamp};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct AudioReplacement {
    root: Arc<Directory>,
    folder: OsString,
    originals: Vec<Original>,
}

pub struct CompletedReplacement {
    recovery: Arc<Directory>,
    originals: Vec<OriginalLocation>,
    backups: Vec<VerifiedCopy>,
    undo: Mutex<Undo>,
    retained: Vec<PathBuf>,
}

#[derive(Default)]
struct Undo {
    outputs: Vec<BoundFile>,
    archive: Option<Vec<VerifiedCopy>>,
    restored: Vec<BoundFile>,
    complete: bool,
}

struct Original {
    file: BoundFile,
    backup: PathBuf,
}

struct OriginalLocation {
    parent: Arc<Directory>,
    name: OsString,
    path: PathBuf,
}

struct BoundFile {
    parent: Arc<Directory>,
    name: OsString,
    path: PathBuf,
    identity: same_file::Handle,
    stamp: Mutex<Stamp>,
}

type Checksum = [u8; 32];

struct VerifiedCopy {
    file: BoundFile,
    checksum: Checksum,
}

#[derive(Serialize)]
struct RecoveryFile<'a> {
    original: &'a Path,
    saved: &'a Path,
    sha256: String,
}

#[derive(Serialize)]
struct RecoveryManifest<'a> {
    version: u8,
    operation: &'static str,
    files: Vec<RecoveryFile<'a>>,
    outputs: Vec<&'a Path>,
}

fn changed(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        format!("The file changed: {}", path.display()),
    )
}

fn ensure_missing(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("A file or folder already exists at {}", path.display()),
        )),
    }
}

fn directory(path: &Path) -> io::Result<Arc<Directory>> {
    let path = path.canonicalize()?;
    Ok(Arc::new(Directory::open(&path, &path)?))
}

fn check_directory(directory: &Arc<Directory>) -> io::Result<()> {
    if directory.actual_path()? != directory.path || !directory.logical_current(directory)? {
        return Err(changed(&directory.path));
    }
    Ok(())
}

fn sync_directory(directory: &Directory) -> io::Result<()> {
    #[cfg(unix)]
    directory.directory_file()?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

impl BoundFile {
    fn open(path: &Path) -> io::Result<Self> {
        let parent = directory(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?;
        parent.ensure_exclusive_rename()?;
        let name = path.file_name().ok_or(io::ErrorKind::InvalidInput)?;
        Self::in_directory(&parent, name)
    }

    fn in_directory(parent: &Arc<Directory>, name: &OsStr) -> io::Result<Self> {
        check_directory(parent)?;
        let file = parent.open_regular(name, false)?;
        Ok(Self {
            parent: parent.clone(),
            name: name.to_owned(),
            path: parent.path.join(name),
            stamp: Mutex::new(Stamp::of(&file, &file.metadata()?)?),
            identity: same_file::Handle::from_file(file)?,
        })
    }

    fn stamp(&self) -> io::Result<Stamp> {
        self.stamp
            .lock()
            .map(|stamp| *stamp)
            .map_err(|_| io::Error::other("Audio file proof is unavailable"))
    }

    fn current_identity(&self) -> io::Result<File> {
        check_directory(&self.parent)?;
        let file = self.parent.open_regular(&self.name, false)?;
        let identity = same_file::Handle::from_file(file.try_clone()?)?;
        if !same_handles(&self.identity, &identity)? {
            return Err(changed(&self.path));
        }
        Ok(file)
    }

    fn current(&self) -> io::Result<File> {
        let file = self.current_identity()?;
        if self.stamp()? != Stamp::of(&file, &file.metadata()?)? {
            return Err(changed(&self.path));
        }
        Ok(file)
    }

    fn rebind_after_rollback(&self, checksum: Checksum) -> io::Result<()> {
        let file = self.current_identity()?;
        let stamp = Stamp::of(&file, &file.metadata()?)?;
        if stream_checksum(&file, stamp, io::sink())? != checksum {
            return Err(changed(&self.path));
        }
        *self
            .stamp
            .lock()
            .map_err(|_| io::Error::other("Audio file proof is unavailable"))? = stamp;
        Ok(())
    }
}

fn stream_checksum(
    mut source: &File,
    expected: Stamp,
    mut target: impl Write,
) -> io::Result<Checksum> {
    if Stamp::of(source, &source.metadata()?)? != expected {
        return Err(io::Error::other("Audio changed before copying"));
    }
    source.seek(SeekFrom::Start(0))?;
    let mut count = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .filter(|&bytes| bytes <= expected.len)
            .ok_or_else(|| io::Error::other("Audio changed size while copying"))?;
        hash.update(&buffer[..read]);
        target.write_all(&buffer[..read])?;
    }
    if count != expected.len || Stamp::of(source, &source.metadata()?)? != expected {
        return Err(io::Error::other("Audio changed while copying"));
    }
    Ok(hash.finalize().into())
}

fn copy_bound(
    expected: &BoundFile,
    parent: &Arc<Directory>,
    name: &OsStr,
) -> io::Result<VerifiedCopy> {
    let source = expected.current()?;
    let metadata = source.metadata()?;
    check_directory(parent)?;
    let mut writer = parent.open_regular(name, true)?;
    let identity = same_file::Handle::from_file(writer.try_clone()?)?;
    let checksum = stream_checksum(&source, expected.stamp()?, &mut writer)?;
    let mut times = std::fs::FileTimes::new();
    if let Ok(modified) = metadata.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    writer.set_times(times)?;
    writer.set_permissions(metadata.permissions())?;
    writer.sync_all()?;
    drop(writer);
    let saved = BoundFile::in_directory(parent, name)?;
    if !same_handles(&identity, &saved.identity)?
        || stream_checksum(&saved.current()?, saved.stamp()?, io::sink())? != checksum
    {
        return Err(changed(&saved.path));
    }
    Ok(VerifiedCopy {
        file: saved,
        checksum,
    })
}

fn numbered_name(index: usize, path: &Path) -> OsString {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("recording");
    let mut name = tagrex_core::mask::portable_file_name(name, true);
    if name.len() > 240 {
        let extension = path.extension().and_then(OsStr::to_str).unwrap_or("audio");
        name = format!("recording.{extension}");
        name = tagrex_core::mask::portable_file_name(&name, true);
        if name.len() > 240 {
            name = "recording.audio".into();
        }
    }
    format!("{:03} - {name}", index + 1).into()
}

fn create_recovery(parent: &Arc<Directory>, folder: &OsStr) -> io::Result<Arc<Directory>> {
    check_directory(parent)?;
    let mut created = false;
    let directory = parent.child(folder, &mut created).map_err(|error| {
        if created {
            io::Error::other(format!(
                "{error}. Inspect the recovery location at {}",
                parent.path.join(folder).display()
            ))
        } else {
            error
        }
    })?;
    Ok(Arc::new(directory))
}

fn write_manifest<'a>(
    folder: &Arc<Directory>,
    operation: &'static str,
    inputs: impl IntoIterator<Item = &'a BoundFile>,
    copies: &[VerifiedCopy],
    outputs: &[BoundFile],
) -> io::Result<()> {
    check_directory(folder)?;
    let files = inputs
        .into_iter()
        .zip(copies)
        .map(|(old, new)| RecoveryFile {
            original: &old.path,
            saved: &new.file.path,
            sha256: hex(&new.checksum),
        })
        .collect();
    let manifest = RecoveryManifest {
        version: 1,
        operation,
        files,
        outputs: outputs.iter().map(|file| file.path.as_path()).collect(),
    };
    let mut file = folder.open_regular(OsStr::new("recovery.json"), true)?;
    serde_json::to_writer_pretty(&mut file, &manifest)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    sync_directory(folder)
}

struct Capture<'a> {
    original: &'a BoundFile,
    private: Private,
    name: OsString,
    stamp: Stamp,
    checksum: Checksum,
}

fn private_directory(parent: &Arc<Directory>) -> io::Result<Private> {
    let mut orphaned = Vec::new();
    let mut unlocated = false;
    Private::create(parent.clone(), &mut orphaned, &mut unlocated).map_err(|error| {
        if let Some(path) = orphaned.first() {
            io::Error::other(format!("{error}. Check {}", path.display()))
        } else if unlocated {
            io::Error::other(format!(
                "{error}. Check private staging near {}",
                parent.path.display()
            ))
        } else {
            error
        }
    })
}

impl<'a> Capture<'a> {
    fn new(original: &'a BoundFile, checksum: Checksum) -> io::Result<Self> {
        original.current()?;
        let private = private_directory(&original.parent)?;
        check_directory(&original.parent)?;
        let name = match private.capture(&original.parent, &original.name) {
            Ok(name) => name,
            Err(error) => {
                let _ = private.remove_empty();
                return Err(error);
            }
        };
        let checked = (|| {
            let file = private.dir.open_regular(&name, false)?;
            let observed = same_file::Handle::from_file(file)?;
            let file = observed.as_file();
            let stamp = Stamp::of(file, &file.metadata()?)?;
            if !same_handles(&original.identity, &observed)?
                || stream_checksum(file, stamp, io::sink())? != checksum
            {
                return Err(changed(&original.path));
            }
            #[cfg(test)]
            super::observe(super::Event::BackupCaptured, &original.path)?;
            check_directory(&original.parent)?;
            sync_directory(&private.dir)?;
            sync_directory(&original.parent)?;
            Ok(stamp)
        })();
        match checked {
            Ok(stamp) => Ok(Self {
                original,
                private,
                name,
                stamp,
                checksum,
            }),
            Err(error) => {
                let rollback = restore_capture(&private, &name, original)
                    .and_then(|()| original.rebind_after_rollback(checksum));
                Err(retention_error(error, rollback.as_ref().err(), &private))
            }
        }
    }

    fn restore(self) -> io::Result<()> {
        restore_capture(&self.private, &self.name, self.original)?;
        self.original.rebind_after_rollback(self.checksum)
    }

    fn discard(self) -> Vec<PathBuf> {
        let result = (|| {
            let current = self.private.dir.open_regular(&self.name, false)?;
            let identity = same_file::Handle::from_file(current.try_clone()?)?;
            if self.stamp != Stamp::of(&current, &current.metadata()?)?
                || !same_handles(&self.original.identity, &identity)?
            {
                return Err(changed(&self.original.path));
            }
            drop(identity);
            drop(current);
            self.private.dir.unlink_private(&self.name, false)?;
            self.private.forget(false, &self.name);
            self.private.remove_empty()?;
            sync_directory(&self.original.parent)
        })();
        match result {
            Ok(()) => Vec::new(),
            Err(_) => self.private.retained_paths(),
        }
    }
}

fn restore_capture(private: &Private, name: &OsStr, original: &BoundFile) -> io::Result<()> {
    check_directory(&original.parent)?;
    private
        .dir
        .move_exclusive(name, &original.parent, &original.name)?;
    private.forget(false, name);
    let _ = private.remove_empty();
    sync_directory(&original.parent)
}

fn retention_error(error: io::Error, rollback: Option<&io::Error>, private: &Private) -> io::Error {
    match rollback {
        None => error,
        Some(rollback) => {
            let paths = private.retained_paths();
            let location = paths.first().unwrap_or(&private.parent.path);
            io::Error::other(format!(
                "{error}. Could not restore a captured file ({rollback}). Inspect {}",
                location.display()
            ))
        }
    }
}

fn retire<'a>(
    pairs: impl IntoIterator<Item = (&'a BoundFile, Checksum)>,
) -> io::Result<Vec<Capture<'a>>> {
    let mut captured = Vec::<Capture>::new();
    for (file, checksum) in pairs {
        match Capture::new(file, checksum) {
            Ok(capture) => captured.push(capture),
            Err(error) => return Err(rollback(error, captured)),
        }
    }
    Ok(captured)
}

fn rollback(error: io::Error, captured: Vec<Capture<'_>>) -> io::Error {
    let failures: Vec<_> = captured
        .into_iter()
        .rev()
        .filter_map(|capture| {
            let path = capture.private.path().ok();
            capture.restore().err().map(|failure| match path {
                Some(path) => format!("{}: {failure}", path.display()),
                None => failure.to_string(),
            })
        })
        .collect();
    if failures.is_empty() {
        error
    } else {
        io::Error::other(format!(
            "{error}. Could not restore captured files: {}",
            failures.join(", ")
        ))
    }
}

impl AudioReplacement {
    pub fn plan(backup_root: &Path, originals: &[PathBuf]) -> io::Result<Self> {
        if originals.is_empty() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let root = directory(backup_root)?;
        let folder = nonce("TAGnetic audio backup - ")?;
        let backup_dir = root.path.join(&folder);
        let mut seen = BTreeSet::new();
        let originals = originals
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let file = BoundFile::open(path)?;
                if !seen.insert(file.path.clone()) {
                    return Err(io::Error::other("A recording was selected twice"));
                }
                let backup = backup_dir.join(numbered_name(index, &file.path));
                Ok(Original { file, backup })
            })
            .collect::<io::Result<_>>()?;
        Ok(Self {
            root,
            folder,
            originals,
        })
    }

    pub fn backups(&self) -> impl Iterator<Item = (&Path, &Path)> {
        self.originals
            .iter()
            .map(|original| (original.file.path.as_path(), original.backup.as_path()))
    }

    pub fn commit(self, outputs: &[(PathBuf, AudioWitness)]) -> io::Result<CompletedReplacement> {
        let prepare = || {
            let mut seen: BTreeSet<_> = self
                .originals
                .iter()
                .map(|old| old.file.path.as_path())
                .collect();
            let outputs = outputs
                .iter()
                .map(|(path, expected)| {
                    let file = BoundFile::open(path)?;
                    if !seen.insert(path)
                        || file.path != *path
                        || file.stamp()? != expected.0
                        || !same_handles(&file.identity, &expected.1)?
                    {
                        return Err(changed(path));
                    }
                    Ok(file)
                })
                .collect::<io::Result<Vec<_>>>()?;
            if outputs.is_empty() {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            for original in &self.originals {
                original.file.current()?;
            }
            let recovery = create_recovery(&self.root, &self.folder)?;
            Ok((outputs, recovery))
        };
        let (outputs, recovery) = prepare().map_err(|error: io::Error| {
            io::Error::new(
                error.kind(),
                format!("{error}. Published outputs were kept"),
            )
        })?;
        let location = recovery.path.clone();
        let finish = || {
            let backups = self
                .originals
                .iter()
                .map(|old| {
                    let name = old.backup.file_name().ok_or(io::ErrorKind::InvalidInput)?;
                    copy_bound(&old.file, &recovery, name)
                })
                .collect::<io::Result<Vec<_>>>()?;
            write_manifest(
                &recovery,
                "replace",
                self.originals.iter().map(|old| &old.file),
                &backups,
                &outputs,
            )?;
            sync_directory(&self.root)?;
            for output in &outputs {
                output.current()?;
            }
            let captured = retire(
                self.originals
                    .iter()
                    .zip(&backups)
                    .map(|(old, backup)| (&old.file, backup.checksum)),
            )?;
            // Recheck the recovery names and verified outputs at the commit
            // boundary. Originals stay captured and restorable until then.
            for file in backups.iter().map(|copy| &copy.file).chain(&outputs) {
                if let Err(error) = file.current() {
                    return Err(rollback(error, captured));
                }
            }
            let retained = captured.into_iter().flat_map(Capture::discard).collect();
            let originals = self
                .originals
                .into_iter()
                .map(|original| OriginalLocation {
                    parent: original.file.parent,
                    name: original.file.name,
                    path: original.file.path,
                })
                .collect();
            Ok(CompletedReplacement {
                recovery,
                originals,
                backups,
                undo: Mutex::new(Undo {
                    outputs,
                    ..Undo::default()
                }),
                retained,
            })
        };
        finish().map_err(|error| {
            io::Error::other(format!(
                "{error}. Published outputs were kept. Check {} for recovery files",
                location.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests;

fn restore_original(original: &OriginalLocation, backup: &VerifiedCopy) -> io::Result<BoundFile> {
    check_directory(&original.parent)?;
    ensure_missing(&original.path)?;
    let private = private_directory(&original.parent)?;
    let staging = Arc::new(private.dir.child(OsStr::new("restore"), &mut false)?);
    let copy = copy_bound(&backup.file, &staging, &original.name)?;
    if copy.checksum != backup.checksum {
        return Err(changed(&backup.file.path));
    }
    copy.file.current()?;
    check_directory(&original.parent)?;
    staging.move_exclusive(&copy.file.name, &original.parent, &original.name)?;
    // A successful exclusive move is the restore boundary. Best-effort
    // cleanup failures must not invite a second restore over that file.
    let _ = private.dir.unlink_private(OsStr::new("restore"), true);
    let _ = private.remove_empty();
    let _ = sync_directory(&original.parent);
    let restored = BoundFile::in_directory(&original.parent, &original.name)?;
    if !same_handles(&restored.identity, &copy.file.identity)?
        || stream_checksum(&restored.current()?, restored.stamp()?, io::sink())? != backup.checksum
    {
        return Err(changed(&original.path));
    }
    Ok(restored)
}

impl CompletedReplacement {
    pub fn retained_paths(&self) -> &[PathBuf] {
        &self.retained
    }

    pub fn undo(&self) -> io::Result<()> {
        let mut undo = self
            .undo
            .lock()
            .map_err(|_| io::Error::other("Audio replacement state is unavailable"))?;
        if undo.complete {
            return Ok(());
        }
        self.undo_inner(&mut undo).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "{error}. Recovery files are in {}",
                    self.recovery.path.display()
                ),
            )
        })
    }

    fn undo_inner(&self, undo: &mut Undo) -> io::Result<()> {
        for output in &undo.outputs {
            output.current()?;
        }
        for restored in &undo.restored {
            restored.current()?;
        }
        for (original, backup) in self
            .originals
            .iter()
            .zip(&self.backups)
            .skip(undo.restored.len())
        {
            check_directory(&original.parent)?;
            ensure_missing(&original.path)?;
            backup.file.current()?;
        }
        if undo.archive.is_none() {
            let folder = nonce("Exported tracks - ")?;
            let archive = create_recovery(&self.recovery, &folder)?;
            let copies = undo
                .outputs
                .iter()
                .enumerate()
                .map(|(index, output)| {
                    copy_bound(output, &archive, &numbered_name(index, &output.path))
                })
                .collect::<io::Result<Vec<_>>>()?;
            write_manifest(&archive, "undo", &undo.outputs, &copies, &[])?;
            sync_directory(&self.recovery)?;
            undo.archive = Some(copies);
        }
        let archive = undo.archive.as_ref().ok_or(io::ErrorKind::NotFound)?;
        for copy in archive {
            copy.file.current()?;
        }
        for (original, backup) in self
            .originals
            .iter()
            .zip(&self.backups)
            .skip(undo.restored.len())
        {
            undo.restored.push(restore_original(original, backup)?);
        }
        let captured = retire(
            undo.outputs
                .iter()
                .zip(archive)
                .map(|(output, backup)| (output, backup.checksum)),
        )?;
        for copy in archive {
            if let Err(error) = copy.file.current() {
                return Err(rollback(error, captured));
            }
        }
        for capture in captured {
            capture.discard();
        }
        undo.outputs.clear();
        undo.restored.clear();
        undo.archive = None;
        undo.complete = true;
        Ok(())
    }
}
