use crate::{
    activity::{Activity, SessionControl, HEARTBEAT},
    ipc::{self, Discovery, Reply, Request, TransportError, AUTH_TIMEOUT},
};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tagrex_service::{Principal, Service};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{oneshot, Semaphore},
    task::JoinHandle,
};

const MAX_CONNECTIONS: usize = 64;
const MAX_WORKERS: usize = 14;

/// Owns the endpoint and its exclusive profile lease. Drop stops accepting new
/// connections; already accepted service operations retain their own lifecycle.
pub struct HostHandle {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    discovery: Discovery,
    profile: PathBuf,
    activity: Arc<Activity>,
    _lease: Arc<std::fs::File>,
}

impl HostHandle {
    pub fn discovery(&self) -> &Discovery {
        &self.discovery
    }

    pub fn activity(&self) -> serde_json::Value {
        self.activity.snapshot()
    }

    pub fn control_session(&self, id: &str, action: SessionControl) -> bool {
        uuid::Uuid::parse_str(id).is_ok_and(|id| self.activity.control(id, action))
    }

    pub fn annotate_writes(&self, writes: &mut [serde_json::Value]) {
        self.activity.annotate_writes(writes);
    }

    pub async fn shutdown(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.cleanup();
    }

    fn cleanup(&self) {
        // Never remove an endpoint written by a different desktop instance.
        if ipc::read_discovery(&self.profile).is_ok_and(|d| d.instance == self.discovery.instance) {
            let _ = std::fs::remove_file(ipc::discovery_path(&self.profile));
        }
        #[cfg(unix)]
        {
            if let Ok(socket) = ipc::verify_unix_socket(&self.profile, &self.discovery) {
                let _ = std::fs::remove_file(socket);
            }
        }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.cleanup();
    }
}

