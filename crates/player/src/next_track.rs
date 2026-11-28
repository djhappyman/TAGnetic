//! Prepare the explicitly queued track without blocking transport controls.
use super::{prepare_track, PreparedTrack};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Weak,
    },
};

type Prepare = Box<dyn Fn(PathBuf) -> Option<PreparedTrack> + Send>;

pub(super) struct Worker {
    prepare: Option<Prepare>,
    requests: Option<SyncSender<Request>>,
}

struct Request {
    path: PathBuf,
    reply: SyncSender<Option<PreparedTrack>>,
    owner: Weak<()>,
}

pub(super) struct Pending {
    reply: Receiver<Option<PreparedTrack>>,
    _owner: Arc<()>,
}

impl Worker {
    pub(super) fn new() -> Self {
        Self::with(prepare_track)
    }

    pub(super) fn with(
        prepare: impl Fn(PathBuf) -> Option<PreparedTrack> + Send + 'static,
    ) -> Self {
        Self {
            prepare: Some(Box::new(prepare)),
            requests: None,
        }
    }

    pub(super) fn start(&mut self, path: PathBuf) -> Option<Pending> {
        if let Some(prepare) = self.prepare.take() {
            // A single queued slot lets a new playback session request its
            // next track while an obsolete file open finishes.
            let (requests, jobs) = mpsc::sync_channel::<Request>(1);
            std::thread::Builder::new()
                .name("tagnetic-next-track".into())
                .spawn(move || {
                    while let Ok(request) = jobs.recv() {
                        if request.owner.strong_count() != 0 {
                            let _ = request.reply.send(prepare(request.path));
                        }
                    }
                })
                .map_err(|error| eprintln!("audio: next-track preparation unavailable: {error}"))
                .ok()?;
            self.requests = Some(requests);
        }
        let (send, reply) = mpsc::sync_channel(1);
        let owner = Arc::new(());
        self.requests
            .as_ref()?
            .try_send(Request {
                path,
                reply: send,
                owner: Arc::downgrade(&owner),
            })
            .ok()?;
        Some(Pending {
            reply,
            _owner: owner,
        })
    }
}

impl Pending {
    pub(super) fn poll(&self) -> std::task::Poll<Option<PreparedTrack>> {
        match self.reply.try_recv() {
            Ok(result) => std::task::Poll::Ready(result),
            Err(TryRecvError::Empty) => std::task::Poll::Pending,
            Err(TryRecvError::Disconnected) => std::task::Poll::Ready(None),
        }
    }
}
