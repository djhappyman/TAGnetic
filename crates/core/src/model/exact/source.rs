//! A single held source for all compatibility-reader passes of one exact read.
//! The logical path is deliberately unchanged: extension-directed parser and
//! recovery behaviour must not depend on an artificial descriptor filename.
use std::{
    cell::RefCell,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

struct Source {
    logical: PathBuf,
    file: Arc<File>,
}
thread_local! { static SOURCE: RefCell<Option<Source>> = const { RefCell::new(None) }; }
pub(in crate::model) struct Scope(Option<Source>);
impl Scope {
    // A reentrant public request starts from its own logical path. Only the
    // private passes inside the exact request may borrow its held authority.
    pub fn suspend() -> Self {
        Self(SOURCE.with(|slot| slot.take()))
    }
    pub fn enter(logical: &Path, file: Arc<File>) -> Self {
        Self(SOURCE.with(|slot| {
            slot.replace(Some(Source {
                logical: logical.to_path_buf(),
                file,
            }))
        }))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        SOURCE.with(|slot| slot.replace(self.0.take()));
    }
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    SOURCE.with(|slot| {
        match slot
            .borrow()
            .as_ref()
            .filter(|source| source.logical == path)
        {
            Some(source) => {
                // Compatibility passes are synchronous and serial on this
                // worker. No source handle escapes this scope or is used by a
                // parser concurrently. Each pass starts at zero; cloned handles
                // may share an OS seek cursor, so retaining a pathname-only
                // reopen here would break the held-source authority instead.
                let mut file = source.file.try_clone()?;
                file.seek(SeekFrom::Start(0))?;
                Ok(file)
            }
            None => File::open(path),
        }
    })
}

pub(super) fn read_all(path: &Path) -> io::Result<Vec<u8>> {
    let held = SOURCE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|source| source.logical == path)
    });
    let mut file = open(path)?;
    // The compatibility repair is a whole-file fallback. Under exact read
    // admission it cannot silently expand to an unbounded audio-sized buffer.
    let mut bytes = Vec::new();
    if held {
        const LIMIT: u64 = 64 * 1024 * 1024;
        let too_large =
            || io::Error::other("Whole-file compatibility repair exceeds exact-read budget");
        let len = file.metadata()?.len();
        if len > LIMIT {
            return Err(too_large());
        }
        bytes
            .try_reserve_exact(len as usize)
            .map_err(io::Error::other)?;
        // A regular file can grow after metadata(); bound the read itself too.
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(too_large());
        }
    } else {
        file.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}
