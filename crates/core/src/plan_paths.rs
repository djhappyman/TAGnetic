//! Operation-scoped directory capabilities. Public progress callbacks cannot
//! redirect a later mutation by replacing an ancestor with a directory link.
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs::File,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

thread_local! { static ACTIVE: RefCell<Option<Authority>> = const { RefCell::new(None) }; }
#[cfg(test)]
type RenameHook = Box<dyn FnMut(&Path, &Path)>;
#[cfg(test)]
thread_local! { static RENAME_HOOK: RefCell<Option<RenameHook>> = const { RefCell::new(None) }; }
#[cfg(test)]
pub(super) struct RenameHookGuard(Option<RenameHook>);
#[cfg(test)]
impl Drop for RenameHookGuard {
    fn drop(&mut self) {
        RENAME_HOOK.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}
#[cfg(test)]
pub(super) fn rename_hook(hook: RenameHook) -> RenameHookGuard {
    RenameHookGuard(RENAME_HOOK.with(|slot| slot.replace(Some(hook))))
}
#[cfg(test)]
fn before_rename(from: &Path, to: &Path) {
    RENAME_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(from, to);
        }
    });
}

pub(super) struct Scope(Option<Authority>);
impl Scope {
    pub(super) fn enter(
        roots: &[PathBuf],
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> io::Result<Self> {
        if roots.len() > 256 {
            return Err(io::Error::other(
                "Mutation root budget exceeded; use at most 256 approved roots",
            ));
        }
        let mut authority = Authority {
            roots: roots.to_vec(),
            directories: HashMap::new(),
            aliases: HashMap::new(),
            #[cfg(unix)]
            directory_paths: std::collections::BTreeSet::new(),
            #[cfg(unix)]
            aliases_by_directory: std::collections::BTreeMap::new(),
            optional: HashSet::new(),
            anchors: HashMap::new(),
            step_pins: Vec::new(),
            step_files: HashMap::new(),
            #[cfg(unix)]
            fallback_pins: 0,
        };
        let mut planned = HashSet::new();
        for root in roots {
            for parent in root.ancestors() {
                planned.insert(parent.to_path_buf());
            }
            if let Some(file) = authority.walk(root, false, false, &mut Vec::new())? {
                authority.anchors.insert(root.clone(), file);
            }
        }
        for path in paths {
            if let Some(parent) = path.parent() {
                if parent
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
                {
                    continue;
                }
                let parent = absolute(parent)?;
                let resolved = resolve_existing(&parent)?;
                if roots.iter().any(|root| resolved.starts_with(root)) {
                    for ancestor in resolved.ancestors() {
                        planned.insert(ancestor.to_path_buf());
                    }
                    if planned.len() > MAX_DIRECTORIES {
                        return Err(io::Error::other(
                            "Mutation directory budget exceeded; use a smaller batch",
                        ));
                    }
                    authority.resolved(&parent)?;
                    if authority
                        .walk(&resolved, false, true, &mut Vec::new())?
                        .is_none()
                    {
                        authority.optional.insert(parent);
                    }
                }
            }
        }
        Ok(Self(ACTIVE.with(|slot| slot.replace(Some(authority)))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

struct Authority {
    roots: Vec<PathBuf>,
    #[cfg(unix)]
    directories: HashMap<PathBuf, DirectoryIdentity>,
    #[cfg(windows)]
    directories: HashMap<PathBuf, Arc<File>>,
    aliases: HashMap<PathBuf, PathBuf>,
    // Ordered secondary indexes keep subtree retirement proportional to that
    // subtree rather than scanning every unrelated folder after every move.
    #[cfg(unix)]
    directory_paths: std::collections::BTreeSet<PathBuf>,
    #[cfg(unix)]
    aliases_by_directory: std::collections::BTreeMap<PathBuf, HashSet<PathBuf>>,
    optional: HashSet<PathBuf>,
    anchors: HashMap<PathBuf, Arc<File>>,
    step_pins: Vec<Arc<File>>,
    step_files: HashMap<PathBuf, Arc<File>>,
    #[cfg(unix)]
    fallback_pins: usize,
}
#[cfg(unix)]
const MAX_DIRECTORIES: usize = 100_000;
#[cfg(windows)]
const MAX_DIRECTORIES: usize = 4096;
#[cfg(unix)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    created: Option<std::time::SystemTime>,
    // Without a birth generation, retain the inode instead of allowing reuse.
    retained: Option<Arc<File>>,
}
pub(super) struct Step(Vec<Arc<File>>, HashMap<PathBuf, Arc<File>>);
impl Step {
    pub(super) fn enter(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Self> {
        Self::capture(paths, false)
    }
    pub(super) fn entries(paths: impl IntoIterator<Item = PathBuf>) -> io::Result<Self> {
        Self::capture(paths, true)
    }
    fn capture(paths: impl IntoIterator<Item = PathBuf>, entries: bool) -> io::Result<Self> {
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(authority) = slot.as_mut() else {
                return Ok(Self(Vec::new(), HashMap::new()));
            };
            let mut pins = Vec::new();
            let mut seen = HashSet::new();
            let mut files = HashMap::new();
            for path in paths {
                if let Some(parent) = path.parent() {
                    if seen.insert(parent.to_path_buf()) {
                        // A destination parent may not exist until the step
                        // creates it; retain its nearest existing ancestor.
                        let mut existing = parent;
                        while !existing.exists() {
                            existing = existing.parent().ok_or_else(changed)?;
                        }
                        pins.push(authority.directory(existing)?);
                    }
                }
                match if entries {
                    authority.entry(&path)
                } else {
                    authority.step_file(&path)
                } {
                    Ok(file) => {
                        files.insert(absolute(&path)?, Arc::new(file));
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(Self(
                std::mem::replace(&mut authority.step_pins, pins),
                std::mem::replace(&mut authority.step_files, files),
            ))
        })
    }
}
impl Drop for Step {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            if let Some(authority) = slot.borrow_mut().as_mut() {
                authority.step_pins = std::mem::take(&mut self.0);
                authority.step_files = std::mem::take(&mut self.1);
            }
        });
    }
}
fn resolve_existing(path: &Path) -> io::Result<PathBuf> {
    let mut existing = path;
    let mut tail = Vec::new();
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tail.push(existing.file_name().ok_or_else(changed)?.to_owned());
                existing = existing.parent().ok_or_else(changed)?;
            }
            Err(error) => return Err(error),
        }
    }
    let mut resolved = std::fs::canonicalize(existing)?;
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
impl Authority {
    #[cfg(unix)]
    fn step_file(&mut self, path: &Path) -> io::Result<File> {
        self.open(path, false, false)
    }
    fn resolved(&mut self, path: &Path) -> io::Result<PathBuf> {
        let path = absolute(path)?;
        let current = resolve_existing(&path)?;
        if let Some(previous) = self.aliases.get(&path) {
            if previous != &current {
                return Err(changed());
            }
        } else {
            if self.aliases.len() >= MAX_DIRECTORIES {
                return Err(io::Error::other(
                    "Mutation directory budget exceeded; use a smaller batch",
                ));
            }
            #[cfg(unix)]
            self.aliases_by_directory
                .entry(current.clone())
                .or_default()
                .insert(path.clone());
            self.aliases.insert(path, current.clone());
        }
        if !self.roots.iter().any(|root| current.starts_with(root)) {
            return Err(changed());
        }
        Ok(current)
    }
}
fn absolute(path: &Path) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Parent traversal is not supported by mutation capabilities",
        ));
    }
    Ok(path)
}
fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Directory ancestry changed during the operation",
    )
}

