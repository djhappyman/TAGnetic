//! Production MCP transport. The connector has no library, journal or writer.
//!
//! Standard MCP is served by the official SDK on stdin/stdout. Each request
//! attaches to the desktop's existing service over private, authenticated OS
//! IPC. Opening independent IPC calls lets Stop/status bypass a long-running
//! request on the same MCP connection.

mod bridge;
mod host;
mod ipc;

pub use bridge::{handle_cli, run_stdio, StdioOptions};
pub use host::{start_host, HostHandle};
pub use ipc::{Discovery, TransportError, MAX_FRAME_BYTES};

/// Write a newly paired client's secret to a private, newly-created file.
pub fn write_credential_file(
    path: &std::path::Path,
    credential: &str,
) -> Result<(), TransportError> {
    use std::io::Write;
    if credential.is_empty()
        || credential.len() > ipc::MAX_CREDENTIAL_BYTES
        || credential.contains(['\n', '\r', '\0'])
    {
        return Err(TransportError::Authentication);
    }
    let mut file = ipc::private_file(path)?;
    file.write_all(credential.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Version of the private connector/desktop contract, separate from MCP.
pub const IPC_VERSION: u32 = 1;
