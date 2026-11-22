use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_CREDENTIAL_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("TAGnetic MCP is not running or its local connection is unavailable")]
    Unavailable,
    #[error("TAGnetic MCP desktop is already running for this profile")]
    AlreadyRunning,
    #[error("TAGnetic MCP authentication was refused; pair this client in the desktop")]
    Authentication,
    #[error("Invalid MCP connector arguments: {0}")]
    Arguments(String),
    #[error("The MCP file or directory is not private to the current user")]
    Permissions,
    #[error("The local MCP frame exceeds its size limit")]
    FrameTooLarge,
    #[error("The local MCP protocol is incompatible or malformed")]
    Protocol,
    #[error("Local MCP I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP server stopped: {0}")]
    Server(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub protocol: u32,
    pub transport: String,
    pub address: String,
    pub instance: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Hello {
        protocol: u32,
        instance: String,
        credential: String,
    },
    ListTools,
    Presence {
        session: String,
        name: String,
        model: Option<String>,
        connected: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        controllable: bool,
    },
    Call {
        tool: String,
        arguments: Value,
    },
    SessionCall {
        session: String,
        tool: String,
        arguments: Value,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub result: Option<Value>,
    pub error: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WireError {
    pub code: String,
    pub message: String,
}

impl Reply {
    pub fn ok(result: Value) -> Self {
        Self {
            result: Some(result),
            error: None,
        }
    }
    pub fn err(code: &str, message: &str) -> Self {
        Self {
            result: None,
            error: Some(WireError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

pub(crate) async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), TransportError> {
    let encoded = serde_json::to_vec(value).map_err(|_| TransportError::Protocol)?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge);
    }
    writer.write_u32(encoded.len() as u32).await?;
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

pub(crate) async fn read_frame<R: AsyncRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<T, TransportError> {
    let length = reader.read_u32().await? as usize;
    if length == 0 {
        return Err(TransportError::Protocol);
    }
    if length > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge);
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|_| TransportError::Protocol)
}

pub(crate) fn private_dir(path: &Path) -> Result<(), TransportError> {
    tagrex_service::private_storage::ensure_private_dir(path).map_err(storage_error)
}

pub(crate) fn verify_private(path: &Path, directory: bool) -> Result<(), TransportError> {
    tagrex_service::private_storage::verify_private(path, directory).map_err(storage_error)
}

pub(crate) fn private_file(path: &Path) -> Result<std::fs::File, TransportError> {
    tagrex_service::private_storage::create_private_file(path).map_err(storage_error)
}

fn storage_error(error: std::io::Error) -> TransportError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        TransportError::Permissions
    } else {
        TransportError::Io(error)
    }
}

pub(crate) fn read_credential(path: &Path) -> Result<String, TransportError> {
    use std::io::Read;
    let file = tagrex_service::private_storage::open_private_read(path).map_err(storage_error)?;
    let mut bytes = Vec::new();
    file.take((MAX_CREDENTIAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CREDENTIAL_BYTES {
        return Err(TransportError::Authentication);
    }
    let token = String::from_utf8(bytes).map_err(|_| TransportError::Authentication)?;
    let token = token.trim();
    if token.is_empty() || token.contains(['\n', '\r', '\0']) {
        return Err(TransportError::Authentication);
    }
    Ok(token.to_owned())
}

pub(crate) fn discovery_path(profile: &Path) -> PathBuf {
    profile.join("mcp").join("endpoint.json")
}

/// Unix socket addresses have a much shorter limit than filesystem paths
/// (104 bytes on macOS, 108 on Linux). Keep only the private discovery file in
/// the profile; bind the socket in a short, deterministic per-user directory.
#[cfg(unix)]
pub(crate) fn unix_socket_directory() -> PathBuf {
    // SAFETY: geteuid has no arguments or memory access requirements.
    Path::new("/tmp").join(format!("tagnetic-mcp-{}", unsafe { libc::geteuid() }))
}

#[cfg(unix)]
pub(crate) fn unix_socket_path(profile: &Path, instance: &str) -> Result<PathBuf, TransportError> {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    if instance.len() != 32 || !instance.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TransportError::Protocol);
    }
    let canonical_profile = std::fs::canonicalize(profile)?;
    let digest = Sha256::digest(canonical_profile.as_os_str().as_bytes());
    let profile_id: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(unix_socket_directory().join(format!("{profile_id}-{instance}.sock")))
}

