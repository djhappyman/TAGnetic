//! Windows counterparts of Unix owner-only modes. File permissions and pipe
//! permissions are separate: protecting a pipe does not protect its token file.
fn permission_denied() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Service state must be private to the current user",
    )
}
use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, GetSecurityInfo, SetNamedSecurityInfoW, SE_FILE_OBJECT,
        },
        GetAce, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACL,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        TOKEN_QUERY, TOKEN_USER,
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
    #[test]
    fn owner_only_acl_roundtrips_on_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        set_private(dir.path()).unwrap();
        verify_private(dir.path()).unwrap();
        let file = dir.path().join("credential");
        std::fs::write(&file, b"disposable-test-value").unwrap();
        set_private(&file).unwrap();
        verify_private(&file).unwrap();
    }
}
