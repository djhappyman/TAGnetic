//! Split Studio borrows output, never the library sink or its decoder queue.
mod retune;
mod stream;

use super::{router, Output};
use std::{
    fs::File,
    sync::{Arc, Mutex},
    time::Duration,
};
use stream::{DecodeRequest, Handoff, Voice, Worker};
use tagrex_audio_workbench::{
    AudioTreatment, PlaybackHistory, PlaybackProbeWindow, PreparedTreatment, SeekIndex,
};

#[derive(Clone, Copy, Debug)]
pub struct StudioAudition {
    pub start: Duration,
    pub end: Duration,
    pub smooth: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StudioPreviewRange {
    pub start: Duration,
    pub end: Duration,
    pub repeat: bool,
    pub smooth: bool,
}

impl StudioPreviewRange {
    fn position(self, position: Duration) -> Duration {
        if (self.start..self.end).contains(&position) {
            position
        } else {
            self.start
        }
    }
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
    pub retune_pending: Option<u64>,
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
        treatment: Box<PreparedTreatment>,
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
        treatment: Box<PreparedTreatment>,
        range: Option<StudioPreviewRange>,
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
            Self::Retune { range, .. } => {
                range.is_none_or(|range| range.start < range.end && range.end <= MAX_POSITION)
            }
            Self::Pause { .. } | Self::Close { .. } => true,
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
    analysis: Option<Arc<Mutex<PlaybackHistory>>>,
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