#[cfg(unix)]
pub(crate) fn prepare_unix_socket(
    profile: &Path,
    instance: &str,
) -> Result<PathBuf, TransportError> {
    let directory = unix_socket_directory();
    // Do not adopt a pre-existing symlink, foreign owner or broadly-readable
    // directory in /tmp. A predictable name is not authority.
    match std::fs::symlink_metadata(&directory) {
        Ok(_) => verify_private(&directory, true)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => private_dir(&directory)?,
        Err(error) => return Err(error.into()),
    }
    unix_socket_path(profile, instance)
}

#[cfg(unix)]
pub(crate) fn verify_unix_socket(
    profile: &Path,
    discovery: &Discovery,
) -> Result<PathBuf, TransportError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let expected = unix_socket_path(profile, &discovery.instance)?;
    if discovery.transport != "unix" || Path::new(&discovery.address) != expected {
        return Err(TransportError::Permissions);
    }
    verify_private(&unix_socket_directory(), true)?;
    let metadata = std::fs::symlink_metadata(&expected)?;
    // SAFETY: geteuid has no arguments or memory access requirements.
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(TransportError::Permissions);
    }
    Ok(expected)
}

pub(crate) fn read_discovery(profile: &Path) -> Result<Discovery, TransportError> {
    verify_private(&profile.join("mcp"), true)?;
    let path = discovery_path(profile);
    verify_private(&path, false)?;
    let bytes =
        tagrex_service::private_storage::read_bounded(&path, 8192).map_err(storage_error)?;
    let discovery: Discovery =
        serde_json::from_slice(&bytes).map_err(|_| TransportError::Protocol)?;
    if discovery.protocol != crate::IPC_VERSION
        || discovery.instance.is_empty()
        || discovery.instance.len() > 64
    {
        return Err(TransportError::Protocol);
    }
    #[cfg(unix)]
    {
        verify_unix_socket(profile, &discovery)?;
    }
    #[cfg(windows)]
    {
        if discovery.transport != "windows-pipe"
            || !discovery.address.starts_with(r"\\.\pipe\tagnetic-mcp-")
        {
            return Err(TransportError::Permissions);
        }
    }
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn framed_roundtrip_and_eof() {
        let (mut sender, mut reader) = duplex(1024);
        write_frame(&mut sender, &serde_json::json!({"hello":"world"}))
            .await
            .unwrap();
        let value: Value = read_frame(&mut reader).await.unwrap();
        assert_eq!(value["hello"], "world");
        drop(sender);
        assert!(matches!(
            read_frame::<_, Value>(&mut reader).await,
            Err(TransportError::Io(_))
        ));
    }

    #[tokio::test]
    async fn refuses_length_before_allocating_body() {
        let (mut sender, mut reader) = duplex(32);
        sender
            .write_u32((MAX_FRAME_BYTES + 1) as u32)
            .await
            .unwrap();
        assert!(matches!(
            read_frame::<_, Value>(&mut reader).await,
            Err(TransportError::FrameTooLarge)
        ));
    }

    #[tokio::test]
    async fn refuses_empty_and_malformed_frames() {
        for body in [vec![], b"not JSON".to_vec()] {
            let (mut sender, mut reader) = duplex(32);
            sender.write_u32(body.len() as u32).await.unwrap();
            sender.write_all(&body).await.unwrap();
            assert!(matches!(
                read_frame::<_, Value>(&mut reader).await,
                Err(TransportError::Protocol)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn credential_must_be_private_regular_file() {
        use std::{
            io::Write,
            os::unix::fs::{symlink, PermissionsExt},
        };
        let dir = tempfile::tempdir().unwrap();
        private_dir(dir.path()).unwrap();
        let path = dir.path().join("credential");
        private_file(&path)
            .unwrap()
            .write_all(b"test-secret\n")
            .unwrap();
        assert_eq!(read_credential(&path).unwrap(), "test-secret");
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(matches!(
            read_credential(&link),
            Err(TransportError::Permissions)
        ));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            read_credential(&path),
            Err(TransportError::Permissions)
        ));
    }
}