/// Attach an MCP host to the existing desktop service, never opening an App.
pub async fn start_host(
    profile: &Path,
    service: Arc<Service>,
) -> Result<HostHandle, TransportError> {
    ipc::private_dir(&profile.join("mcp"))?;
    let lock_path = profile.join("mcp/host.lock");
    let lease = tagrex_service::private_storage::open_private_lock(&lock_path)?;
    match lease.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(TransportError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(error)) => return Err(TransportError::Io(error)),
    }
    let lease = Arc::new(lease);
    let instance = uuid::Uuid::new_v4().simple().to_string();
    #[cfg(unix)]
    let discovery = Discovery {
        protocol: crate::IPC_VERSION,
        transport: "unix".into(),
        address: ipc::prepare_unix_socket(profile, &instance)?
            .to_string_lossy()
            .into_owned(),
        instance,
    };
    #[cfg(windows)]
    let discovery = Discovery {
        protocol: crate::IPC_VERSION,
        transport: "windows-pipe".into(),
        address: format!(r"\\.\pipe\tagnetic-mcp-{instance}"),
        instance,
    };
    #[cfg(not(any(unix, windows)))]
    compile_error!("TAGnetic MCP requires Unix local sockets or Windows named pipes");

    #[cfg(unix)]
    let listener = {
        use std::os::unix::fs::PermissionsExt;
        let listener = tokio::net::UnixListener::bind(&discovery.address)?;
        std::fs::set_permissions(&discovery.address, std::fs::Permissions::from_mode(0o600))?;
        listener
    };
    #[cfg(windows)]
    let listener = create_windows_pipe(&discovery.address, true)?;

    let endpoint_path = ipc::discovery_path(profile);
    let staged_path = profile
        .join("mcp")
        .join(format!("{}.json", discovery.instance));
    let mut staged = ipc::private_file(&staged_path)?;
    staged.write_all(&serde_json::to_vec(&discovery).map_err(|_| TransportError::Protocol)?)?;
    staged.sync_all()?;
    if endpoint_path.exists() {
        ipc::verify_private(&endpoint_path, false)?;
        // Exclusive host lease proves the previous endpoint has no live owner.
        std::fs::remove_file(&endpoint_path)?;
    }
    std::fs::rename(&staged_path, &endpoint_path)?;
    let (shutdown, mut stopped) = oneshot::channel();
    let endpoint = discovery.clone();
    let serving_lease = lease.clone();
    let activity = Arc::new(Activity::default());
    let tracker = activity.clone();
    let task = tokio::spawn(async move {
        let admission = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let workers = Arc::new(Semaphore::new(MAX_WORKERS));
        let mut expiry = tokio::time::interval(HEARTBEAT);
        #[cfg(windows)]
        let mut listener = listener;
        loop {
            #[cfg(unix)]
            let accepted = tokio::select! {
                _ = &mut stopped => break,
                _ = expiry.tick() => {
                    if tracker.expire() { service.notify_agent_activity(); }
                    continue;
                }
                connection = listener.accept() => connection.map(|(stream, _)| stream),
            };
            #[cfg(windows)]
            let accepted = tokio::select! {
                _ = &mut stopped => break,
                _ = expiry.tick() => {
                    if tracker.expire() { service.notify_agent_activity(); }
                    continue;
                }
                result = listener.connect() => match result {
                    Err(error) => Err(error),
                    Ok(()) => match create_windows_pipe(&endpoint.address, false) {
                        Ok(next) => Ok(std::mem::replace(&mut listener, next)),
                        Err(error) => Err(error),
                    },
                },
            };
            let Ok(stream) = accepted else {
                break;
            };
            let Ok(permit) = admission.clone().try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let service = service.clone();
            let workers = workers.clone();
            let lease = serving_lease.clone();
            let instance = endpoint.instance.clone();
            let tracker = tracker.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _lease = lease;
                // Failures never log request bodies, credentials or tag data.
                let _ = serve_connection(stream, service, &instance, workers, tracker).await;
            });
        }
    });
    Ok(HostHandle {
        shutdown: Some(shutdown),
        task: Some(task),
        discovery,
        profile: profile.to_path_buf(),
        activity,
        _lease: lease,
    })
}

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin + Send>(
    mut stream: S,
    service: Arc<Service>,
    instance: &str,
    workers: Arc<Semaphore>,
    activity: Arc<Activity>,
) -> Result<(), TransportError> {
    let hello: Request = tokio::time::timeout(AUTH_TIMEOUT, ipc::read_frame(&mut stream))
        .await
        .map_err(|_| TransportError::Authentication)??;
    let principal: Principal = match hello {
        Request::Hello {
            protocol,
            instance: requested,
            credential,
        } if protocol == crate::IPC_VERSION
            && requested == instance
            && credential.len() <= ipc::MAX_CREDENTIAL_BYTES =>
        {
            match service.authenticate(&credential) {
                Ok(principal) => principal,
                Err(_) => {
                    ipc::write_frame(
                        &mut stream,
                        &Reply::err(
                            "unauthorized",
                            "Pair this client in TAGnetic before connecting.",
                        ),
                    )
                    .await?;
                    return Err(TransportError::Authentication);
                }
            }
        }
        _ => return Err(TransportError::Authentication),
    };
    ipc::write_frame(
        &mut stream,
        &Reply::ok(json!({"protocol":crate::IPC_VERSION,"session_controls":true})),
    )
    .await?;
    let request: Request = tokio::time::timeout(AUTH_TIMEOUT, ipc::read_frame(&mut stream))
        .await
        .map_err(|_| TransportError::Protocol)??;
    let reply = match request {
        Request::ListTools => {
            // The authenticated principal is checked again, even for catalogues.
            // Calling a harmless service method is unnecessary: authentication
            // cannot grant capabilities, and the static schema holds no data.
            let _ = principal;
            match serde_json::to_value(Service::tool_catalog()) {
                Ok(catalog) => Reply::ok(catalog),
                Err(_) => Reply::err("internal", "Tool catalogue could not be encoded."),
            }
        }
        Request::Presence {
            session,
            name,
            model,
            connected,
            controllable,
        } => {
            let changed = uuid::Uuid::parse_str(&session).ok().and_then(|id| {
                activity.heartbeat(
                    principal.id(),
                    id,
                    &name,
                    model.as_deref(),
                    connected,
                    controllable,
                )
            });
            match changed {
                Some(changed) => {
                    if changed {
                        service.notify_agent_activity();
                    }
                    Reply::ok(json!({}))
                }
                None => Reply::err(
                    "presence_unavailable",
                    "Agent presence could not be recorded.",
                ),
            }
        }
        call @ (Request::Call { .. } | Request::SessionCall { .. }) => {
            let (session, tool, arguments) = match call {
                Request::Call { tool, arguments } => (None, tool, arguments),
                Request::SessionCall {
                    session,
                    tool,
                    arguments,
                } => (
                    Some(uuid::Uuid::parse_str(&session).map_err(|_| TransportError::Protocol)?),
                    tool,
                    arguments,
                ),
                _ => unreachable!(),
            };
            if let Some((code, message)) =
                session.and_then(|id| activity.denied(id, principal.id(), &tool))
            {
                ipc::write_frame(&mut stream, &Reply::err(code, message)).await?;
                return Ok(());
            }
            // A saturated work budget cannot consume all Stop/status lanes.
            // Refuse new heavy work instead of queuing another blocking thread.
            let _worker = if matches!(
                tool.as_str(),
                "operations.status" | "operations.cancel" | "session.status" | "queries.cancel"
            ) {
                None
            } else {
                match workers.try_acquire_owned() {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        ipc::write_frame(&mut stream, &Reply::err("busy", "The local work queue is full. Inspect operation status or try again later.")).await?;
                        return Ok(());
                    }
                }
            };
            let mut active = activity.start(&service, principal.id(), session, &tool);
            let result =
                tokio::task::spawn_blocking(move || service.call(&principal, &tool, arguments))
                    .await;
            if let (Some(active), Ok(Ok(value))) = (&mut active, &result) {
                active.completed(value);
            }
            match result {
                Ok(Ok(value)) => Reply::ok(value),
                Ok(Err(error)) => Reply::err(&error.code, &error.message),
                Err(_) => Reply::err("internal", "Native operation worker failed; inspect operation status before retrying a write."),
            }
        }
        Request::Hello { .. } => return Err(TransportError::Protocol),
    };
    match ipc::write_frame(&mut stream, &reply).await {
        Err(TransportError::FrameTooLarge) => {
            ipc::write_frame(
                &mut stream,
                &Reply::err(
                    "response_too_large",
                    "Request a smaller projected page or binary chunk.",
                ),
            )
            .await
        }
        result => result,
    }
}