#[cfg(unix)]
impl Authority {
    fn walk(
        &mut self,
        path: &Path,
        create: bool,
        missing_ok: bool,
        created: &mut Vec<PathBuf>,
    ) -> io::Result<Option<Arc<File>>> {
        use rustix::fs::{mkdirat, openat, Mode, OFlags};
        use std::os::unix::fs::MetadataExt;
        let path = absolute(path)?;
        let mut current = PathBuf::from("/");
        let mut directory = Arc::new(File::open("/")?);
        for component in path.components() {
            let Component::Normal(name) = component else {
                continue;
            };
            let next = current.join(name);
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let opened = match openat(directory.as_ref(), name, flags, Mode::empty()) {
                Ok(file) => file,
                Err(error)
                    if error == rustix::io::Errno::NOENT
                        && create
                        && self.roots.iter().any(|root| next.starts_with(root)) =>
                {
                    mkdirat(directory.as_ref(), name, Mode::from_bits_truncate(0o755))?;
                    created.push(next.clone());
                    openat(directory.as_ref(), name, flags, Mode::empty())?
                }
                Err(error) if error == rustix::io::Errno::NOENT && missing_ok => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let opened = Arc::new(File::from(opened));
            if let Some(previous) = self.directories.get(&next) {
                let now = opened.metadata()?;
                if (previous.device, previous.inode) != (now.dev(), now.ino())
                    || previous
                        .created
                        .is_some_and(|before| now.created().ok() != Some(before))
                {
                    return Err(changed());
                }
            } else {
                if self.directories.len() >= MAX_DIRECTORIES {
                    return Err(io::Error::other(
                        "Mutation directory budget exceeded; use a smaller batch",
                    ));
                }
                let metadata = opened.metadata()?;
                let created = metadata.created().ok();
                let retained = if created.is_none() {
                    if self.fallback_pins >= 256 {
                        return Err(io::Error::other("This filesystem needs retained directory handles; mutation directory budget exceeded"));
                    }
                    self.fallback_pins += 1;
                    Some(opened.clone())
                } else {
                    None
                };
                self.directory_paths.insert(next.clone());
                self.directories.insert(
                    next.clone(),
                    DirectoryIdentity {
                        device: metadata.dev(),
                        inode: metadata.ino(),
                        created,
                        retained,
                    },
                );
            }
            current = next;
            directory = opened;
        }
        Ok(Some(directory))
    }
    fn directory(&mut self, path: &Path) -> io::Result<Arc<File>> {
        let original = absolute(path)?;
        let path = self.resolved(path)?;
        let file = self
            .walk(&path, false, false, &mut Vec::new())?
            .ok_or_else(changed)?;
        self.optional.remove(&original);
        Ok(file)
    }
    fn parent(&mut self, path: &Path) -> io::Result<(Arc<File>, std::ffi::OsString)> {
        let path = absolute(path)?;
        Ok((
            self.directory(path.parent().ok_or_else(changed)?)?,
            path.file_name().ok_or_else(changed)?.to_owned(),
        ))
    }
    fn open(&mut self, path: &Path, write: bool, create_new: bool) -> io::Result<File> {
        use rustix::fs::{openat, Mode, OFlags};
        let (parent, name) = self.parent(path)?;
        let mut flags = if write { OFlags::RDWR } else { OFlags::RDONLY };
        flags |= OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if create_new {
            flags |= OFlags::CREATE | OFlags::EXCL;
        }
        let file = File::from(openat(
            parent.as_ref(),
            &name,
            flags,
            Mode::from_bits_truncate(0o666),
        )?);
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Expected a regular media file",
            ));
        }
        if !create_new {
            if let Some(expected) = self.step_files.get(&absolute(path)?) {
                use std::os::unix::fs::MetadataExt;
                let before = expected.metadata()?;
                let now = file.metadata()?;
                if (before.dev(), before.ino()) != (now.dev(), now.ino()) {
                    return Err(changed());
                }
            }
        }
        Ok(file)
    }
    fn entry(&mut self, path: &Path) -> io::Result<File> {
        use rustix::fs::{openat, Mode, OFlags};
        let (parent, name) = self.parent(path)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let flags = OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let file = File::from(openat(parent.as_ref(), &name, flags, Mode::empty())?);
        let kind = file.metadata()?.file_type();
        if !kind.is_file() && !kind.is_symlink() {
            return Err(changed());
        }
        if let Some(expected) = self.step_files.get(&absolute(path)?) {
            use std::os::unix::fs::MetadataExt;
            let expected = expected.metadata()?;
            let actual = file.metadata()?;
            if (expected.dev(), expected.ino()) != (actual.dev(), actual.ino()) {
                return Err(changed());
            }
        }
        Ok(file)
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let (source, source_name) = self.parent(from)?;
        let (destination, destination_name) = self.parent(to)?;
        if from == to {
            return Ok(());
        }
        let before = rustix::fs::statat(
            source.as_ref(),
            &source_name,
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )?;
        let kind = rustix::fs::FileType::from_raw_mode(before.st_mode);
        let permitted_link = self.step_files.get(&absolute(from)?).is_some_and(|file| {
            file.metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
        });
        if kind != rustix::fs::FileType::RegularFile
            && !(kind == rustix::fs::FileType::Symlink && permitted_link)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Transfer source must remain a regular file",
            ));
        }
        if let Some(expected) = self.step_files.get(&absolute(from)?) {
            // Compare native stat fields on both sides: Darwin's dev_t is
            // signed and differs from std's cross-Unix u64 representation.
            let expected = rustix::fs::fstat(expected)?;
            if (expected.st_dev, expected.st_ino) != (before.st_dev, before.st_ino) {
                return Err(changed());
            }
        }
        let folded_same_entry = (|| -> io::Result<bool> {
            use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
            let source_parent = source.metadata()?;
            let destination_parent = destination.metadata()?;
            if (source_parent.dev(), source_parent.ino())
                != (destination_parent.dev(), destination_parent.ino())
            {
                return Ok(false);
            }
            let after = match rustix::fs::statat(
                destination.as_ref(),
                &destination_name,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(after) => after,
                Err(error) if error == rustix::io::Errno::NOENT => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino) {
                return Ok(false);
            }
            let mut entries = rustix::fs::Dir::read_from(destination.as_ref())?;
            for entry in &mut entries {
                if entry?.file_name().to_bytes() == destination_name.as_bytes() {
                    return Ok(false);
                }
            }
            Ok(true)
        })()?;
        #[cfg(test)]
        before_rename(from, to);
        if folded_same_entry {
            return rustix::fs::renameat(
                source.as_ref(),
                &source_name,
                destination.as_ref(),
                &destination_name,
            )
            .map_err(Into::into);
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios"
        ))]
        {
            rustix::fs::renameat_with(
                source.as_ref(),
                &source_name,
                destination.as_ref(),
                &destination_name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(Into::into)
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "macos",
            target_os = "ios"
        )))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Atomic no-replace rename is unavailable",
            ))
        }
    }
    fn remove(&mut self, path: &Path, directory: bool) -> io::Result<()> {
        let (parent, name) = self.parent(path)?;
        let result = rustix::fs::unlinkat(
            parent.as_ref(),
            &name,
            if directory {
                rustix::fs::AtFlags::REMOVEDIR
            } else {
                rustix::fs::AtFlags::empty()
            },
        )
        .map_err(Into::into);
        if result.is_ok() && directory {
            let path = absolute(path)?;
            let resolved = self.aliases.get(&path).cloned().unwrap_or(path);
            let paths: Vec<_> = self
                .directory_paths
                .range(resolved.clone()..)
                .take_while(|path| path.starts_with(&resolved))
                .cloned()
                .collect();
            for path in paths {
                self.directory_paths.remove(&path);
                if self
                    .directories
                    .remove(&path)
                    .is_some_and(|identity| identity.retained.is_some())
                {
                    self.fallback_pins -= 1;
                }
            }
            let canonical: Vec<_> = self
                .aliases_by_directory
                .range(resolved.clone()..)
                .take_while(|(path, _)| path.starts_with(&resolved))
                .map(|(path, _)| path.clone())
                .collect();
            for path in canonical {
                if let Some(aliases) = self.aliases_by_directory.remove(&path) {
                    for alias in aliases {
                        self.aliases.remove(&alias);
                    }
                }
            }
        }
        result
    }
    fn mkdir(&mut self, path: &Path) -> io::Result<()> {
        let (parent, name) = self.parent(path)?;
        rustix::fs::mkdirat(
            parent.as_ref(),
            &name,
            rustix::fs::Mode::from_bits_truncate(0o700),
        )?;
        self.directory(path)?;
        Ok(())
    }
}

