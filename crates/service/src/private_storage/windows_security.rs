//! Windows counterparts of Unix owner-only modes. File permissions and pipe
//! permissions are separate: protecting a pipe does not protect its token file.
fn permission_denied() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Service state must be private to the current user",
    )
}
use std::{
    ffi::c_void,
    fs::{self, File},
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, GetSecurityInfo, SetNamedSecurityInfoW, SE_FILE_OBJECT,
        },
        GetAce, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACL,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: every instance wraps an allocation returned by a Windows
        // API documented to release with LocalFree (or a null pointer).
        unsafe { LocalFree(self.0) };
    }
}

// TokenOwner may be an administrative group even when TokenUser is a user
// account. Specify both owner and DACL at creation instead of creating with
// that default owner and then trying to take ownership of an existing object.
struct PrivateSecurity {
    _allocation: LocalAllocation,
    attributes: SECURITY_ATTRIBUTES,
}

impl PrivateSecurity {
    fn new() -> io::Result<Self> {
        let current = current_sid()?;
        let sddl: Vec<u16> = format!("O:{current}D:P(A;OICI;GA;;;{current})(A;OICI;GA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: input is NUL-terminated; the returned descriptor is retained
        // with its SECURITY_ATTRIBUTES for the complete creation call.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            _allocation: LocalAllocation(descriptor),
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }
}

pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            return if metadata.is_dir() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err(permission_denied())
            };
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_private_dir_all(parent)?;
    }
    let security = PrivateSecurity::new()?;
    let encoded = wide_path(path)?;
    // SAFETY: path is NUL-terminated and the owned descriptor/attributes stay
    // alive through CreateDirectoryW. No inherited/default owner is used.
    if unsafe { CreateDirectoryW(encoded.as_ptr(), &security.attributes) } == 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        // An object that raced creation must already have the required
        // identity/permissions; never take over a replacement's ownership.
        return super::verify_private(path, true);
    }
    super::verify_private(path, true)
}

pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    let security = PrivateSecurity::new()?;
    let encoded = wide_path(path)?;
    // SAFETY: the initialized descriptor remains alive, CREATE_NEW never opens
    // an existing object, and the returned valid handle is owned exactly once.
    let handle = unsafe {
        CreateFileW(
            encoded.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &security.attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW transferred a valid, uniquely owned file handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn wide_path(path: &Path) -> Result<Vec<u16>, std::io::Error> {
    let value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(permission_denied());
    }
    Ok(value.into_iter().chain(Some(0)).collect())
}

unsafe fn sid_string(sid: *mut c_void) -> Result<String, std::io::Error> {
    if sid.is_null() {
        return Err(permission_denied());
    }
    let mut string = ptr::null_mut();
    // SAFETY: callers provide a valid SID returned in an owned descriptor or
    // TOKEN_USER buffer; out-pointer is initialized and owned below.
    if unsafe { ConvertSidToStringSidW(sid, &mut string) } == 0 {
        return Err(permission_denied());
    }
    let _allocation = LocalAllocation(string.cast());
    let mut length = 0;
    // SAFETY: ConvertSidToStringSidW returns a NUL-terminated UTF-16 string.
    while unsafe { *string.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the string allocation remains alive through this copy.
    String::from_utf16(unsafe { std::slice::from_raw_parts(string, length) })
        .map_err(|_| permission_denied())
}

pub(crate) fn current_sid() -> Result<String, std::io::Error> {
    let mut token = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle and OpenProcessToken
    // writes a real handle to the initialized out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(permission_denied());
    }
    struct Token(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    let token = Token(token);
    let mut length = 0;
    // SAFETY: the first call asks only for the required buffer length.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length) };
    if length == 0 || length > 64 * 1024 {
        return Err(permission_denied());
    }
    // usize alignment is sufficient for TOKEN_USER and the SID follows it.
    let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: buffer is aligned, initialized and length bytes long.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(permission_denied());
    }
    // SAFETY: successful TokenUser query initialized this structure and SID.
    unsafe { sid_string((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid) }
}

struct FileSecurity {
    _allocation: LocalAllocation,
    owner: *mut c_void,
    dacl: *mut ACL,
}

fn file_security(path: &Path) -> Result<FileSecurity, std::io::Error> {
    let path = wide_path(path)?;
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: path is NUL-terminated and out-pointers are initialized. The API
    // returns all referenced pointers into descriptor's owned allocation.
    let status = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(permission_denied());
    }
    Ok(FileSecurity {
        _allocation: LocalAllocation(descriptor),
        owner,
        dacl,
    })
}

pub(crate) fn verify_private(path: &Path) -> Result<(), std::io::Error> {
    let security = file_security(path)?;
    verify_security(&security)
}

pub(crate) fn verify_opened(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::os::windows::io::AsRawHandle;
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: file retains this valid handle and all initialized out-pointers
    // point into the returned descriptor, owned until verification finishes.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(permission_denied());
    }
    verify_security(&FileSecurity {
        _allocation: LocalAllocation(descriptor),
        owner,
        dacl,
    })
}