#[cfg(windows)]
fn create_windows_pipe(
    address: &str,
    first: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, std::io::Error> {
    use std::ptr;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };
    // Protected DACL: owner rights and LocalSystem only. In particular it does
    // not inherit Windows named-pipe defaults granting broad read access.
    let sid = tagrex_service::private_storage::current_user_sid().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Cannot establish named-pipe owner",
        )
    })?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: the NUL-terminated SDDL lives through the call and descriptor is
    // an initialized out-pointer. LocalFree releases the returned allocation.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: attributes and its valid security descriptor remain alive until
    // CreateNamedPipe returns. The OS copies their security data.
    let result = unsafe {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(address, &mut attributes as *mut _ as *mut _)
    };
    // SAFETY: descriptor was allocated by the conversion API above.
    unsafe { LocalFree(descriptor) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tagrex_service::Grant;
    use tokio::io::duplex;

    fn service(dir: &Path) -> (Arc<Service>, String) {
        let service = Arc::new(
            Service::new(Arc::new(std::sync::Mutex::new(None)), dir.join("service")).unwrap(),
        );
        service.set_enabled(true).unwrap();
        let pairing = service
            .pair_client("transport test".into(), Grant::default())
            .unwrap();
        (service, pairing.credential)
    }

    async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
        credential: String,
    ) -> Reply {
        ipc::write_frame(
            stream,
            &Request::Hello {
                protocol: crate::IPC_VERSION,
                instance: "test-instance".into(),
                credential,
            },
        )
        .await
        .unwrap();
        ipc::read_frame(stream).await.unwrap()
    }

    #[tokio::test]
    async fn wrong_credential_never_reaches_a_tool() {
        let directory = tempfile::tempdir().unwrap();
        let (service, _) = service(directory.path());
        let (server, mut client) = duplex(4096);
        let task = tokio::spawn(serve_connection(
            server,
            service,
            "test-instance",
            Arc::new(Semaphore::new(1)),
            Arc::new(Activity::default()),
        ));
        let reply = handshake(&mut client, "wrong-secret".into()).await;
        assert_eq!(reply.error.unwrap().code, "unauthorized");
        assert!(matches!(
            task.await.unwrap(),
            Err(TransportError::Authentication)
        ));
    }

    #[tokio::test]
    async fn saturation_preserves_control_requests_and_refuses_new_work() {
        let directory = tempfile::tempdir().unwrap();
        let (service, credential) = service(directory.path());
        for (tool, expected_error) in [("session.status", None), ("files.list", Some("busy"))] {
            let (server, mut client) = duplex(8192);
            let task = tokio::spawn(serve_connection(
                server,
                service.clone(),
                "test-instance",
                Arc::new(Semaphore::new(0)),
                Arc::new(Activity::default()),
            ));
            assert!(handshake(&mut client, credential.clone())
                .await
                .error
                .is_none());
            ipc::write_frame(
                &mut client,
                &Request::Call {
                    tool: tool.into(),
                    arguments: json!({}),
                },
            )
            .await
            .unwrap();
            let reply: Reply = ipc::read_frame(&mut client).await.unwrap();
            assert_eq!(
                reply.error.map(|error| error.code),
                expected_error.map(str::to_owned)
            );
            task.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn revoked_credential_is_refused_on_next_attachment() {
        let directory = tempfile::tempdir().unwrap();
        let (service, credential) = service(directory.path());
        let principal = service.authenticate(&credential).unwrap();
        service.revoke_client(principal.id()).unwrap();
        let (server, mut client) = duplex(8192);
        let task = tokio::spawn(serve_connection(
            server,
            service,
            "test-instance",
            Arc::new(Semaphore::new(1)),
            Arc::new(Activity::default()),
        ));
        assert_eq!(
            handshake(&mut client, credential).await.error.unwrap().code,
            "unauthorized"
        );
        assert!(matches!(
            task.await.unwrap(),
            Err(TransportError::Authentication)
        ));
    }

    #[tokio::test]
    async fn profile_lease_excludes_second_host_and_drop_removes_only_owned_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let (service, _) = service(directory.path());
        let host = start_host(directory.path(), service.clone()).await.unwrap();
        assert!(matches!(
            start_host(directory.path(), service).await,
            Err(TransportError::AlreadyRunning)
        ));
        let mut other = host.discovery().clone();
        other.instance = "ffffffffffffffffffffffffffffffff".into();
        #[cfg(unix)]
        let (other_listener, other_path) = {
            use std::os::unix::fs::PermissionsExt;
            let path = ipc::prepare_unix_socket(directory.path(), &other.instance).unwrap();
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            other.address = path.to_string_lossy().into_owned();
            (listener, path)
        };
        std::fs::write(
            ipc::discovery_path(directory.path()),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        host.shutdown().await;
        assert_eq!(
            ipc::read_discovery(directory.path()).unwrap().instance,
            "ffffffffffffffffffffffffffffffff"
        );
        #[cfg(unix)]
        {
            assert!(
                other_path.exists(),
                "another host's socket must remain untouched"
            );
            drop(other_listener);
            std::fs::remove_file(other_path).unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_discovery_socket_and_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let (service, _) = service(directory.path());
        let host = start_host(directory.path(), service).await.unwrap();
        for (path, expected) in [
            (directory.path().join("mcp"), 0o700),
            (ipc::unix_socket_directory(), 0o700),
            (ipc::discovery_path(directory.path()), 0o600),
            (PathBuf::from(&host.discovery().address), 0o600),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                expected
            );
        }
        host.shutdown().await;
        assert!(!ipc::discovery_path(directory.path()).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forged_wrong_profile_and_nonprivate_socket_discovery_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let (service, _) = service(directory.path());
        let host = start_host(directory.path(), service).await.unwrap();
        let original = host.discovery().clone();
        let wrong_profile = directory.path().join("another-profile");
        std::fs::create_dir(&wrong_profile).unwrap();
        let mut forged = original.clone();
        forged.address = ipc::unix_socket_path(&wrong_profile, &original.instance)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for address in [
            forged.address,
            "/tmp/an-arbitrary-local-endpoint.sock".into(),
        ] {
            let mut changed = original.clone();
            changed.address = address;
            std::fs::write(
                ipc::discovery_path(directory.path()),
                serde_json::to_vec(&changed).unwrap(),
            )
            .unwrap();
            assert!(matches!(
                ipc::read_discovery(directory.path()),
                Err(TransportError::Permissions)
            ));
        }
        let mut changed = original.clone();
        changed.instance = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into();
        std::fs::write(
            ipc::discovery_path(directory.path()),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            ipc::read_discovery(directory.path()),
            Err(TransportError::Permissions)
        ));
        std::fs::write(
            ipc::discovery_path(directory.path()),
            serde_json::to_vec(&original).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&original.address, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(matches!(
            ipc::read_discovery(directory.path()),
            Err(TransportError::Permissions)
        ));
        std::fs::set_permissions(&original.address, std::fs::Permissions::from_mode(0o600))
            .unwrap();
        assert!(ipc::read_discovery(directory.path()).is_ok());
        host.shutdown().await;
        assert!(!Path::new(&original.address).exists());
    }
}
