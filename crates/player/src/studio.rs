//! Split Studio borrows output, never the library sink or its decoder queue.
mod stream;

use super::{router, Output};
use std::{
    fs::File,
    sync::{Arc, Mutex},
    time::Duration,
};
use stream::{DecodeRequest, Voice, Worker};
use tagrex_audio_workbench::{PreparedTreatment, SeekIndex};

#[derive(Clone, Copy, Debug)]
pub struct StudioAudition {
    pub start: Duration,
    pub end: Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StudioPreviewState {
    #[default]
    Idle,
    Loading,
    Ready,
    Unavailable,
    Refused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StudioStatus {
    pub position: Duration,
    pub playing: bool,
    pub state: StudioPreviewState,
    pub sequence: u64,
    pub retune_failed: Option<u64>,
}

pub(super) enum Command {
    Play {
        owner: String,
        sequence: u64,
        file: File,
        start: Duration,
        end: Duration,
        audition: Option<StudioAudition>,
        sample_rate: u32,
        seek_index: Option<Arc<SeekIndex>>,
        treatment: PreparedTreatment,
    },
    Seek {
        owner: String,
        sequence: u64,
        position: Duration,
    },
    Pause {
        owner: String,
        sequence: u64,
    },
    Retune {
        owner: String,
        sequence: u64,
        revision: u64,
        file: File,
        treatment: PreparedTreatment,
    },
    Close {
        owner: String,
    },
}

impl Command {
    pub(super) fn owner(&self) -> &str {
        match self {
            Self::Play { owner, .. }
            | Self::Seek { owner, .. }
            | Self::Pause { owner, .. }
            | Self::Retune { owner, .. }
            | Self::Close { owner } => owner,
        }
    }

    fn sequence(&self) -> Option<u64> {
        match self {
            Self::Play { sequence, .. }
            | Self::Seek { sequence, .. }
            | Self::Pause { sequence, .. } => Some(*sequence),
            Self::Close { .. } | Self::Retune { .. } => None,
        }
    }

    pub(super) fn requests_play(&self) -> bool {
        matches!(self, Self::Play { .. })
    }

    pub(super) fn valid(&self) -> bool {
        const MAX_POSITION: Duration = Duration::from_secs(u32::MAX as u64);
        if self.owner().is_empty() || self.owner().len() > 128 {
            return false;
        }
        match self {
            Self::Play {
                start,
                end,
                audition,
                sample_rate,
                ..
            } => {
                (1..=655_350).contains(sample_rate)
                    && start < end
                    && *end <= MAX_POSITION
                    && audition.is_none_or(|a| a.start <= *start && *start < a.end && a.end <= *end)
            }
            Self::Seek { position, .. } => *position <= MAX_POSITION,
            Self::Pause { .. } | Self::Retune { .. } | Self::Close { .. } => true,
        }
    }
}

#[derive(Default, Clone)]
pub(super) struct Shared(Arc<Mutex<Snapshot>>);

#[derive(Default)]
struct Snapshot {
    owner: Option<Arc<str>>,
    owns_output: bool,
    value: StudioStatus,
}

impl Shared {
    pub(super) fn for_owner(&self, owner: &str) -> StudioStatus {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.owner.as_deref() == Some(owner) {
            state.value
        } else {
            StudioStatus {
                state: if state.owns_output {
                    StudioPreviewState::Refused
                } else {
                    StudioPreviewState::Idle
                },
                ..StudioStatus::default()
            }
        }
    }
}

pub(super) struct Actor {
    shared: Shared,
    owner: Option<Arc<str>>,
    state: StudioStatus,
    lease: Option<Lease>,
    worker: Option<Worker>,
    alternate: Option<Worker>,
}

impl Actor {
    pub(super) fn new(shared: Shared) -> Self {
        Self {
            shared,
            owner: None,
            state: StudioStatus::default(),
            lease: None,
            worker: None,
            alternate: None,
        }
    }

    pub(super) fn accepts(&self, command: &Command) -> bool {
        let same_owner = self.owner.as_deref() == Some(command.owner());
        if let Command::Retune { sequence, .. } = command {
            return same_owner && *sequence == self.state.sequence && self.lease.is_some();
        }
        if same_owner {
            return command
                .sequence()
                .is_none_or(|sequence| sequence == 0 || sequence >= self.state.sequence);
        }
        self.lease.is_none() && !matches!(command, Command::Pause { .. })
    }

    pub(super) fn owns_output(&self) -> bool {
        self.lease.is_some()
    }

    pub(super) fn needs_tick(&self) -> bool {
        self.lease.as_ref().is_some_and(|lease| {
            lease.pending.is_some()
                || lease.playing
                || self.state.state == StudioPreviewState::Loading
        })
    }

    pub(super) fn set_volume(&self, volume: f32) {
        if let Some(lease) = self.lease.as_ref() {
            lease.voice.set_volume(volume);
            if let Some(pending) = &lease.pending {
                pending.voice.set_volume(volume);
            }
        }
    }

    pub(super) fn revoke(&mut self) -> bool {
        if let Some(lease) = self.lease.take() {
            self.state.position = lease.position();
            self.state.playing = false;
            self.state.state = StudioPreviewState::Ready;
            self.publish();
            true
        } else {
            false
        }
    }

    pub(super) fn handle<O: Output>(&mut self, command: Command, output: Option<&O>, volume: f32) {
        if !self.accepts(&command) {
            return;
        }
        let sequence = command.sequence();
        match command {
            Command::Play {
                owner,
                file,
                start,
                end,
                audition,
                sample_rate,
                seek_index,
                treatment,
                ..
            } => {
                self.lease = None;
                self.owner = Some(owner.into());
                self.state = StudioStatus {
                    position: start,
                    playing: false,
                    state: StudioPreviewState::Loading,
                    sequence: 0,
                    retune_failed: None,
                };
                if let Some(router) = output.and_then(Output::studio_router) {
                    if self.worker.is_none() {
                        self.worker = Worker::new().ok();
                    }
                    if let Some(worker) = self.worker.as_ref() {
                        let geometry = Geometry {
                            sample_rate,
                            end,
                            audition,
                            treatment,
                        };
                        match Lease::open(worker, router, volume, file, geometry, start, seek_index)
                        {
                            Ok(lease) => self.lease = Some(lease),
                            Err(err) => {
                                eprintln!("audio: Studio could not start preview: {err}");
                                self.state.state = StudioPreviewState::Refused;
                            }
                        }
                    } else {
                        self.state.state = StudioPreviewState::Unavailable;
                    }
                } else {
                    self.state.state = StudioPreviewState::Unavailable;
                }
            }
            Command::Seek {
                owner, position, ..
            } => {
                if let Some(lease) = self.lease.as_mut() {
                    if position >= lease.geometry.end && lease.geometry.audition.is_none() {
                        let position = lease.geometry.end;
                        self.lease = None;
                        self.state = StudioStatus {
                            position,
                            playing: false,
                            state: StudioPreviewState::Ready,
                            sequence: 0,
                            retune_failed: None,
                        };
                    } else {
                        let worker = self
                            .worker
                            .as_ref()
                            .expect("a worker for every preview lease");
                        match lease.seek(worker, volume, position) {
                            Ok(()) => {
                                self.state.position = position;
                                self.state.state = StudioPreviewState::Loading;
                            }
                            Err(error) => {
                                eprintln!("audio: Studio could not seek: {error}");
                                self.lease = None;
                                self.state.state = StudioPreviewState::Refused;
                            }
                        }
                        self.state.playing = false;
                    }
                } else {
                    self.owner = Some(owner.into());
                    self.state = StudioStatus {
                        position,
                        playing: false,
                        state: StudioPreviewState::Idle,
                        sequence: 0,
                        retune_failed: None,
                    };
                }
            }
            Command::Pause { owner, .. } => {
                if self.owner.as_deref() == Some(&owner) {
                    if let Some(lease) = self.lease.as_mut() {
                        lease.playing = false;
                        lease.voice.pause(true);
                        self.state.playing = false;
                    }
                }
            }
            Command::Retune {
                revision,
                file,
                treatment,
                ..
            } => {
                let lease = self.lease.as_mut().expect("accepted preview owns a lease");
                if revision <= lease.revision {
                    return;
                }
                lease.revision = revision;
                self.state.retune_failed = None;
                if let Some(pending) = &lease.pending {
                    if same_treatment(pending.treatment, treatment) {
                        return;
                    }
                }
                lease.pending = None;
                if same_treatment(lease.geometry.treatment, treatment) {
                    return;
                }
                if self.alternate.is_none() {
                    self.alternate = Worker::new().ok();
                }
                let result = self
                    .alternate
                    .as_ref()
                    .ok_or("preview worker unavailable".to_owned())
                    .and_then(|worker| {
                        lease.retune(worker, volume, lease.retune_start(), file, treatment)
                    });
                if let Err(error) = result {
                    eprintln!("audio: Studio could not change preview treatment: {error}");
                    self.state.retune_failed = Some(revision);
                }
            }
            Command::Close { owner } => {
                if self.owner.as_deref() == Some(&owner) {
                    self.lease = None;
                    self.owner = None;
                    self.state = StudioStatus::default();
                }
            }
        }
        if let Some(sequence) = sequence {
            self.state.sequence = sequence;
            self.state.retune_failed = None;
        }
        self.publish();
    }

    pub(super) fn tick(&mut self) {
        let Some(lease) = self.lease.as_mut() else {
            return;
        };
        if let Some(worker) = &self.alternate {
            match lease.advance_retune(worker) {
                Ok(false) => {}
                Ok(true) => {
                    std::mem::swap(&mut self.worker, &mut self.alternate);
                }
                Err(error) => {
                    eprintln!("audio: Studio kept the previous preview: {error}");
                    self.state.retune_failed = Some(lease.revision);
                }
            }
        }
        let ready = lease.voice.ready();
        let ended = lease.voice.ended();
        let refused = lease.voice.failed();
        if !ready && !ended && !refused {
            return;
        }
        self.state.position = lease.position();
        if refused || ended {
            self.state.playing = false;
            self.state.state = if refused {
                StudioPreviewState::Refused
            } else {
                StudioPreviewState::Ready
            };
            self.lease = None;
        } else {
            self.state.playing = lease.playing;
            self.state.state = StudioPreviewState::Ready;
            lease.voice.pause(!lease.playing);
        }
        self.publish();
    }

    fn publish(&mut self) {
        let owns_output = self.owns_output();
        let mut state = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.value != self.state
            || state.owns_output != owns_output
            || state.owner != self.owner
        {
            state.owner.clone_from(&self.owner);
            state.value = self.state;
            state.owns_output = owns_output;
        }
    }
}

struct Lease {
    voice: Voice,
    file: File,
    router: router::Handle,
    geometry: Geometry,
    playing: bool,
    seek_index: Option<Arc<SeekIndex>>,
    pending: Option<Retune>,
    revision: u64,
}

struct Retune {
    voice: Voice,
    file: File,
    treatment: PreparedTreatment,
    retries: u8,
}

fn same_treatment(a: PreparedTreatment, b: PreparedTreatment) -> bool {
    a.settings() == b.settings() && a.gain_db() == b.gain_db()
}

#[derive(Clone, Copy)]
struct Geometry {
    sample_rate: u32,
    end: Duration,
    audition: Option<StudioAudition>,
    treatment: PreparedTreatment,
}

impl Lease {
    fn open(
        worker: &Worker,
        router: router::Handle,
        volume: f32,
        file: File,
        geometry: Geometry,
        start: Duration,
        seek_index: Option<Arc<SeekIndex>>,
    ) -> Result<Self, String> {
        let request = geometry.request(&file, start, seek_index.clone())?;
        let voice = Voice::new(&router, volume, worker, request);
        Ok(Self {
            voice,
            file,
            router,
            geometry,
            playing: true,
            seek_index,
            pending: None,
            revision: 0,
        })
    }

    fn seek(&mut self, worker: &Worker, volume: f32, position: Duration) -> Result<(), String> {
        if let Some(pending) = self.pending.take() {
            self.geometry.treatment = pending.treatment;
        }
        let request = self
            .geometry
            .request(&self.file, position, self.seek_index.clone())?;
        self.voice.pause(true);
        self.voice = Voice::new(&self.router, volume, worker, request);
        Ok(())
    }

    fn retune(
        &mut self,
        worker: &Worker,
        volume: f32,
        position: Duration,
        file: File,
        treatment: PreparedTreatment,
    ) -> Result<(), String> {
        let geometry = Geometry {
            treatment,
            ..self.geometry
        };
        let request = geometry.request(&file, position, self.seek_index.clone())?;
        self.pending = Some(Retune {
            voice: Voice::prepare(&self.router, volume, worker, request),
            file,
            treatment,
            retries: 0,
        });
        Ok(())
    }

    fn advance_retune(&mut self, worker: &Worker) -> Result<bool, String> {
        let Some(pending) = &self.pending else {
            return Ok(false);
        };
        if pending.voice.failed() {
            self.pending = None;
            return Err("The updated audio could not be decoded".into());
        }
        if !pending.voice.ready() {
            return Ok(false);
        }
        let position = self.play_position();
        let start = pending.voice.position();
        // The new voice's position is rounded to its source frame. The
        // outgoing device clock may sit just before that frame without
        // having wrapped to an earlier pass of the loop.
        let wrapped = start.saturating_sub(position) > frame_time(1, self.geometry.sample_rate);
        if !wrapped && pending.voice.covers(position) {
            let mut pending = self.pending.take().expect("a prepared preview");
            pending.voice.pause(!self.playing);
            pending.voice.activate(&self.router, position);
            self.geometry.treatment = pending.treatment;
            self.file = pending.file;
            self.voice = pending.voice;
            return Ok(true);
        }
        let wait_for_loop = self.geometry.audition.is_some_and(|range| {
            let rate = self.geometry.sample_rate;
            frame(start, rate) == frame(range.start, rate)
                && range.end.saturating_sub(position) <= Duration::from_millis(40)
        });
        if !wrapped && (!pending.voice.full() || wait_for_loop) {
            return Ok(false);
        }
        let Retune {
            voice,
            file,
            treatment,
            retries,
        } = self.pending.take().expect("a prepared preview");
        drop(voice);
        if retries >= 2 {
            return Err(format!(
                "The updated preview could not catch the playhead ({start:?} to {position:?})"
            ));
        }
        let start = self
            .geometry
            .audition
            .filter(|_| wrapped)
            .map_or(position, |range| range.start);
        self.retune(worker, self.voice.volume(), start, file, treatment)?;
        self.pending.as_mut().expect("an updated preview").retries = retries + 1;
        Ok(false)
    }

    fn play_position(&self) -> Duration {
        let position = self.position();
        self.geometry
            .audition
            .filter(|range| position >= range.end)
            .map_or(position, |range| range.start)
    }

    fn retune_start(&self) -> Duration {
        let position = self.play_position();
        self.geometry
            .audition
            .filter(|range| range.end.saturating_sub(position) <= Duration::from_millis(40))
            .map_or(position, |range| range.start)
    }

    fn position(&self) -> Duration {
        self.voice.position().min(self.geometry.end)
    }
}

impl Geometry {
    fn request(
        &self,
        file: &File,
        position: Duration,
        seek_index: Option<Arc<SeekIndex>>,
    ) -> Result<DecodeRequest, String> {
        let mut start = frame(position, self.sample_rate);
        let end = frame(self.audition.map_or(self.end, |a| a.end), self.sample_rate);
        let loop_start = self.audition.map(|a| frame(a.start, self.sample_rate));
        if let Some(loop_start) = loop_start {
            if start < loop_start || start >= end {
                start = loop_start;
            }
        }
        if start >= end {
            return Err("The preview range does not contain an audio frame".into());
        }
        let file = file.try_clone().map_err(|error| error.to_string())?;
        Ok(DecodeRequest {
            file,
            source_rate: self.sample_rate,
            start,
            end,
            loop_start,
            seek_index,
            treatment: self.treatment,
        })
    }
}

fn frame(time: Duration, rate: u32) -> u64 {
    ((time.as_nanos() * u128::from(rate) + 500_000_000) / 1_000_000_000) as u64
}

fn frame_time(frame: u64, rate: u32) -> Duration {
    let rate = u64::from(rate);
    Duration::new(frame / rate, (frame % rate * 1_000_000_000 / rate) as u32)
}

#[cfg(test)]
mod tests;
