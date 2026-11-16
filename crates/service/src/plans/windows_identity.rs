//! Stable Windows file identity, captured alongside metadata from one handle.
//!
//! A copy can preserve timestamps and length. ReFS also requires its full 128-bit
//! ID: the older 64-bit file index is explicitly not guaranteed unique on ReFS.

use serde::Serialize;
use std::{fs::File, fs::Metadata, io, mem, os::windows::io::AsRawHandle, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW, BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) enum WindowsIdentity {
    Extended { volume: u64, file: [u8; 16] },
    Legacy { volume: u32, file: u64 },
}

pub(super) fn read(path: &Path) -> io::Result<(Metadata, WindowsIdentity)> {
    // File::open retains Rust's read/write/delete sharing, so identity capture
    // does not interfere with other readers or the native writer's own handles.
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let identity = from_file(&file)?;
    Ok((metadata, identity))
}

fn from_file(file: &File) -> io::Result<WindowsIdentity> {
    let handle = file.as_raw_handle();
    let mut info = FILE_ID_INFO::default();
    // SAFETY: file keeps the handle alive; info is writable for exactly its size.
    let success = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if success != 0 && info.FileId.Identifier != [0; 16] {
        return Ok(WindowsIdentity::Extended {
            volume: info.VolumeSerialNumber,
            file: info.FileId.Identifier,
        });
    }
    if success == 0 {
        let error = io::Error::last_os_error();
        // Only unsupported information classes permit the documented older
        // query. Access/I/O failures must not silently downgrade the check.
        if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 120 | 124)) {
            return Err(error);
        }
    }

    // Older local FAT/exFAT/NTFS implementations can expose only the 64-bit
    // index. Positively identify the filesystem before accepting it. This API
    // does not support SMB; modern SMB supports FileIdInfo above. Providers
    // without a trustworthy ID fail closed rather than rely on timestamps.
    let mut filesystem = [0u16; 32];
    // SAFETY: the live handle is valid, optional outputs are null, and the
    // filesystem buffer has the advertised number of UTF-16 elements.
    if unsafe {
        GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let end = filesystem
        .iter()
        .position(|v| *v == 0)
        .unwrap_or(filesystem.len());
    let name = String::from_utf16_lossy(&filesystem[..end]);
    if !supports_legacy_id(&name) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem does not expose a supported stable file identity",
        ));
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file keeps the handle alive and info is a valid writable output.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    if index == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unavailable",
        ));
    }
    Ok(WindowsIdentity::Legacy {
        volume: info.dwVolumeSerialNumber,
        file: index,
    })
}

fn supports_legacy_id(filesystem: &str) -> bool {
    ["FAT", "FAT32", "EXFAT", "NTFS"]
        .iter()
        .any(|name| filesystem.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_reads_and_hard_links_identify_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original");
        std::fs::write(&path, b"same bytes").unwrap();
        let first = read(&path).unwrap().1;
        assert_eq!(read(&path).unwrap().1, first);
        let link = dir.path().join("link");
        std::fs::hard_link(&path, &link).unwrap();
        assert_eq!(read(&link).unwrap().1, first);
    }

    #[test]
    fn replacing_with_a_copy_changes_identity_despite_equal_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original");
        let replacement = dir.path().join("replacement");
        std::fs::write(&path, b"same bytes").unwrap();
        let (before, before_id) = read(&path).unwrap();
        std::fs::copy(&path, &replacement).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let (after, after_id) = read(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_ne!(before_id, after_id);
    }

    #[test]
    fn legacy_fallback_never_truncates_refs_or_unknown_filesystem_ids() {
        for name in ["NTFS", "FAT", "FAT32", "exFAT"] {
            assert!(supports_legacy_id(name));
        }
        for name in ["ReFS", "SMB", "", "unknown"] {
            assert!(!supports_legacy_id(name));
        }
    }
}