    pub(super) fn playback_window(
        &self,
        owner: &str,
        frame: u64,
        settings: AudioTreatment,
    ) -> (bool, Option<PlaybackProbeWindow>) {
        let history = {
            let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if state.owner.as_deref() != Some(owner) || !state.value.playing {
                return (false, None);
            }
            state.analysis.clone()
        };
        let Some(history) = history else {
            return (false, None);
        };
        let mut history = history.lock().unwrap_or_else(|p| p.into_inner());
        (history.producing(), history.take(frame, settings))
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
                || lease.queued.is_some()
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
                    retune_pending: None,
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
                            treatment: *treatment,
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
                            retune_pending: None,
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
                        retune_pending: None,
                    };
                }
            }
            Command::Pause { owner, .. } => {
                if self.owner.as_deref() == Some(&owner) {
                    if let Some(lease) = self.lease.as_mut() {
                        lease.playing = false;
                        lease.voice.pause(true);
                        if let Some(pending) = &lease.pending {
                            pending.voice.pause(true);
                        }
                        self.state.playing = false;
                    }
                }
            }
            Command::Retune {
                revision,
                file,
                treatment,
                range,
                ..
            } => {
                let lease = self.lease.as_mut().expect("accepted preview owns a lease");
                if revision <= lease.revision {
                    return;
                }
                lease.revision = revision;
                self.state.retune_failed = None;
                lease.queue_retune(RetuneTarget {
                    file,
                    treatment: *treatment,
                    range,
                });
                if let Err(error) = lease.start_retune(&mut self.alternate) {
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
                    if lease.queued.is_none() {
                        self.state.retune_failed = Some(lease.revision);
                    }
                }
            }
        }
        if let Err(error) = lease.start_retune(&mut self.alternate) {
            eprintln!("audio: Studio could not change preview treatment: {error}");
            self.state.retune_failed = Some(lease.revision);
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
        let retune_pending = self.lease.as_ref().and_then(|lease| {
            (lease.playing && (lease.pending.is_some() || lease.queued.is_some()))
                .then_some(lease.revision)
        });
        let current = StudioStatus {
            retune_pending,
            ..self.state
        };
        let mut state = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
        state.analysis = self
            .lease
            .as_ref()
            .map(|lease| lease.voice.playback_history());
        if state.value != current || state.owns_output != owns_output || state.owner != self.owner {
            state.owner.clone_from(&self.owner);
            state.value = current;
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
    queued: Option<RetuneTarget>,
    revision: u64,
}

struct Retune {
    voice: Voice,
    target: RetuneTarget,
    retries: u8,
    submitted: bool,
}

struct RetuneTarget {
    file: File,
    treatment: PreparedTreatment,
    range: Option<StudioPreviewRange>,
}

impl RetuneTarget {
    fn matches(&self, other: &Self) -> bool {
        self.range == other.range && same_treatment(&self.treatment, &other.treatment)
    }
}

fn same_treatment(a: &PreparedTreatment, b: &PreparedTreatment) -> bool {
    a.same_preview(b)
}

#[derive(Clone)]
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
            queued: None,
            revision: 0,
        })
    }

    fn seek(&mut self, worker: &Worker, volume: f32, position: Duration) -> Result<(), String> {
        let pending = self.pending.take();
        if let Some(target) = self.queued.take().or_else(|| pending.map(|job| job.target)) {
            self.geometry = self.geometry.retuned(&target);
        }
        let request = self
            .geometry
            .request(&self.file, position, self.seek_index.clone())?;
        self.voice.pause(true);
        self.voice = Voice::new(&self.router, volume, worker, request);
        Ok(())
    }

    fn queue_retune(&mut self, target: RetuneTarget) {
        self.queued = None;
        if let Some(pending) = self.pending.as_ref().filter(|pending| pending.submitted) {
            // A device handoff can already have taken effect. Keep the old
            // and new voices alive until the device acknowledges it.
            if !pending.target.matches(&target) {
                self.queued = Some(target);
            }
        } else if target.range.is_none()
            && same_treatment(&self.geometry.treatment, &target.treatment)
        {
            self.pending = None;
        } else if !self
            .pending
            .as_ref()
            .is_some_and(|job| job.target.matches(&target))
        {
            // Let the prepared voice reach the playhead even during a drag.
            // Only the latest edit waits behind it.
            self.queued = Some(target);
        }
    }

    fn start_retune(&mut self, worker: &mut Option<Worker>) -> Result<(), String> {
        if self.pending.is_some() {
            return Ok(());
        }
        let Some(target) = self.queued.take() else {
            return Ok(());
        };
        if target.range.is_none()
            && self
                .geometry
                .treatment
                .can_retune_eq(&target.treatment, self.geometry.sample_rate)
            && self.voice.retune_eq(target.treatment.clone())
        {
            self.file = target.file;
            self.geometry.treatment = target.treatment;
            return Ok(());
        }
        if worker.is_none() {
            *worker = Worker::new().ok();
        }
        let worker = worker.as_ref().ok_or("preview worker unavailable")?;
        let position = if target.range.is_some() {
            self.play_position()
        } else {
            self.retune_start()
        };
        self.retune(worker, position, target)
    }

    fn retune(
        &mut self,
        worker: &Worker,
        position: Duration,
        target: RetuneTarget,
    ) -> Result<(), String> {
        let geometry = self.geometry.retuned(&target);
        let position = target
            .range
            .map_or(position, |range| range.position(position));
        let request = geometry.request(&target.file, position, self.seek_index.clone())?;
        self.pending = Some(Retune {
            voice: Voice::prepare(&self.router, self.voice.volume(), worker, request),
            target,
            retries: 0,
            submitted: false,
        });
        Ok(())
    }

    fn advance_retune(&mut self, worker: &Worker) -> Result<bool, String> {
        let position = self.play_position();
        let Some(pending) = self.pending.as_mut() else {
            return Ok(false);
        };
        let missed = if pending.submitted {
            match pending.voice.handoff() {
                Handoff::Waiting => return Ok(false),
                Handoff::Switched => {
                    let pending = self.pending.take().expect("a prepared preview");
                    self.geometry = self.geometry.retuned(&pending.target);
                    self.file = pending.target.file;
                    self.voice = pending.voice;
                    return Ok(true);
                }
                Handoff::Missed => true,
            }
        } else {
            false
        };
        if !missed && pending.voice.failed() {
            self.pending = None;
            return Err("The updated audio could not be decoded".into());
        }
        if !missed && !pending.voice.ready() {
            return Ok(false);
        }
        let start = pending.voice.position();
        let position = pending
            .target
            .range
            .map_or(position, |range| range.position(position));
        // The new voice's position is rounded to its source frame. The
        // outgoing device clock may sit just before that frame without
        // having wrapped to an earlier pass of the loop.
        let wrapped = start.saturating_sub(position) > frame_time(1, self.geometry.sample_rate);
        let pending = self.pending.as_mut().expect("a prepared preview");
        if !missed && !wrapped && pending.voice.covers(position) {
            pending.voice.pause(!self.playing);
            pending.voice.submit_handoff(
                &self.router,
                position,
                &self.voice,
                self.geometry.audition,
                pending.target.range,
            );
            pending.submitted = true;
            return Ok(false);
        }
        let wait_for_loop = pending.target.range.is_none()
            && self.geometry.audition.is_some_and(|range| {
                let rate = self.geometry.sample_rate;
                frame(start, rate) == frame(range.start, rate)
                    && range.end.saturating_sub(position) <= Duration::from_millis(40)
            });
        if !missed && !wrapped && (!pending.voice.full() || wait_for_loop) {
            return Ok(false);
        }
        let Retune {
            voice,
            target,
            retries,
            ..
        } = self.pending.take().expect("a prepared preview");
        drop(voice);
        // A prepared voice that can still meet the playhead may become
        // audible during a drag. Once it misses, a queued newer edit gets
        // the decoder instead of waiting behind retries of the old sound.
        if self.queued.is_some() {
            return Ok(false);
        }
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
        self.retune(worker, start, target)?;
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
    fn retuned(&self, target: &RetuneTarget) -> Self {
        let mut geometry = self.clone();
        geometry.treatment = target.treatment.clone();
        if let Some(range) = target.range {
            geometry.end = range.end;
            geometry.audition = range.repeat.then_some(StudioAudition {
                start: range.start,
                end: range.end,
                smooth: range.smooth,
            });
        }
        geometry
    }

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
            smooth_loop: self.audition.is_some_and(|audition| audition.smooth),
            seek_index,
            treatment: self.treatment.clone(),
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