fn verify_security(security: &FileSecurity) -> Result<(), std::io::Error> {
    let current = current_sid()?;
    // SAFETY: owner is retained inside the security descriptor allocation.
    if unsafe { sid_string(security.owner)? } != current || security.dacl.is_null() {
        return Err(permission_denied());
    }
    // A null DACL means everybody has access, not an empty ACL.
    let mut owner_allowed = false;
    // SAFETY: non-null DACL is returned by GetNamedSecurityInfoW.
    for index in 0..unsafe { (*security.dacl).AceCount } {
        let mut ace = ptr::null_mut();
        // SAFETY: index is inside AceCount; GetAce returns a pointer into the
        // descriptor-owned ACL retained for the complete loop.
        if unsafe { GetAce(security.dacl, index as u32, &mut ace) } == 0 {
            return Err(permission_denied());
        }
        // Only our simple explicit allow ACEs are accepted. Refusing a more
        // complex ACL is safer than guessing whether a token is private.
        let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
        // SAFETY: every ACE contains its header; type 0 is ACCESS_ALLOWED_ACE.
        if unsafe { (*allowed).Header.AceType } != 0 {
            return Err(permission_denied());
        }
        // SAFETY: SidStart is the variable-length SID in a valid allow ACE.
        let sid = unsafe { sid_string(ptr::addr_of_mut!((*allowed).SidStart).cast())? };
        if sid == current {
            owner_allowed = true;
        } else if sid != "S-1-5-18" {
            return Err(permission_denied());
        }
    }
    if !owner_allowed {
        return Err(permission_denied());
    }
    Ok(())
}

pub(crate) fn set_private(path: &Path) -> Result<(), std::io::Error> {
    let current = current_sid()?;
    let security = file_security(path)?;
    // Do not change another user's object even when elevated.
    if unsafe { sid_string(security.owner)? } != current {
        return Err(permission_denied());
    }
    let sddl: Vec<u16> = format!("D:P(A;OICI;GA;;;{current})(A;OICI;GA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: NUL-terminated input, initialized output, owned with LocalFree.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(permission_denied());
    }
    let _allocation = LocalAllocation(descriptor);
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = ptr::null_mut();
    // SAFETY: descriptor was successfully parsed above; all out-pointers live.
    if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        == 0
        || present == 0
    {
        return Err(permission_denied());
    }
    let encoded_path = wide_path(path)?;
    // SAFETY: path is NUL-terminated and dacl lives until this call returns.
    if unsafe {
        SetNamedSecurityInfoW(
            encoded_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    } != 0
    {
        return Err(permission_denied());
    }
    verify_private(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn explicit_owner_and_private_acl_roundtrip_on_new_nested_directory_and_file() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private").join("nested");
        super::super::ensure_private_dir(&private).unwrap();
        for path in [private.parent().unwrap(), &private] {
            let security = file_security(path).unwrap();
            assert_eq!(
                unsafe { sid_string(security.owner).unwrap() },
                current_sid().unwrap()
            );
            verify_private(path).unwrap();
            set_private(path).unwrap();
        }
        let path = private.join("credential");
        let mut file = super::super::create_private_file(&path).unwrap();
        // Verify the opened empty file before any credential bytes are written.
        verify_opened(&file).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
        let security = file_security(&path).unwrap();
        assert_eq!(
            unsafe { sid_string(security.owner).unwrap() },
            current_sid().unwrap()
        );
        file.write_all(b"disposable-test-value").unwrap();
        verify_private(&path).unwrap();
    }

    #[test]
    fn protecting_an_existing_directory_never_reassigns_its_owner() {
        let dir = tempfile::tempdir().unwrap();
        let before = file_security(dir.path()).unwrap();
        let owner = unsafe { sid_string(before.owner).unwrap() };
        let result = set_private(dir.path());
        if owner == current_sid().unwrap() {
            result.unwrap();
            verify_private(dir.path()).unwrap();
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
        let after = file_security(dir.path()).unwrap();
        assert_eq!(unsafe { sid_string(after.owner).unwrap() }, owner);
    }
}