#[cfg(windows)]
impl Authority {
    fn entry(&mut self, path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        let actual = self
            .resolved(path.parent().ok_or_else(changed)?)?
            .join(path.file_name().ok_or_else(changed)?);
        self.directory(actual.parent().ok_or_else(changed)?)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2 | 0x4) // read, write and delete sharing for transport
            .custom_flags(0x00200000) // OPEN_REPARSE_POINT: inspect the entry, never follow it
            .open(actual)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() && !metadata.file_type().is_symlink() {
            return Err(changed());
        }
        Ok(file)
    }
    fn step_file(&mut self, path: &Path) -> io::Result<File> {
        let file = self.entry(path)?;
        if !file.metadata()?.is_file() || file.metadata()?.file_type().is_symlink() {
            return Err(changed());
        }
        Ok(file)
    }
    fn walk(
        &mut self,
        path: &Path,
        create: bool,
        missing_ok: bool,
        created: &mut Vec<PathBuf>,
    ) -> io::Result<Option<Arc<File>>> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let path = absolute(path)?;
        let mut current = PathBuf::new();
        let mut last = None;
        for component in path.components() {
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0x1 | 0x2) // deny ancestor/held-metadata replacement
                .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                .open(&current);
            let file = match opened {
                Ok(file) => file,
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && create
                        && self.roots.iter().any(|root| current.starts_with(root)) =>
                {
                    std::fs::create_dir(&current)?;
                    created.push(current.clone());
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0x1 | 0x2) // deny ancestor/held-metadata replacement
                        .custom_flags(0x02000000 | 0x00200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                        .open(&current)?
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound && missing_ok => {
                    return Ok(None)
                }
                Err(error) => return Err(error),
            };
            let metadata = file.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
                return Err(changed());
            }
            if let Some(previous) = self.directories.get(&current) {
                if same_file::Handle::from_file(previous.try_clone()?)?
                    != same_file::Handle::from_file(file.try_clone()?)?
                {
                    return Err(changed());
                }
            }
            let file = Arc::new(file);
            self.directories.insert(current.clone(), file.clone());
            last = Some(file);
        }
        Ok(last)
    }
    fn directory(&mut self, path: &Path) -> io::Result<Arc<File>> {
        let original = absolute(path)?;
        let path = self.resolved(path)?;
        let file = self
            .walk(&path, false, false, &mut Vec::new())?
            .ok_or_else(changed)?;
        self.optional.remove(&original);
        Ok(file)
    }
    fn open(&mut self, path: &Path, write: bool, create_new: bool) -> io::Result<File> {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        let actual = self
            .resolved(path.parent().ok_or_else(changed)?)?
            .join(path.file_name().ok_or_else(changed)?);
        self.directory(actual.parent().ok_or_else(changed)?)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .create_new(create_new)
            .share_mode(0x1 | 0x2) // deny ancestor/held-metadata replacement
            .custom_flags(0x00200000) // OPEN_REPARSE_POINT: inspect the entry, never follow it
            .open(&actual)?;
        if !file.metadata()?.is_file() || file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(changed());
        }
        if !create_new {
            if let Some(expected) = self.step_files.get(&absolute(path)?) {
                if same_file::Handle::from_file(expected.try_clone()?)?
                    != same_file::Handle::from_file(file.try_clone()?)?
                {
                    return Err(changed());
                }
            }
        }
        Ok(file)
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let actual_from = self
            .resolved(from.parent().ok_or_else(changed)?)?
            .join(from.file_name().ok_or_else(changed)?);
        let actual_to = self
            .resolved(to.parent().ok_or_else(changed)?)?
            .join(to.file_name().ok_or_else(changed)?);
        self.directory(actual_from.parent().ok_or_else(changed)?)?;
        self.directory(actual_to.parent().ok_or_else(changed)?)?;
        let leaf = self.entry(from)?;
        if leaf.metadata()?.file_type().is_symlink()
            && !self.step_files.get(&absolute(from)?).is_some_and(|file| {
                file.metadata()
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
            })
        {
            return Err(changed());
        }
        if let Some(expected) = self.step_files.get(&absolute(from)?) {
            if same_file::Handle::from_file(expected.try_clone()?)?
                != same_file::Handle::from_file(leaf.try_clone()?)?
            {
                return Err(changed());
            }
        }
        #[cfg(test)]
        before_rename(from, to);
        super::native_rename_noreplace(&actual_from, &actual_to)
    }
    fn remove(&mut self, path: &Path, directory: bool) -> io::Result<()> {
        let actual = self
            .resolved(path.parent().ok_or_else(changed)?)?
            .join(path.file_name().ok_or_else(changed)?);
        self.directory(actual.parent().ok_or_else(changed)?)?;
        if directory {
            self.directories.retain(|key, _| !key.starts_with(&actual));
            let result = std::fs::remove_dir(&actual);
            if result.is_ok() {
                self.aliases
                    .retain(|_, canonical| !canonical.starts_with(&actual));
            }
            result
        } else {
            std::fs::remove_file(&actual)
        }
    }
    fn mkdir(&mut self, path: &Path) -> io::Result<()> {
        let actual = self
            .resolved(path.parent().ok_or_else(changed)?)?
            .join(path.file_name().ok_or_else(changed)?);
        self.directory(actual.parent().ok_or_else(changed)?)?;
        std::fs::create_dir(&actual)?;
        self.directory(&actual)?;
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("Pinned mutation capabilities require a Unix or Windows implementation");

pub(super) fn open(path: &Path, write: bool, create_new: bool) -> io::Result<File> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => authority.open(path, write, create_new),
        None => std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .create_new(create_new)
            .open(path),
    })
}
pub(super) fn directory(path: &Path) -> io::Result<File> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => authority.directory(path)?.try_clone(),
        None => Err(changed()),
    })
}
pub(super) fn entry(path: &Path) -> io::Result<File> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => authority.entry(path),
        // This seam is only called within an operation capability scope.
        None => Err(changed()),
    })
}
pub(super) fn rename(from: &Path, to: &Path) -> io::Result<Option<()>> {
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|authority| authority.rename(from, to))
            .transpose()
    })
}
pub(super) fn remove(path: &Path, directory: bool) -> io::Result<()> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => authority.remove(path, directory),
        None => {
            if directory {
                std::fs::remove_dir(path)
            } else {
                std::fs::remove_file(path)
            }
        }
    })
}
pub(super) fn mkdir(path: &Path) -> io::Result<()> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => authority.mkdir(path),
        None => std::fs::create_dir(path),
    })
}
pub(super) fn create_all(path: &Path) -> io::Result<Vec<PathBuf>> {
    ACTIVE.with(|slot| match slot.borrow_mut().as_mut() {
        Some(authority) => {
            let mut created = Vec::new();
            let resolved = authority.resolved(path)?;
            authority.directory(
                authority
                    .roots
                    .iter()
                    .find(|root| resolved.starts_with(root))
                    .ok_or_else(changed)?
                    .clone()
                    .as_path(),
            )?;
            authority.walk(&resolved, true, false, &mut created)?;
            authority.optional.remove(&absolute(path)?);
            Ok(created)
        }
        None => {
            std::fs::create_dir_all(path)?;
            Ok(Vec::new())
        }
    })
}
pub(super) fn sync(path: &Path) -> io::Result<Option<()>> {
    ACTIVE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|authority| {
                let file = authority.directory(path)?;
                #[cfg(unix)]
                {
                    file.sync_all()
                }
                #[cfg(not(unix))]
                {
                    let _ = file;
                    Ok(())
                }
            })
            .transpose()
    })
}
/// Hold the media object for one logical metadata request. Individual model IO
/// sessions reopen through this authority and compare against the held object;
/// descriptor pseudo-paths are not equivalent to fresh opens on Darwin.
pub(super) fn metadata_file(path: &Path, write: bool) -> io::Result<Option<File>> {
    if ACTIVE.with(|slot| slot.borrow().is_some()) {
        open(path, write, false).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn validate() -> io::Result<()> {
    ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(authority) = slot.as_mut() else {
            return Ok(());
        };
        let paths: Vec<_> = authority.aliases.keys().cloned().collect();
        for path in paths {
            if authority.optional.contains(&path)
                && std::fs::symlink_metadata(&path)
                    .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            {
                continue;
            }
            authority.directory(&path)?;
        }
        Ok(())
    })
}

#[cfg(all(test, unix))]
mod cleanup_index_tests {
    use super::*;
    fn assert_indexes(authority: &Authority) {
        let paths = authority
            .directories
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(paths, authority.directory_paths);
        let mut reverse = std::collections::BTreeMap::<PathBuf, HashSet<PathBuf>>::new();
        for (alias, canonical) in &authority.aliases {
            reverse
                .entry(canonical.clone())
                .or_default()
                .insert(alias.clone());
        }
        assert_eq!(reverse, authority.aliases_by_directory);
        assert_eq!(
            authority.fallback_pins,
            authority
                .directories
                .values()
                .filter(|identity| identity.retained.is_some())
                .count()
        );
    }
    #[test]
    fn ordered_path_subtrees_match_full_component_prefix_scan() {
        let names = [
            "a",
            "a-0",
            "a.foo",
            "a0",
            "aa",
            "álbum",
            "a\\b",
            "a b",
            "a(1)",
            "a[0]",
            "a👍",
            "a\u{0301}",
        ];
        let mut paths = std::collections::BTreeSet::new();
        let root = PathBuf::from("/fixture");
        paths.insert(root.clone());
        for first in names {
            let first = root.join(first);
            paths.insert(first.clone());
            for second in names {
                let second = first.join(second);
                paths.insert(second.clone());
                for third in names {
                    paths.insert(second.join(third));
                }
            }
        }
        for parent in &paths {
            let indexed = paths
                .range(parent.clone()..)
                .take_while(|path| path.starts_with(parent))
                .cloned()
                .collect::<Vec<_>>();
            let expected = paths
                .iter()
                .filter(|path| path.starts_with(parent))
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(indexed, expected, "component prefix {parent:?}");
        }
    }
    #[test]
    fn retirement_keeps_unrelated_alias_authority_and_fallback_pins() {
        use std::os::unix::fs::symlink;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-cleanup-index-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        // Match public Executor admission before entering this private scope.
        let root = std::fs::canonicalize(root).unwrap();
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _fixture = Fixture(root.clone());
        for name in ["a", "aa", "a-0", "other"] {
            std::fs::create_dir(root.join(name)).unwrap();
            std::fs::write(root.join(name).join("data"), b"fixture").unwrap();
        }
        let private = root.join("a/.owned");
        let aa = root.join("aa");
        let scope = Scope::enter(
            std::slice::from_ref(&root),
            [
                root.join("a/data"),
                aa.join("data"),
                root.join("a-0/data"),
                private.join("entry"),
            ],
        )
        .unwrap();
        mkdir(&private).unwrap();
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let authority = slot.as_mut().unwrap();
            assert_indexes(authority);
            for directory in [&private, &aa] {
                let retained = authority.directory(directory).unwrap();
                let identity = authority.directories.get_mut(directory).unwrap();
                if identity.retained.is_none() {
                    authority.fallback_pins += 1;
                }
                identity.retained = Some(retained);
            }
            assert_indexes(authority);
        });
        remove(&private, true).unwrap();
        ACTIVE.with(|slot| {
            let slot = slot.borrow();
            let authority = slot.as_ref().unwrap();
            assert_indexes(authority);
            assert!(authority.directories.get(&aa).unwrap().retained.is_some());
            assert_eq!(authority.fallback_pins, 1);
            assert!(!authority.directory_paths.contains(&private));
        });
        // The file identity is deliberately the same: only retained directory
        // alias authority can refuse this redirection after unrelated cleanup.
        std::fs::remove_file(root.join("other/data")).unwrap();
        std::fs::hard_link(aa.join("data"), root.join("other/data")).unwrap();
        std::fs::rename(&aa, root.join("preserved-aa")).unwrap();
        symlink(root.join("other"), &aa).unwrap();
        assert!(entry(&aa.join("data")).is_err());
        assert!(validate().is_err());
        assert_eq!(
            std::fs::read(root.join("preserved-aa/data")).unwrap(),
            b"fixture"
        );
        drop(scope);
    }
}
