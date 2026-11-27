//! Native audio backend for the in-app preview player.
//!
//! Playback lives on a dedicated thread that owns a rodio [`Sink`] (rodio's
//! `OutputStream` is `!Send`, so it can't sit in Tauri's shared state). The UI
//! talks to it through [`Player`]: commands go over an mpsc channel, and a
//! shared [`PlayerStatus`] snapshot is polled back for the seek bar / time.
//!
//! Gapless is the whole point (#30): the sink is kept primed with the current
//! track *and* the next one, so rodio streams from one into the other with no
//! decode gap at the boundary. The frontend feeds the "next" track whenever the
//! current one changes (see the status `wants_next` flag), which also realizes
//! auto-advance (#29) natively — the sink simply plays through the queue.
//!
//! Every format we handle decodes through rodio/Symphonia, including OGG, which
//! the previous WebView `<audio>` player couldn't play.
//!
//! The same decoder also feeds tempo detection in [`tempo`] (#536).
//!
//! The thread outlives a missing output device: it keeps taking commands, says
//! so in the status, and retries the device on the next play.

mod aiff;
mod controlled;
mod fixed;
mod mailbox;
mod mp4;
mod router;
mod studio;
pub use mailbox::PlayerError;
pub use studio::{StudioAudition, StudioPreviewRange, StudioPreviewState, StudioStatus};
pub mod tempo;

use controlled::{Control, Controlled, Ticket};
use std::collections::VecDeque;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use rodio::cpal::traits::HostTrait;
use rodio::cpal::{self, BufferSize, SupportedBufferSize};
use rodio::stream::{DeviceSinkBuilder, DeviceSinkError, MixerDeviceSink};
use rodio::DeviceTrait;
// rodio 0.22 renamed its `Sink` to `Player`, which is also the name of the
// handle in this module — keep the queue on the device under its old name so
// the two never read as the same thing.
use rodio::{Decoder, Player as RodioSink};
use serde::Serialize;
use tagrex_core::model::TagEngine;

/// Snapshot of the player, polled by the UI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlayerStatus {
    /// Path of the track currently playing, or `None` when idle.
    pub path: Option<String>,
    pub is_paused: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    /// True when a track is playing but no next track is queued yet, so the UI
    /// should feed the next one to keep playback gapless.
    pub wants_next: bool,
    /// The last seek on this track was refused by the decoder (#190). The clock
    /// does NOT move in that case, so the bar has to go back to where the audio
    /// really is — and say so, rather than showing a position nothing is playing
    /// from. Cleared when a track starts or playback stops.
    pub seek_refused: bool,
    /// A track-bound seek is waiting for the audio consumer to acknowledge it.
    pub seek_pending: bool,
    /// No audio output device could be opened: a headless session, a sound
    /// server that is not running, or a device that is busy or unplugged. The
    /// player keeps running and a later play retries the device, so this can
    /// clear without a restart. Nothing is playing while it is set.
    pub output_unavailable: bool,
}

enum Cmd {
    Play(PathBuf),
    SetNext(PathBuf),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    SetVolume(f32),
    Studio(studio::Command),
}

/// Handle to the audio thread. `Send + Sync`, so it lives directly in Tauri's
/// managed state.
pub struct Player {
    tx: mailbox::Sender,
    status: Arc<Mutex<PlayerStatus>>,
    studio_status: studio::Shared,
}

impl Player {
    pub fn new() -> Self {
        let (tx, rx) = mailbox::channel();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let status_for_thread = Arc::clone(&status);
        let studio_status = studio::Shared::default();
        let studio_for_thread = studio_status.clone();
        std::thread::spawn(move || {
            audio_thread(rx, status_for_thread, studio_for_thread, open_output)
        });
        Self {
            tx,
            status,
            studio_status,
        }
    }

    pub fn play(&self, path: PathBuf) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Play(path))
    }
    pub fn set_next(&self, path: PathBuf) -> Result<(), PlayerError> {
        self.tx.send(Cmd::SetNext(path))
    }
    pub fn pause(&self) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Pause)
    }
    pub fn resume(&self) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Resume)
    }
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Stop)
    }
    pub fn seek(&self, secs: f64) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Seek(secs))
    }
    pub fn set_volume(&self, level: f32) -> Result<(), PlayerError> {
        self.tx.send(Cmd::SetVolume(level))
    }
    pub fn status(&self) -> PlayerStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn studio_play(
        &self,
        owner: String,
        file: std::fs::File,
        start: Duration,
        end: Duration,
        audition: Option<StudioAudition>,
        sample_rate: u32,
    ) -> Result<(), PlayerError> {
        self.studio_play_request(
            owner,
            0,
            StudioPlayback {
                file,
                start,
                end,
                audition,
                sample_rate,
                seek_index: None,
                treatment: Default::default(),
            },
        )
    }

    pub fn studio_play_request(
        &self,
        owner: String,
        sequence: u64,
        request: StudioPlayback,
    ) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Studio(studio::Command::Play {
            owner,
            sequence,
            file: request.file,
            start: request.start,
            end: request.end,
            audition: request.audition,
            sample_rate: request.sample_rate,
            seek_index: request.seek_index,
            treatment: Box::new(request.treatment),
        }))
    }

    pub fn studio_seek(&self, owner: String, position: Duration) -> Result<(), PlayerError> {
        self.studio_seek_sequenced(owner, 0, position)
    }

    pub fn studio_seek_sequenced(
        &self,
        owner: String,
        sequence: u64,
        position: Duration,
    ) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Studio(studio::Command::Seek {
            owner,
            sequence,
            position,
        }))
    }

    pub fn studio_pause(&self, owner: String) -> Result<(), PlayerError> {
        self.studio_pause_sequenced(owner, 0)
    }

    pub fn studio_pause_sequenced(&self, owner: String, sequence: u64) -> Result<(), PlayerError> {
        self.tx
            .send(Cmd::Studio(studio::Command::Pause { owner, sequence }))
    }

    pub fn studio_retune(
        &self,
        owner: String,
        sequence: u64,
        revision: u64,
        file: std::fs::File,
        treatment: tagrex_audio_workbench::PreparedTreatment,
    ) -> Result<(), PlayerError> {
        self.studio_retune_range(owner, sequence, revision, file, treatment, None)
    }

    pub fn studio_retune_range(
        &self,
        owner: String,
        sequence: u64,
        revision: u64,
        file: std::fs::File,
        treatment: tagrex_audio_workbench::PreparedTreatment,
        range: Option<StudioPreviewRange>,
    ) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Studio(studio::Command::Retune {
            owner,
            sequence,
            revision,
            file,
            treatment: Box::new(treatment),
            range,
        }))
    }

    pub fn studio_close(&self, owner: String) -> Result<(), PlayerError> {
        self.tx.send(Cmd::Studio(studio::Command::Close { owner }))
    }

    pub fn studio_playback_window(
        &self,
        owner: &str,
        frame: u64,
        settings: tagrex_audio_workbench::AudioTreatment,
    ) -> (bool, Option<tagrex_audio_workbench::PlaybackProbeWindow>) {
        self.studio_status.playback_window(owner, frame, settings)
    }

    pub fn studio_status(&self, owner: &str) -> StudioStatus {
        self.studio_status.for_owner(owner)
    }
}

pub struct StudioPlayback {
    pub file: std::fs::File,
    pub start: Duration,
    pub end: Duration,
    pub audition: Option<StudioAudition>,
    pub sample_rate: u32,
    pub seek_index: Option<std::sync::Arc<tagrex_audio_workbench::SeekIndex>>,
    pub treatment: tagrex_audio_workbench::PreparedTreatment,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

/// Exercise the complete playback decoder for the separate fuzz workspace.
/// Normal app builds do not expose or call this probe.
#[cfg(feature = "fuzzing")]
pub fn fuzz_decode(path: &std::path::Path) -> Result<(), String> {
    for _ in open_decoder(path)? {}
    Ok(())
}

/// One queued track: its path and total duration (from lofty, since rodio's
/// `total_duration` is unreliable for MP3).
struct Track {
    /// Shared so telling one track from another in a status comparison is a
    /// pointer count rather than a path copy.
    path: Arc<PathBuf>,
    duration: Duration,
}

/// A decoded track, as it is handed to the output.
type Decoded = PanicSafe<Decoder<BufReader<aiff::AudioFile>>>;

/// The output the player thread holds: the one place a device is opened or a
/// sink is made from it.
///
/// It is a trait so the playback loop can be driven without a sound card: the
/// queue, gapless priming, clock and status behaviour is the part that has to
/// be right, and a machine with no device would otherwise leave all of it
/// untested. Production uses [`Output`]'s one implementation, [`Device`].
trait Output {
    /// The sink tracks are queued on. One is made per Play and per Stop, and a
    /// fresh one is always full volume, so `volume` is applied as it is made.
    type Sink: Sink;

    fn sink(&self, volume: f32) -> Self::Sink;

    fn studio_router(&self) -> Option<router::Handle> {
        None
    }
}

/// The sink operations the playback loop uses, so the loop does not depend on
/// rodio's types: seeking, which the decoder may refuse, and the number of
/// tracks still to play are the only answers it needs.
trait Sink {
    fn append(&self, source: Decoded);
    fn stop(&self);
    fn play(&self);
    fn pause(&self);
    fn set_volume(&self, volume: f32);
    fn request_seek(&self, to: Duration, queued: usize) -> Result<Ticket, String>;
    /// Tracks appended and not yet played out. A track that has reached its
    /// end stops counting, which is how the loop notices a track boundary.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Rodio handles mixing, gain and queue completion. Source-local controls own
/// pause and seek so its blocking sink-level seek API is never called.
struct DevicePlayer {
    sink: RodioSink,
    sources: Mutex<VecDeque<Control>>,
    paused: Arc<AtomicBool>,
    format: (rodio::ChannelCount, rodio::SampleRate),
}
impl Sink for DevicePlayer {
    fn append(&self, source: Decoded) {
        let source = fixed::Fixed::new(source, self.format.0, self.format.1);
        let (source, control) = Controlled::new_fixed(source, self.paused.clone());
        let mut sources = self.sources.lock().unwrap_or_else(|p| p.into_inner());
        while sources.len() > self.sink.len() {
            sources.pop_front();
        }
        sources.push_back(control);
        drop(sources);
        self.sink.append(source);
    }
    fn stop(&self) {
        self.sink.stop();
    }
    fn play(&self) {
        self.paused.store(false, Ordering::Release);
    }
    fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }
    fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
    }
    fn request_seek(&self, target: Duration, queued: usize) -> Result<Ticket, String> {
        let mut sources = self.sources.lock().unwrap_or_else(|p| p.into_inner());
        while sources.len() > queued {
            sources.pop_front();
        }
        let source = sources.front().ok_or("No track remains to seek")?;
        Ok(source.seek(target))
    }
    fn len(&self) -> usize {
        self.sink.len()
    }
}

/// A real audio output device.
struct Device {
    _output: MixerDeviceSink,
    router: router::Handle,
    studio_router: std::cell::OnceCell<router::Handle>,
}

impl DevicePlayer {
    fn new(router: &router::Handle, volume: f32) -> Self {
        let (sink, output) = RodioSink::new();
        router.replace(output);
        sink.set_volume(volume);
        Self {
            sink,
            sources: Mutex::new(VecDeque::new()),
            paused: Arc::new(AtomicBool::new(false)),
            format: router.format(),
        }
    }
}
impl Output for Device {
    type Sink = DevicePlayer;
    fn sink(&self, volume: f32) -> DevicePlayer {
        DevicePlayer::new(&self.router, volume)
    }

    fn studio_router(&self) -> Option<router::Handle> {
        Some(
            self.studio_router
                .get_or_init(|| {
                    let (channels, rate) = self.router.format();
                    let (handle, source) = router::Router::new(channels, rate);
                    self._output.mixer().add(source);
                    handle
                })
                .clone(),
        )
    }
}

/// The playback loop.
///
/// `open` is how the thread gets an output device. It is a parameter so tests
/// can drive this loop with an opener that fails deterministically — on a
/// developer machine the real opener would find the sound card and prove
/// nothing about the no-device path — and, through [`Output`], with a sink that
/// needs no sound card at all.
fn audio_thread<O, F>(
    rx: mailbox::Receiver,
    status: Arc<Mutex<PlayerStatus>>,
    studio_status: studio::Shared,
    mut open: F,
) where
    O: Output,
    F: FnMut() -> Result<O, DeviceSinkError>,
{
    // A missing output device (a headless box, a sound server that is not
    // running yet, a device that is busy at login) must not end this thread:
    // the user can plug one in or start the sound server later, and the next
    // play retries the device, so preview recovers without an app restart.
    // Until then `output_unavailable` says why nothing plays.
    let mut device = match open() {
        Ok(device) => Some(device),
        Err(err) => {
            eprintln!("audio: no output device, preview waits for one: {err}");
            None
        }
    };

    // Play and Stop each replace the sink, and a fresh sink is always full
    // volume — so the level has to live out here and be re-applied to every new
    // sink, or turning the volume down would silently reset on the next track.
    // It also outlives a stretch with no device, so a level set then applies to
    // the sink the first device-backed play creates.
    let mut volume: f32 = 1.0;
    let mut sink = fresh_sink(device.as_ref(), volume);
    let mut queue: VecDeque<Track> = VecDeque::new();
    let mut clock = PlayClock::default();
    let mut paused = false;
    // Whether the decoder refused the last seek on the current track (#190).
    let mut seek_refused = false;
    let mut pending_seek: Option<PendingSeek> = None;
    let mut studio = studio::Actor::new(studio_status);
    // What the UI was last told. Compared before anything is built, so a tick
    // that changes nothing costs no allocation and takes no lock.
    let mut published = StatusKey::default();

    // Publish the startup state before waiting for the first command, so a
    // status poll right after launch already reports a missing device.
    write_status(
        &status,
        &mut published,
        StatusKey::of(
            &queue,
            &clock,
            paused,
            seek_refused,
            device.is_none(),
            pending_seek.is_some(),
        ),
    );

    loop {
        // Wait for a command. While a track is queued the thread wakes every
        // 200 ms to notice its end and refresh the status snapshot; with
        // nothing queued there is nothing to advance and nothing to report, so
        // it parks until a command arrives instead of ticking an idle player
        // five times a second.
        let wait = if pending_seek.is_some() || studio.needs_tick() {
            Duration::from_millis(10)
        } else if queue.is_empty() {
            IDLE_WAIT
        } else {
            PLAYING_TICK
        };
        let received = rx.recv_timeout(wait);
        if let Ok(cmd) = &received {
            // The main controls belong to the user even while Studio is
            // open. A later Studio close cannot revive older playback.
            if matches!(
                cmd,
                Cmd::Play(_) | Cmd::Pause | Cmd::Resume | Cmd::Stop | Cmd::Seek(_)
            ) && studio.revoke()
                && matches!(cmd, Cmd::Seek(_))
                && !paused
            {
                if let Some(sink) = sink.as_ref() {
                    sink.play();
                }
                clock.resume();
            }
        }
        match received {
            Ok(cmd) => match cmd {
                Cmd::Play(path) => {
                    if let Some(pending) = pending_seek.take() {
                        pending.ticket.cancel();
                    }
                    // The one place a device is retried: a track the user has
                    // just asked for plays as soon as a device is there. A
                    // second failure is harmless and leaves the status honest.
                    if device.is_none() {
                        match open() {
                            Ok(fresh) => device = Some(fresh),
                            Err(err) => eprintln!("audio: still no output device: {err}"),
                        }
                    }
                    // Silence the old sink, then play on a fresh one — a new
                    // Sink guarantees a clean, re-appendable queue regardless of
                    // rodio's drop-vs-stop semantics.
                    if let Some(old) = sink.take() {
                        old.stop();
                    }
                    sink = fresh_sink(device.as_ref(), volume);
                    queue.clear();
                    paused = false;
                    seek_refused = false;
                    // Without a device there is no sink to append to: the track
                    // is not reported as playing, so the status stays idle.
                    let started = sink
                        .as_ref()
                        .is_some_and(|sink| enqueue(sink, &mut queue, path));
                    if started {
                        clock.start();
                    } else {
                        clock.stop();
                    }
                }
                Cmd::SetNext(path) => {
                    // Only prime a next track when exactly the current one is
                    // queued; ignore otherwise (already primed, or idle).
                    if queue.len() == 1 {
                        if let Some(sink) = sink.as_ref() {
                            enqueue(sink, &mut queue, path);
                        }
                    }
                }
                Cmd::Pause => {
                    if !queue.is_empty() {
                        if let Some(sink) = sink.as_ref() {
                            sink.pause();
                            clock.pause();
                            paused = true;
                        }
                    }
                }
                Cmd::Resume => {
                    if !queue.is_empty() {
                        if let Some(sink) = sink.as_ref() {
                            sink.play();
                            clock.resume();
                            paused = false;
                        }
                    }
                }
                Cmd::Stop => {
                    if let Some(pending) = pending_seek.take() {
                        pending.ticket.cancel();
                    }
                    if let Some(old) = sink.take() {
                        old.stop();
                    }
                    sink = fresh_sink(device.as_ref(), volume);
                    queue.clear();
                    clock.stop();
                    paused = false;
                    seek_refused = false;
                }
                Cmd::SetVolume(level) => {
                    volume = level.clamp(0.0, 1.0);
                    if let Some(sink) = sink.as_ref() {
                        sink.set_volume(volume);
                    }
                    studio.set_volume(volume);
                }
                Cmd::Seek(secs) => {
                    if !queue.is_empty() {
                        // The clock is what the UI shows, so it may only move
                        // when the audio did (#190). A sink can also finish the
                        // track between that check and the seek, so what the
                        // sink reports afterwards decides — not the `Ok`.
                        let target = Duration::from_secs_f64(secs.max(0.0)).min(
                            queue.front().map_or(Duration::MAX, |track| {
                                if track.duration.is_zero() {
                                    Duration::MAX
                                } else {
                                    track.duration
                                }
                            }),
                        );
                        let previous = pending_seek.take();
                        if let Some(previous) = &previous {
                            previous.ticket.supersede();
                        }
                        match seek(sink.as_ref(), target, queue.len()) {
                            SeekOutcome::Seeked => {
                                clock.seek(target);
                                seek_refused = false;
                            }
                            SeekOutcome::Pending(ticket) => {
                                pending_seek = Some(PendingSeek {
                                    ticket,
                                    track: queue.front().expect("nonempty seek queue").path.clone(),
                                    started: Instant::now(),
                                });
                            }
                            SeekOutcome::Refused(err) => {
                                if let Some(previous) = &previous {
                                    previous.ticket.cancel();
                                }
                                eprintln!("audio: seek refused: {err}");
                                seek_refused = true;
                            }
                            // Nothing is left to seek; the boundary below takes
                            // the player idle.
                            SeekOutcome::PlayedOut => {
                                if let Some(previous) = &previous {
                                    previous.ticket.cancel();
                                }
                            }
                        }
                    }
                }
                Cmd::Studio(command) => {
                    let owned_before = studio.owns_output();
                    if command.requests_play() && studio.accepts(&command) && device.is_none() {
                        match open() {
                            Ok(fresh) => device = Some(fresh),
                            Err(err) => eprintln!("audio: Studio output is unavailable: {err}"),
                        }
                    }
                    studio.handle(command, device.as_ref(), volume);
                    if !owned_before && studio.owns_output() && !queue.is_empty() && !paused {
                        if let Some(sink) = sink.as_ref() {
                            sink.pause();
                        }
                        clock.pause();
                    }
                    if owned_before && !studio.owns_output() && !paused {
                        if let Some(sink) = sink.as_ref() {
                            sink.play();
                        }
                        clock.resume();
                    }
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if let Some(pending) = pending_seek.take() {
            let same_track = queue
                .front()
                .is_some_and(|track| Arc::ptr_eq(&track.path, &pending.track))
                && sink.as_ref().is_some_and(|sink| sink.len() == queue.len());
            if !same_track {
                pending.ticket.cancel();
            } else if let Some(result) = pending.ticket.poll() {
                match result {
                    Ok((target, acknowledged)) => {
                        // Paused seeks stay frozen; running seeks include the
                        // small acknowledgement-to-publication interval.
                        clock.seek_at(
                            target,
                            if paused || studio.owns_output() {
                                Instant::now()
                            } else {
                                acknowledged
                            },
                        );
                        pending.ticket.accept();
                        seek_refused = false;
                    }
                    Err(error) => {
                        pending.ticket.accept();
                        eprintln!("audio: seek refused: {error}");
                        seek_refused = true;
                    }
                }
            } else if pending.started.elapsed() >= SEEK_ACK_TIMEOUT {
                pending.ticket.cancel();
                eprintln!("audio: seek refused: output did not acknowledge within the deadline");
                seek_refused = true;
            } else {
                pending_seek = Some(pending);
            }
        }

        let owned_before = studio.owns_output();
        studio.tick();
        if owned_before && !studio.owns_output() && !paused {
            if let Some(sink) = sink.as_ref() {
                sink.play();
            }
            clock.resume();
        }

        // The sink drains sources as they finish. When it holds fewer than our
        // queue, a track boundary was crossed: drop the finished front and
        // restart the clock for the new current track.
        while queue.len() > sink.as_ref().map_or(0, Sink::len) && !queue.is_empty() {
            queue.pop_front();
            clock.start();
            if paused || studio.owns_output() {
                clock.pause();
            }
            seek_refused = false; // a new track, and a new answer on seeking
        }
        if queue.is_empty() {
            clock.stop();
            paused = false;
            seek_refused = false;
        }

        write_status(
            &status,
            &mut published,
            StatusKey::of(
                &queue,
                &clock,
                paused || studio.owns_output() && !queue.is_empty(),
                seek_refused,
                device.is_none(),
                pending_seek.is_some(),
            ),
        );
    }
}

/// Open the default output device.
///
/// rodio asks every device for a fixed ~50 ms buffer — 2048 frames at 48 kHz —
/// and since cpal 0.18 a device whose range stops short of that refuses the
/// stream: Bluetooth headphones on macOS take at most 960 (#513). rodio's own
/// fallback tries no other configuration there, so the retry is ours: the same
/// device, with rodio's 50 ms clamped into the range the device reports.
fn open_output() -> Result<Device, DeviceSinkError> {
    let mut output = open_device()?;
    // The device sink lives as long as the player thread; its parting message
    // on drop is noise in a terminal-launched build.
    output.log_on_drop(false);
    let config = output.config();
    let (router, source) = router::Router::new(config.channel_count(), config.sample_rate());
    output.mixer().add(source);
    Ok(Device {
        _output: output,
        router,
        studio_router: std::cell::OnceCell::new(),
    })
}

fn open_device() -> Result<MixerDeviceSink, DeviceSinkError> {
    let first = match DeviceSinkBuilder::open_default_sink() {
        Ok(device) => return Ok(device),
        Err(err) => err,
    };
    let fitted = || -> Option<MixerDeviceSink> {
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        let SupportedBufferSize::Range { min, max } = *config.buffer_size() else {
            return None;
        };
        let frames = (config.sample_rate() / 20).clamp(min, max);
        DeviceSinkBuilder::from_device(device)
            .ok()?
            .with_buffer_size(BufferSize::Fixed(frames))
            .open_stream()
            .ok()
    };
    fitted().ok_or(first)
}

/// A fresh queue on the output device. Play and Stop each start one, because a
/// new sink guarantees a clean, re-appendable queue regardless of rodio's
/// drop-vs-stop semantics. Without a device there is nothing to build one on,
/// so the level is carried by [`audio_thread`]'s `volume` until a device is
/// open again.
fn fresh_sink<O: Output>(device: Option<&O>, volume: f32) -> Option<O::Sink> {
    device.map(|output| output.sink(volume))
}

/// Decode `path` and append it to the sink, recording it in `queue`. Returns
/// false (and leaves the queue untouched) if the file can't be opened/decoded.
fn enqueue<S: Sink>(sink: &S, queue: &mut VecDeque<Track>, path: PathBuf) -> bool {
    let decoder = match open_decoder(&path) {
        Ok(decoder) => decoder,
        Err(err) => {
            eprintln!("audio: can't play {}: {err}", path.display());
            return false;
        }
    };
    let duration = TagEngine::read_duration(&path).unwrap_or(Duration::ZERO);
    sink.append(decoder);
    queue.push_back(Track {
        path: Arc::new(path),
        duration,
    });
    true
}

/// A seek is bound to the tracked source. A boundary cancels its effect on
/// the visible clock; it never requires rebuilding or seeking the next track.
struct PendingSeek {
    ticket: Ticket,
    track: Arc<PathBuf>,
    started: Instant,
}
const SEEK_ACK_TIMEOUT: Duration = Duration::from_secs(2);

enum SeekOutcome {
    Pending(Ticket),
    /// A source sought and the sink still holds it: the clock may follow.
    Seeked,
    /// The decoder refused; the audio did not move.
    Refused(String),
    /// The last track played out during the call; there is nothing to follow,
    /// and the boundary check takes the player idle.
    PlayedOut,
}

fn seek<S: Sink>(sink: Option<&S>, target: Duration, queued: usize) -> SeekOutcome {
    // No sink, or a sink that has already played out: nothing to seek, and no
    // audio for a clock to follow.
    let Some(sink) = sink.filter(|sink| !sink.is_empty()) else {
        return SeekOutcome::PlayedOut;
    };
    // Compare with the player's tracked queue, not an already advanced sink.
    // Otherwise a boundary just before this call can hide the same race.
    if sink.len() < queued {
        return SeekOutcome::PlayedOut;
    }
    let result = sink.request_seek(target, queued);
    // Even an acknowledged source-local request cannot move the visible clock
    // after its source ended. The next track remains primed and untouched.
    if sink.is_empty() {
        return SeekOutcome::PlayedOut;
    }
    if sink.len() < queued {
        return SeekOutcome::PlayedOut;
    }
    match result {
        Ok(ticket) => match ticket.poll() {
            Some(Ok(_)) => {
                ticket.accept();
                SeekOutcome::Seeked
            }
            Some(Err(error)) => {
                ticket.accept();
                SeekOutcome::Refused(error)
            }
            None => SeekOutcome::Pending(ticket),
        },
        Err(err) => SeekOutcome::Refused(err),
    }
}

/// Open `path` for decoding — shared by playback and tempo detection.
///
/// Through [`aiff::AudioFile`], which reads every file as it is except an AIFF
/// with padding before its samples, which Symphonia would refuse (#525). The
/// length goes to the decoder as `TryFrom<File>` would pass it: without it
/// Symphonia calls a FLAC unseekable (#190).
///
/// Symphonia panics on some damaged headers instead of returning an error: an
/// AIFF that states a sample rate of 0 fails an assert, an MP3 it reads as
/// having no channels divides by zero (#721). The player thread opens every
/// queued track here, so one such file stopped the player until a restart. As
/// an error, the file is just one that cannot be played. The same goes for a
/// panic while it decodes, which [`PanicSafe`] turns into the end of the track.
///
/// An MP4 whose sample tables state more entries than the file holds is
/// refused before the decoder sees it: the decoder would reserve room for all
/// of them first, and an allocation that large can abort the app (#727).
fn open_decoder(
    path: &std::path::Path,
) -> Result<PanicSafe<Decoder<BufReader<aiff::AudioFile>>>, String> {
    mp4::check_sample_tables(path)?;
    let file = aiff::AudioFile::open(path).map_err(|err| err.to_string())?;
    let len = file.len();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Decoder::builder()
            .with_data(BufReader::new(file))
            .with_byte_len(len)
            .with_seekable(true)
            .build()
    }))
    .map_err(|_| "the file is damaged: the decoder failed on it".to_string())?
    .map(PanicSafe::new)
    .map_err(|err| err.to_string())
}

/// A source whose panics end it instead of unwinding out (#725).
///
/// Symphonia also panics part-way through some damaged files, not only on
/// their headers: a broken AAC frame indexes past one of its tables. Playback
/// pulls samples on the audio output thread, where nothing of ours could catch
/// that, so one such file could take the app down mid-track; tempo detection
/// only lost its result. A panic now ends the track at
/// that point, as if the file stopped there.
struct PanicSafe<S> {
    inner: S,
    /// Set once the inner source has panicked; it is not touched again.
    failed: bool,
}

impl<S> PanicSafe<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            failed: false,
        }
    }
}

impl<S: rodio::Source> Iterator for PanicSafe<S> {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.next())).unwrap_or_else(
            |_| {
                self.failed = true;
                None
            },
        )
    }
}

impl<S: rodio::Source> rodio::Source for PanicSafe<S> {
    fn current_span_len(&self) -> Option<usize> {
        if self.failed {
            Some(0)
        } else {
            self.inner.current_span_len()
        }
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        if self.failed {
            return Err(rodio::source::SeekError::Other(std::sync::Arc::new(
                std::io::Error::other("the file is damaged: the decoder failed on it"),
            )));
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.try_seek(pos)))
            .unwrap_or_else(|_| {
                self.failed = true;
                Err(rodio::source::SeekError::Other(std::sync::Arc::new(
                    std::io::Error::other("the file is damaged: the decoder failed on it"),
                )))
            })
    }
}

/// How close to the end of the current track the queue gets primed with the
/// next one. Priming at track START (the old behaviour) meant the next source
/// was already appended to the sink the moment playback began — and rodio has
/// no way to remove an appended source, so a Repeat change made mid-track was
/// silently ignored and only took effect one track later. Priming late leaves
/// that decision open for almost the whole track; a few seconds is still far
/// more than enough to open and append the next decoder gaplessly.
const PRIME_LEAD_SECS: f64 = 5.0;

/// How often the thread wakes while a track is queued: the status the UI polls
/// is refreshed at this interval, and a finished track is noticed within it.
const PLAYING_TICK: Duration = Duration::from_millis(200);

/// How long the thread parks with nothing queued. There is no position to
/// advance and no queue to notice, so an idle player has no reason to wake; a
/// command wakes it, and the long timeout is only a safety net.
const IDLE_WAIT: Duration = Duration::from_secs(60);

fn write_status(status: &Arc<Mutex<PlayerStatus>>, published: &mut StatusKey, next: StatusKey) {
    // The common case while paused or idle: nothing the UI could see changed,
    // so no path string is built and the mutex the UI polls is not touched.
    if next == *published {
        return;
    }
    // Built before the lock is taken, so a status poll never waits on this
    // thread's allocation of the path.
    let value = next.to_status();
    *status.lock().unwrap_or_else(|p| p.into_inner()) = value;
    *published = next;
}

/// The part of a status snapshot that decides whether the UI would see a
/// change: plain values plus the track by identity, so comparing it allocates
/// nothing. The published [`PlayerStatus`] is only materialised from this when
/// the comparison says something moved.
#[derive(Default, PartialEq)]
struct StatusKey {
    track: Option<Arc<PathBuf>>,
    is_paused: bool,
    position: Duration,
    duration: Duration,
    wants_next: bool,
    seek_refused: bool,
    seek_pending: bool,
    output_unavailable: bool,
}

impl StatusKey {
    fn of(
        queue: &VecDeque<Track>,
        clock: &PlayClock,
        paused: bool,
        seek_refused: bool,
        output_unavailable: bool,
        seek_pending: bool,
    ) -> Self {
        let Some(current) = queue.front() else {
            return Self {
                output_unavailable,
                ..Self::default()
            };
        };
        let duration = current.duration;
        // Never report past the track length (the wall clock can overshoot
        // by a poll interval before the boundary is detected).
        let position = clock.position().min(if duration > Duration::ZERO {
            duration
        } else {
            Duration::MAX
        });
        // With an unknown duration (0) there's nothing to count down from,
        // so fall back to priming immediately rather than never.
        let near_end = duration <= Duration::ZERO
            || duration.saturating_sub(position) <= Duration::from_secs_f64(PRIME_LEAD_SECS);
        Self {
            track: Some(Arc::clone(&current.path)),
            is_paused: paused,
            position,
            duration,
            wants_next: queue.len() == 1 && near_end,
            seek_refused,
            seek_pending,
            output_unavailable,
        }
    }

    /// The snapshot the UI polls. This is the only place status publication
    /// builds the path string, so it is only called for a changed key.
    fn to_status(&self) -> PlayerStatus {
        PlayerStatus {
            path: self
                .track
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            is_paused: self.is_paused,
            position_secs: self.position.as_secs_f64(),
            duration_secs: self.duration.as_secs_f64(),
            wants_next: self.wants_next,
            seek_refused: self.seek_refused,
            seek_pending: self.seek_pending,
            output_unavailable: self.output_unavailable,
        }
    }
}

/// Wall-clock position tracker for the current track. rodio plays in real time,
/// so elapsed wall time (minus paused spans, plus any seek offset) equals the
/// playback position. Kept free of `rodio` so its arithmetic is unit-testable.
#[derive(Debug)]
struct PlayClock {
    running: bool,
    /// Position at the last (re)start — 0 on a fresh track, the target on seek.
    base: Duration,
    /// When the current running span began.
    started: Instant,
    /// Total paused time accumulated since `base`/`started`.
    paused_total: Duration,
    /// Set while paused, to when the pause began.
    paused_at: Option<Instant>,
}

impl Default for PlayClock {
    fn default() -> Self {
        Self {
            running: false,
            base: Duration::ZERO,
            started: Instant::now(),
            paused_total: Duration::ZERO,
            paused_at: None,
        }
    }
}

impl PlayClock {
    fn start(&mut self) {
        self.start_at(Instant::now());
    }
    fn pause(&mut self) {
        self.pause_at(Instant::now());
    }
    fn resume(&mut self) {
        self.resume_at(Instant::now());
    }
    fn seek(&mut self, to: Duration) {
        self.seek_at(to, Instant::now());
    }
    fn stop(&mut self) {
        self.running = false;
    }
    fn position(&self) -> Duration {
        self.position_at(Instant::now())
    }

    // `*_at` variants take an explicit `now` so the arithmetic can be tested
    // deterministically.
    fn start_at(&mut self, now: Instant) {
        self.running = true;
        self.base = Duration::ZERO;
        self.started = now;
        self.paused_total = Duration::ZERO;
        self.paused_at = None;
    }
    fn pause_at(&mut self, now: Instant) {
        if self.running && self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }
    fn resume_at(&mut self, now: Instant) {
        if let Some(at) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(at);
        }
    }
    fn seek_at(&mut self, to: Duration, now: Instant) {
        self.base = to;
        self.started = now;
        self.paused_total = Duration::ZERO;
        // Preserve paused state: if paused, restart the pause span at `now`.
        self.paused_at = self.paused_at.map(|_| now);
    }
    fn position_at(&self, now: Instant) -> Duration {
        if !self.running {
            return Duration::ZERO;
        }
        let mut elapsed = now.saturating_duration_since(self.started);
        elapsed = elapsed.saturating_sub(self.paused_total);
        if let Some(at) = self.paused_at {
            elapsed = elapsed.saturating_sub(now.saturating_duration_since(at));
        }
        self.base + elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #525: an AIFF with padding before its samples decodes — and to the same
    // audio as the file without it, so playback and tempo detection see the same
    // thing either way. Symphonia alone refuses the padded one.
    #[test]
    fn a_padded_aiff_decodes_like_the_same_file_without_padding() {
        let dir = std::env::temp_dir().join(format!("tagrex-player-aiff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let padded = dir.join("padded.aiff");
        let plain = dir.join("plain.aiff");
        std::fs::write(&padded, aiff::tests::aiff(44_100, 2280)).unwrap();
        std::fs::write(&plain, aiff::tests::aiff(44_100, 0)).unwrap();

        let refused = Decoder::try_from(std::fs::File::open(&padded).unwrap());
        assert!(refused.is_err(), "the decoder alone now reads padded AIFF");

        let samples = |path: &std::path::Path| -> Vec<i16> {
            open_decoder(path)
                .unwrap()
                .map(|s| (s * 32768.0) as i16)
                .collect()
        };
        // Symphonia 0.5 counts the SSND chunk's offset and blockSize fields as
        // sample data and reads two frames past the end of every AIFF, padded
        // or not — so the comparison is with the plain file, not a frame count.
        let read = samples(&padded);
        assert!(read.len() >= 44_100 * 2);
        assert_eq!(read, samples(&plain));

        // Seeking into the middle lands on the frame the plain file has there.
        use rodio::Source;
        let mut decoder = open_decoder(&padded).unwrap();
        decoder.try_seek(Duration::from_millis(500)).unwrap();
        let at = (decoder.next().unwrap() * 32768.0) as i16;
        let mut reference = open_decoder(&plain).unwrap();
        reference.try_seek(Duration::from_millis(500)).unwrap();
        assert_eq!(at, (reference.next().unwrap() * 32768.0) as i16);
    }

    // #725: an AAC the decoder panics on part-way through, minimized from a
    // fuzz input. The track ends where it broke instead of unwinding out of
    // whoever pulls the samples (on playback, the audio output thread).
    #[test]
    fn a_panic_while_decoding_ends_the_track() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/decode-panic.aac");
        let mut decoder = open_decoder(&path).expect("the header reads");
        let pulled = decoder.by_ref().count();
        assert!(decoder.failed, "the decoder no longer panics on this file");
        assert_eq!(
            decoder.next(),
            None,
            "a failed track kept going after {pulled} samples"
        );
        use rodio::Source;
        assert_eq!(decoder.current_span_len(), Some(0));
        assert!(decoder.try_seek(Duration::ZERO).is_err());
    }

    // #721: headers the decoder panics on, minimized from fuzz inputs, come
    // back as an error instead of unwinding out of the player thread.
    #[test]
    fn a_header_the_decoder_panics_on_is_an_error() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for name in ["zero-sample-rate.aiff", "zero-channels.mp3"] {
            let path = fixtures.join(name);
            assert!(open_decoder(&path).is_err(), "{name} opened");
        }
    }

    // #727: an M4A whose `stsc` states about 1.87 billion entries, from the
    // fuzzer. The decoder would reserve 30 GB for them before reading one; the
    // file is refused before it gets that far.
    #[test]
    fn an_mp4_sample_table_larger_than_the_file_is_refused() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/huge-sample-table.m4a");
        let err = open_decoder(&path).err().expect("the file opened");
        assert!(
            err.contains("sample tables"),
            "refused for another reason: {err}"
        );
    }

    #[test]
    fn clock_tracks_elapsed_pause_and_seek() {
        let t0 = Instant::now();
        let mut c = PlayClock::default();
        assert_eq!(c.position_at(t0), Duration::ZERO); // idle

        c.start_at(t0);
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(3)),
            Duration::from_secs(3)
        );

        // Pause at 3s, hold for 2s: position frozen at 3s.
        c.pause_at(t0 + Duration::from_secs(3));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(5)),
            Duration::from_secs(3)
        );

        // Resume at 5s: at 6s wall time, 1s more played -> 4s.
        c.resume_at(t0 + Duration::from_secs(5));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(6)),
            Duration::from_secs(4)
        );

        // Seek to 30s at 6s: at 7s wall time -> 31s.
        c.seek_at(Duration::from_secs(30), t0 + Duration::from_secs(6));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(7)),
            Duration::from_secs(31)
        );

        // A fresh track resets to 0.
        c.start_at(t0 + Duration::from_secs(7));
        assert_eq!(c.position_at(t0 + Duration::from_secs(7)), Duration::ZERO);

        c.stop();
        assert_eq!(c.position_at(t0 + Duration::from_secs(8)), Duration::ZERO);
    }

    #[test]
    fn seek_while_paused_keeps_position_frozen() {
        let t0 = Instant::now();
        let mut c = PlayClock::default();
        c.start_at(t0);
        c.pause_at(t0 + Duration::from_secs(2));
        // Seek to 50s while paused; time keeps passing but position holds at 50.
        c.seek_at(Duration::from_secs(50), t0 + Duration::from_secs(4));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(10)),
            Duration::from_secs(50)
        );
        // Resume at 10s: at 11s -> 51s.
        c.resume_at(t0 + Duration::from_secs(10));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(11)),
            Duration::from_secs(51)
        );
    }
}

/// Fixtures and a fake output, so the playback loop's queue, priming, clock and
/// status behaviour is testable on a machine with no sound card — and
/// deterministically on one that has a card, where the real opener would only
/// prove that this machine plays audio.
#[cfg(test)]
mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Something the playback loop asked of its output, in order.
    #[derive(Clone, Debug, PartialEq)]
    pub(super) enum Call {
        /// A fresh sink was made, at this volume.
        NewSink(f32),
        Append,
        Stop,
        Play,
        Pause,
        Volume(f32),
        Seek(Duration),
    }

    /// What every sink of a fake output was asked to do, and whether seeking is
    /// refused the way a decoder that cannot seek does.
    #[derive(Default)]
    pub(super) struct Audio {
        pub(super) calls: Vec<Call>,
        pub(super) refuse_seek: bool,
        /// How many tracks a seek lets play out, as a track reaching its end
        /// while the seek is in flight does.
        drain_on_seek: usize,
        /// Held by a seek to stand in for a device that stopped pulling: rodio
        /// waits for the audio thread to acknowledge a seek, and a stalled
        /// device never does.
        seek_gate: Option<Arc<SeekGate>>,
        /// The queue of each sink made so far, oldest first.
        sinks: Vec<Arc<AtomicUsize>>,
    }

    /// A one-way gate a fake seek can wait on, so a test can decide exactly how
    /// long a blocked output stays blocked.
    pub(super) struct SeekGate {
        open: Mutex<bool>,
        ticket: Mutex<Option<Ticket>>,
    }

    impl SeekGate {
        fn submit(&self, ticket: Ticket) {
            let open = self.open.lock().unwrap();
            if *open {
                ticket.finish(Ok(()));
            } else {
                *self.ticket.lock().unwrap() = Some(ticket);
            }
        }
        pub(super) fn release(&self) {
            *self.open.lock().unwrap() = true;
            if let Some(ticket) = self.ticket.lock().unwrap().take() {
                ticket.finish(Ok(()));
            }
        }
    }

    /// The output the test thread and the audio thread both look at. Sound cards
    /// are absent on purpose: a track is counted when it is appended and played
    /// out when the test says the device reached its end.
    #[derive(Clone)]
    pub(super) struct FakeOutput(Arc<Mutex<Audio>>);

    impl FakeOutput {
        pub(super) fn new() -> Self {
            Self(Arc::new(Mutex::new(Audio::default())))
        }

        /// Everything the loop asked of the output, in order.
        pub(super) fn calls(&self) -> Vec<Call> {
            self.0.lock().unwrap().calls.clone()
        }

        /// Make every seek on this output fail, as a decoder without seek
        /// support does (#190).
        pub(super) fn refuse_seeks(&self) {
            self.0.lock().unwrap().refuse_seek = true;
        }

        /// Let the current track play out while the next seek is in flight, as
        /// a track reaching its end inside `try_seek` does.
        pub(super) fn finish_a_track_during_seek(&self) {
            self.0.lock().unwrap().drain_on_seek = 1;
        }

        /// Make the next seeks wait until the returned gate is released, as a
        /// device that stopped pulling leaves rodio waiting for feedback.
        pub(super) fn hold_seeks(&self) -> Arc<SeekGate> {
            let gate = Arc::new(SeekGate {
                open: Mutex::new(false),
                ticket: Mutex::new(None),
            });
            self.0.lock().unwrap().seek_gate = Some(gate.clone());
            gate
        }

        /// How many tracks the sink playing now still has to play. Zero before
        /// the thread has made one.
        pub(super) fn queued(&self) -> usize {
            let audio = self.0.lock().unwrap();
            audio
                .sinks
                .last()
                .map_or(0, |queue| queue.load(Ordering::SeqCst))
        }

        /// Play out the track at the front of the queue, as the device does when
        /// it reaches the end of one.
        pub(super) fn finish_front(&self) {
            let queue = self.current();
            assert!(queue.load(Ordering::SeqCst) > 0, "no track left to finish");
            queue.fetch_sub(1, Ordering::SeqCst);
        }

        /// The queue of the sink made last — the one playing now.
        fn current(&self) -> Arc<AtomicUsize> {
            self.0
                .lock()
                .unwrap()
                .sinks
                .last()
                .expect("no sink was ever made")
                .clone()
        }
    }

    impl Output for FakeOutput {
        type Sink = FakeSink;

        fn sink(&self, volume: f32) -> FakeSink {
            let queue = Arc::new(AtomicUsize::new(0));
            let mut audio = self.0.lock().unwrap();
            audio.sinks.push(queue.clone());
            audio.calls.push(Call::NewSink(volume));
            FakeSink {
                queue,
                audio: self.0.clone(),
            }
        }
    }

    /// One fake sink: the tracks queued on it and the calls made on it.
    pub(super) struct FakeSink {
        queue: Arc<AtomicUsize>,
        audio: Arc<Mutex<Audio>>,
    }

    impl FakeSink {
        fn record(&self, call: Call) {
            self.audio.lock().unwrap().calls.push(call);
        }
    }

    impl Sink for FakeSink {
        fn append(&self, source: Decoded) {
            // Nothing plays through the fake: a queued track is dropped, as a
            // played-out one is.
            drop(source);
            self.queue.fetch_add(1, Ordering::SeqCst);
            self.record(Call::Append);
        }
        fn stop(&self) {
            self.queue.store(0, Ordering::SeqCst);
            self.record(Call::Stop);
        }
        fn play(&self) {
            self.record(Call::Play);
        }
        fn pause(&self) {
            self.record(Call::Pause);
        }
        fn set_volume(&self, volume: f32) {
            self.record(Call::Volume(volume));
        }
        fn request_seek(&self, to: Duration, _queued: usize) -> Result<Ticket, String> {
            let (refuse, drain, gate) = {
                let mut audio = self.audio.lock().unwrap();
                audio.calls.push(Call::Seek(to));
                (
                    audio.refuse_seek,
                    audio.drain_on_seek,
                    audio.seek_gate.clone(),
                )
            };
            // A blocked device is a blocked seek: the audio thread is inside
            // this call, exactly where rodio waits for its feedback.
            let ticket = Ticket::new(to);
            if let Some(gate) = gate {
                gate.submit(ticket.clone());
                return Ok(ticket);
            }
            if drain > 0 {
                // The track reached its end while the seek was in flight.
                self.queue.fetch_sub(drain, Ordering::SeqCst);
                self.audio.lock().unwrap().drain_on_seek = 0;
            }
            ticket.finish(if refuse {
                Err("the test decoder cannot seek".into())
            } else {
                Ok(())
            });
            Ok(ticket)
        }
        fn len(&self) -> usize {
            self.queue.load(Ordering::SeqCst)
        }
    }

    /// A player thread on a fake output, plus the handles to observe it: how
    /// often a device was asked for, what the output was asked to do, and when
    /// the thread ended.
    pub(super) struct TestPlayer {
        pub(super) tx: mailbox::Sender,
        pub(super) status: Arc<Mutex<PlayerStatus>>,
        pub(super) output: FakeOutput,
        pub(super) attempts: Arc<AtomicUsize>,
        done: std::sync::mpsc::Receiver<()>,
        handle: std::thread::JoinHandle<()>,
    }

    impl TestPlayer {
        fn start(
            output: FakeOutput,
            watching: bool,
            mut open: impl FnMut() -> Result<FakeOutput, DeviceSinkError> + Send + 'static,
        ) -> Self {
            let (tx, rx) = mailbox::channel();
            let (done_tx, done) = std::sync::mpsc::channel();
            let status = Arc::new(Mutex::new(PlayerStatus::default()));
            let attempts = Arc::new(AtomicUsize::new(0));
            let status_for_thread = Arc::clone(&status);
            let attempts_for_thread = Arc::clone(&attempts);
            let handle = std::thread::spawn(move || {
                if watching {
                    allocations::watch_current_thread();
                }
                audio_thread(
                    rx,
                    status_for_thread,
                    studio::Shared::default(),
                    move || {
                        attempts_for_thread.fetch_add(1, Ordering::SeqCst);
                        open()
                    },
                );
                let _ = done_tx.send(());
            });
            Self {
                tx,
                status,
                output,
                attempts,
                done,
                handle,
            }
        }

        /// A thread whose device opener always fails.
        pub(super) fn without_a_device() -> Self {
            Self::start(FakeOutput::new(), false, || Err(DeviceSinkError::NoDevice))
        }

        /// A thread whose device is always there.
        pub(super) fn with_a_device() -> Self {
            let output = FakeOutput::new();
            let for_thread = output.clone();
            Self::start(output, false, move || Ok(for_thread.clone()))
        }

        /// Like [`TestPlayer::with_a_device`], with this thread's heap
        /// allocations counted from [`allocations`].
        pub(super) fn with_a_counted_device() -> Self {
            let output = FakeOutput::new();
            let for_thread = output.clone();
            Self::start(output, true, move || Ok(for_thread.clone()))
        }

        pub(super) fn send(&self, cmd: Cmd) {
            self.tx.send(cmd).expect("the audio thread is gone");
        }

        pub(super) fn status(&self) -> PlayerStatus {
            self.status.lock().unwrap().clone()
        }

        pub(super) fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }

        /// Wait for `cond` with a deadline, so a regression fails the test
        /// instead of hanging it.
        pub(super) fn wait_until(&self, what: &str, mut cond: impl FnMut(&Self) -> bool) {
            wait_until(what, || cond(self));
        }

        /// Dropping the command channel must end the thread; every test ends
        /// this way, so a thread that will not stop fails loudly.
        ///
        /// The idle snapshot is the same before and after every command here, so
        /// tests read the status once an observable call proves the command was
        /// processed, rather than racing a status rewrite.
        pub(super) fn finish(self, what: &str) {
            let Self {
                tx, done, handle, ..
            } = self;
            drop(tx);
            done.recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("the audio thread outlived its channel ({what})"));
            handle.join().unwrap();
        }
    }

    /// Whether the player reports `file` as the track it is playing.
    pub(super) fn playing(player: &TestPlayer, file: &std::path::Path) -> bool {
        player.status().path.as_deref().map(std::path::Path::new) == Some(file)
    }

    /// Wait for `cond` with a deadline, so a regression fails the test instead
    /// of hanging it.
    pub(super) fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if cond() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("timed out waiting for {what}");
    }

    /// A small, valid AIFF of `seconds` seconds, so a regression that pretends
    /// to play is caught by the status and not only by a missing decoder.
    pub(super) fn playable_file(name: &str, seconds: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tagrex-player-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.aiff");
        std::fs::write(&path, aiff::tests::aiff(44_100 * seconds.max(1), 2280)).unwrap();
        path
    }

    pub(super) fn clean_up(path: &std::path::Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Counts the heap allocations of threads that opt in, so a test can say
    /// "this did not allocate" as a measurement rather than a reading of the
    /// source.
    ///
    /// Only the player thread under test opts in, and the counter is global
    /// while the flag is per thread, so a parallel test's audio thread is not
    /// counted.
    pub(super) mod allocations {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub(crate) static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

        thread_local! {
            /// Read from inside the allocator, so it must not allocate: a
            /// `const`-initialised `Cell` needs no lazy initialisation and holds
            /// nothing that needs dropping.
            static WATCHING: Cell<bool> = const { Cell::new(false) };
        }

        pub(super) fn watch_current_thread() {
            WATCHING.with(|watching| watching.set(true));
        }

        fn watching() -> bool {
            // `try_with`: an allocation while this thread is being torn down
            // must not panic the allocator.
            WATCHING.try_with(Cell::get).unwrap_or(false)
        }

        struct Counting;

        // Safety: every method forwards to `System` with the same arguments and
        // returns its pointer unchanged; the only addition is a counter, which
        // does not allocate, free or touch the memory being allocated.
        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                if watching() {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                }
                unsafe { System.alloc(layout) }
            }

            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                if watching() {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                }
                unsafe { System.alloc_zeroed(layout) }
            }

            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
                if watching() {
                    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                }
                unsafe { System.realloc(ptr, layout, size) }
            }

            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) }
            }
        }

        #[global_allocator]
        static ALLOCATOR: Counting = Counting;
    }
}

/// # D1: the player survives a machine with no output device.
///
/// These tests inject the opener, so they are deterministic both on a machine
/// with a working sound card (the real opener would find it and prove nothing
/// about the no-device path) and on a headless one. They cover thread survival,
/// retries and the status; playback after a device appears needs a real device
/// and is not claimed here.
#[cfg(test)]
mod without_an_output_device {
    use super::test_support::{clean_up, playable_file, TestPlayer};
    use super::*;

    #[test]
    fn startup_without_a_device_keeps_the_thread_and_says_so() {
        let player = TestPlayer::without_a_device();
        player.wait_until("the startup open attempt", |p| p.attempts() >= 1);
        player.wait_until("output_unavailable after the failed open", |p| {
            p.status().output_unavailable
        });

        let status = player.status();
        assert!(status.output_unavailable);
        assert_eq!(status.path, None, "an idle player reported a track");
        assert_eq!(status.position_secs, 0.0);
        assert!(!status.is_paused);
        assert!(!status.wants_next);
        assert!(!status.seek_refused);

        player.finish("idle after a failed startup");
    }

    #[test]
    fn play_retries_the_device_and_a_second_failure_is_harmless() {
        let track = playable_file("retry", 1);
        let player = TestPlayer::without_a_device();
        player.wait_until("the startup open attempt", |p| p.attempts() >= 1);

        player.send(Cmd::Play(track.clone()));
        player.wait_until("the device retry from play", |p| p.attempts() >= 2);
        // A second play is another retry, not a dead thread.
        player.send(Cmd::Play(track.clone()));
        player.wait_until("the second device retry", |p| p.attempts() >= 3);

        let status = player.status();
        assert!(status.output_unavailable);
        assert_eq!(status.path, None, "a track played without a device");
        assert_eq!(status.position_secs, 0.0);

        player.finish("after failed plays");
        clean_up(&track);
    }

    #[test]
    fn control_commands_without_a_device_are_harmless() {
        let player = TestPlayer::without_a_device();
        player.wait_until("the startup open attempt", |p| p.attempts() >= 1);

        // One consumer on a FIFO channel, so the final play retry proves every
        // command before it was processed — and that none of them panicked.
        player.send(Cmd::Pause);
        player.send(Cmd::Resume);
        player.send(Cmd::Seek(12.5));
        player.send(Cmd::SetVolume(0.25));
        player.send(Cmd::Stop);
        player.send(Cmd::SetNext(PathBuf::from("/nonexistent/next.aiff")));
        player.send(Cmd::Play(PathBuf::from("/nonexistent/track.aiff")));
        player.wait_until("the play retry after the control commands", |p| {
            p.attempts() >= 2
        });

        let status = player.status();
        assert!(status.output_unavailable);
        assert_eq!(status.path, None, "a control command played a track");
        assert!(!status.is_paused);
        assert_eq!(status.position_secs, 0.0);

        player.finish("after device-free control commands");
    }
}

/// What the device and the UI are shown once there *is* a device.
///
/// The fake sink makes every path after a device is opened testable without a
/// sound card: gapless priming and the track boundary, pause and seek
/// accounting, the volume a fresh sink starts at, and the order accepted
/// transport controls reach the output in. None of it depends on audio hardware,
/// and every wait has a deadline.
#[cfg(test)]
mod with_an_output_device {
    use super::test_support::{clean_up, playable_file, playing, Call, TestPlayer};
    use super::*;

    #[test]
    fn a_boundary_before_seek_does_not_seek_the_new_track_as_the_old_one() {
        let path = playable_file("seek-before-boundary", 4);
        let output = super::test_support::FakeOutput::new();
        let sink = output.sink(1.0);
        sink.append(open_decoder(&path).unwrap());
        // The player still tracks two sources, but the output already finished
        // the old one. A seek must not be sent to the remaining next source.
        assert!(matches!(
            seek(Some(&sink), Duration::from_secs(3), 2),
            SeekOutcome::PlayedOut
        ));
        assert!(!output
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Seek(_))));
        clean_up(&path);
    }

    fn appended(player: &TestPlayer) -> usize {
        player
            .output
            .calls()
            .into_iter()
            .filter(|call| *call == Call::Append)
            .count()
    }

    #[test]
    fn a_primed_track_crosses_the_boundary_gaplessly_and_primes_the_one_after() {
        let first = playable_file("gapless-first", 4);
        let second = playable_file("gapless-second", 4);
        let third = playable_file("gapless-third", 4);
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        player.send(Cmd::Play(first.clone()));
        player.wait_until("the first track", |p| playing(p, &first));
        assert_eq!(player.output.queued(), 1);
        // Four seconds: the last five seconds of it are the whole track, so the
        // next one is wanted at once rather than never.
        assert!(
            player.status().wants_next,
            "the next track was never asked for"
        );
        assert_eq!(player.status().duration_secs, 4.0);

        player.send(Cmd::SetNext(second.clone()));
        player.wait_until("the primed track", |p| p.output.queued() == 2);
        // Priming appended the next source while the current one plays: that is
        // the gapless guarantee, and it must not move the reported track.
        assert!(playing(&player, &first));
        assert!(!player.status().wants_next);

        // The device reaches the end of the first track. The next one is already
        // in the sink, so playback continues on it from its start.
        player.output.finish_front();
        player.wait_until("the boundary", |p| playing(p, &second));
        assert_eq!(player.output.queued(), 1);
        assert!(
            player.status().position_secs < 1.0,
            "the new track's clock started at {}",
            player.status().position_secs
        );
        assert!(
            player.status().wants_next,
            "the queue stopped asking for a next track"
        );

        player.send(Cmd::SetNext(third.clone()));
        player.wait_until("the second primed track", |p| p.output.queued() == 2);
        assert_eq!(appended(&player), 3);

        player.finish("after a gapless boundary");
        for file in [&first, &second, &third] {
            clean_up(file);
        }
    }

    #[test]
    fn a_track_that_plays_out_leaves_the_player_idle() {
        let track = playable_file("played-out", 1);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));

        player.output.finish_front();
        player.wait_until("the end of the list", |p| p.status().path.is_none());
        let status = player.status();
        assert_eq!(status.position_secs, 0.0);
        assert!(!status.is_paused);
        assert!(!status.wants_next);
        assert!(
            !status.output_unavailable,
            "a present device was reported missing"
        );

        player.finish("after the queue played out");
        clean_up(&track);
    }

    #[test]
    fn pause_holds_the_clock_and_resume_lets_it_run_again() {
        let track = playable_file("pause", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| {
            playing(p, &track) && p.status().position_secs > 0.05
        });

        player.send(Cmd::Pause);
        player.wait_until("the pause", |p| p.status().is_paused);
        let paused_at = player.status().position_secs;
        std::thread::sleep(Duration::from_millis(300));
        let still = player.status();
        // A paused clock is frozen, not merely slow: the bar must not creep.
        assert!(still.is_paused);
        assert_eq!(still.position_secs, paused_at);
        assert_eq!(
            player
                .output
                .calls()
                .into_iter()
                .filter(|call| *call == Call::Pause)
                .count(),
            1
        );

        player.send(Cmd::Resume);
        player.wait_until("the clock to move again", |p| {
            p.status().position_secs > paused_at
        });
        assert!(!player.status().is_paused);

        player.finish("after pause and resume");
        clean_up(&track);
    }

    // The paused snapshot is identical five times a second, so the tick that
    // reports it must not build the path string or touch the status mutex the UI
    // polls. Measured on the player thread with a counting allocator, not read
    // from the source.
    #[test]
    fn a_paused_player_allocates_nothing_while_the_snapshot_is_unchanged() {
        use std::sync::atomic::Ordering;

        use super::test_support::allocations;

        let track = playable_file("paused-quiet", 4);
        let player = TestPlayer::with_a_counted_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));
        player.send(Cmd::Pause);
        player.wait_until("the pause", |p| p.status().is_paused);

        // Let the pause itself and the snapshot it publishes settle.
        std::thread::sleep(Duration::from_millis(300));
        let before = allocations::ALLOCATIONS.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(1));
        let after = allocations::ALLOCATIONS.load(Ordering::Relaxed);

        // The queue is not empty, so the thread is on its 200 ms tick during
        // this second: about five unchanged snapshots were due, and every one of
        // them compared equal.
        let paused = player.status();
        assert!(
            paused.is_paused,
            "the pause was lost during the quiet second"
        );
        assert_eq!(paused.path.as_deref(), track.to_str());
        assert_eq!(
            after,
            before,
            "a paused player allocated {} times while nothing changed",
            after - before
        );

        // Positive control: the same counter does move when the snapshot really
        // changes, so the zero above is not a counter that never runs.
        player.send(Cmd::Resume);
        player.wait_until("the clock to run again", |p| {
            !p.status().is_paused && p.status().position_secs > 0.05
        });
        assert!(
            allocations::ALLOCATIONS.load(Ordering::Relaxed) > after,
            "the allocation counter never moved: the measurement proves nothing"
        );

        player.finish("after a quiet second");
        clean_up(&track);
    }

    #[test]
    fn a_refused_seek_is_reported_and_does_not_move_the_clock() {
        let track = playable_file("refused-seek", 4);
        let player = TestPlayer::with_a_device();
        player.output.refuse_seeks();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));

        let before = player.status().position_secs;
        player.send(Cmd::Seek(2.0));
        player.wait_until("the refusal", |p| p.status().seek_refused);
        let status = player.status();
        // The decoder refused, so the audio is still where it was; reporting the
        // seek target would show a position nothing is playing from (#190).
        assert!(
            status.position_secs < before + 1.0,
            "the clock jumped to {} from {before} without the audio",
            status.position_secs
        );
        assert!(player
            .output
            .calls()
            .contains(&Call::Seek(Duration::from_secs(2))));

        player.finish("after a refused seek");
        clean_up(&track);
    }

    #[test]
    fn a_seek_past_the_end_stays_inside_the_track() {
        let track = playable_file("seek-past-end", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));

        player.send(Cmd::Seek(600.0));
        player.wait_until("the position at the end of the track", |p| {
            p.status().position_secs >= 4.0
        });
        let status = player.status();
        // The wall clock would overshoot the track for ever; the bar shows the
        // track's length instead, and the track is still reported as playing.
        assert_eq!(status.position_secs, status.duration_secs);
        assert_eq!(status.duration_secs, 4.0);
        assert!(playing(&player, &track));

        player.finish("after a seek past the end");
        clean_up(&track);
    }

    // #190, the played-out interval: the track can reach its end between the
    // caller's "there is still a track" check and the sink's seek. rodio then
    // answers `Ok(())` without seeking anything, and the clock may not follow it.
    #[test]
    fn a_seek_into_track_that_played_out_does_not_move_the_clock() {
        let track = playable_file("seek-played-out", 1);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));

        // The sink reaches the end of the only track while the seek is in
        // flight, so the seek has nothing to seek and no audio follows it.
        player.output.finish_a_track_during_seek();
        player.send(Cmd::Seek(0.5));
        player.wait_until("the player to go idle at the boundary", |p| {
            p.status().path.is_none()
        });
        let status = player.status();
        assert_eq!(status.position_secs, 0.0);
        assert!(!status.is_paused);
        assert!(
            !status.seek_refused,
            "the track simply ended; nothing refused the seek"
        );

        player.finish("after a seek into a track that played out");
        clean_up(&track);
    }

    #[test]
    fn a_boundary_while_paused_keeps_the_next_tracks_clock_frozen() {
        let first = playable_file("paused-boundary-first", 4);
        let second = playable_file("paused-boundary-second", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(first.clone()));
        player.send(Cmd::SetNext(second.clone()));
        player.send(Cmd::Pause);
        player.wait_until("both sources paused", |p| {
            p.status().is_paused && p.output.queued() == 2
        });
        player.output.finish_front();
        player.wait_until("the next source after a paused boundary", |p| {
            playing(p, &second)
        });
        let before = player.status().position_secs;
        std::thread::sleep(Duration::from_millis(300));
        assert!(player.status().is_paused);
        assert_eq!(player.status().position_secs, before);
        player.finish("paused at a boundary");
        clean_up(&first);
        clean_up(&second);
    }

    // A source-local seek cannot affect the next track, even if the boundary
    // occurs during submission. Keep the primed source rather than rebuilding it.
    #[test]
    fn a_seek_across_a_boundary_leaves_the_primed_next_track_untouched() {
        let first = playable_file("seek-race-first", 4);
        let second = playable_file("seek-race-second", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(first.clone()));
        player.send(Cmd::SetNext(second.clone()));
        player.wait_until("both tracks queued", |p| p.output.queued() == 2);

        let appends = appended(&player);
        let sinks = |p: &TestPlayer| -> usize {
            p.output
                .calls()
                .into_iter()
                .filter(|call| matches!(call, Call::NewSink(_)))
                .count()
        };
        let sinks_before = sinks(&player);

        player.output.finish_a_track_during_seek();
        player.send(Cmd::Seek(3.0));
        player.wait_until("the next track", |p| playing(p, &second));

        // No extra source or sink was made. The gapless primed next track is
        // retained and starts from its own position, with no old seek applied.
        assert_eq!(
            appended(&player),
            appends,
            "a source-local seek unnecessarily rebuilt the primed next track"
        );
        assert_eq!(sinks(&player), sinks_before);
        assert_eq!(player.output.queued(), 1);
        let status = player.status();
        assert!(
            status.position_secs < 1.0,
            "the next track started at {}",
            status.position_secs
        );
        assert!(!status.seek_refused);

        player.finish("after a seek that crossed a boundary");
        clean_up(&first);
        clean_up(&second);
    }

    // A device that never pulls must not stall the transport actor. Releasing
    // an old acknowledgement after Stop/Play cannot seek the replacement track.
    #[test]
    fn an_unanswered_seek_does_not_hold_up_stop_or_replay() {
        let track = playable_file("blocked-seek", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));
        let gate = player.output.hold_seeks();
        player.send(Cmd::Seek(2.0));
        player.wait_until("the seek to become pending", |p| p.status().seek_pending);
        let started = Instant::now();
        player.send(Cmd::Stop);
        player.wait_until("Stop despite unanswered seek", |p| {
            p.status().path.is_none()
        });
        assert!(started.elapsed() < Duration::from_secs(1));
        player.send(Cmd::Play(track.clone()));
        player.wait_until("replay despite unanswered seek", |p| playing(p, &track));
        gate.release();
        std::thread::sleep(Duration::from_millis(250));
        assert!(player.status().position_secs < 1.0);
        assert!(!player.status().seek_pending);
        player.finish("after a nonblocking seek");
        clean_up(&track);
    }

    #[test]
    fn an_unanswered_seek_times_out_without_claiming_success() {
        let track = playable_file("seek-deadline", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(track.clone()));
        player.wait_until("playback to start", |p| playing(p, &track));
        let gate = player.output.hold_seeks();
        player.send(Cmd::Seek(3.5));
        player.wait_until("the seek deadline", |p| p.status().seek_refused);
        assert!(!player.status().seek_pending);
        gate.release();
        std::thread::sleep(Duration::from_millis(100));
        assert!(player.status().position_secs < 3.5);
        player.finish("after the seek deadline");
        clean_up(&track);
    }

    #[test]
    fn an_accepted_stop_clears_the_queue_the_clock_and_the_refusal() {
        let track = playable_file("stop", 4);
        let next = playable_file("stop-next", 4);
        let player = TestPlayer::with_a_device();
        player.output.refuse_seeks();
        player.send(Cmd::Play(track.clone()));
        player.send(Cmd::SetNext(next.clone()));
        player.wait_until("two queued tracks", |p| p.output.queued() == 2);
        player.send(Cmd::Seek(2.0));
        player.wait_until("the refusal", |p| p.status().seek_refused);

        player.send(Cmd::Stop);
        player.wait_until("the stop", |p| p.status().path.is_none());
        let status = player.status();
        assert_eq!(status.position_secs, 0.0);
        assert!(!status.seek_refused);
        assert!(!status.is_paused);
        assert!(!status.wants_next);
        // Stop starts a fresh, empty sink: it is not a pause with a queue
        // behind it.
        assert_eq!(player.output.queued(), 0);
        assert_eq!(
            player
                .output
                .calls()
                .into_iter()
                .filter(|call| *call == Call::Stop)
                .count(),
            2,
            "the old sink and the stop were not silenced"
        );

        player.finish("after stop");
        clean_up(&track);
        clean_up(&next);
    }

    #[test]
    fn a_new_play_replaces_the_session_instead_of_adding_to_it() {
        let first = playable_file("replace-first", 4);
        let second = playable_file("replace-second", 4);
        let player = TestPlayer::with_a_device();
        player.send(Cmd::Play(first.clone()));
        player.send(Cmd::SetNext(second.clone()));
        player.wait_until("both tracks queued", |p| p.output.queued() == 2);

        player.send(Cmd::Play(first.clone()));
        player.wait_until("the restart", |p| {
            p.output.queued() == 1 && playing(p, &first)
        });
        assert!(
            player.status().position_secs < 1.0,
            "the replaced session kept the old clock at {}",
            player.status().position_secs
        );
        // One append for the first play, one for the primed track, one for the
        // replacement — the primed track did not survive the new session.
        assert_eq!(appended(&player), 3);

        player.finish("after replacing the session");
        clean_up(&first);
        clean_up(&second);
    }

    #[test]
    fn the_volume_is_reapplied_to_every_new_sink() {
        let track = playable_file("volume", 4);
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        let sinks = |p: &TestPlayer| -> Vec<Call> {
            p.output
                .calls()
                .into_iter()
                .filter(|call| matches!(call, Call::NewSink(_)))
                .collect()
        };
        player.send(Cmd::SetVolume(0.25));
        player.send(Cmd::Play(track.clone()));
        player.wait_until("a sink at the level that was set", |p| {
            sinks(p) == vec![Call::NewSink(1.0), Call::NewSink(0.25)]
        });

        // A fresh sink is always full volume, so Stop's new one must be levelled
        // too, or turning the volume down would silently reset (#D1).
        player.send(Cmd::Stop);
        player.wait_until("a levelled sink after the stop", |p| {
            sinks(p) == vec![Call::NewSink(1.0), Call::NewSink(0.25), Call::NewSink(0.25)]
        });
        assert!(player.output.calls().contains(&Call::Volume(0.25)));

        player.finish("after the volume survived two sinks");
        clean_up(&track);
    }

    #[test]
    fn control_commands_with_nothing_playing_are_ignored() {
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        // One consumer on a FIFO channel, so the volume proves every command
        // before it was processed and none of them panicked.
        player.send(Cmd::Pause);
        player.send(Cmd::Resume);
        player.send(Cmd::Seek(5.0));
        player.send(Cmd::SetNext(PathBuf::from("/nonexistent/next.aiff")));
        player.send(Cmd::Stop);
        player.send(Cmd::SetVolume(0.5));
        player.wait_until("the volume after them", |p| {
            p.output.calls().contains(&Call::Volume(0.5))
        });

        let calls = player.output.calls();
        assert!(!calls.contains(&Call::Pause));
        assert!(!calls.contains(&Call::Seek(Duration::from_secs(5))));
        assert_eq!(appended(&player), 0);
        let status = player.status();
        assert_eq!(status.path, None);
        assert_eq!(status.position_secs, 0.0);
        assert!(!status.is_paused);
        assert!(!status.seek_refused);

        player.finish("after controls with nothing playing");
    }

    #[test]
    fn a_track_that_cannot_be_decoded_leaves_the_player_idle_and_then_plays() {
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        player.send(Cmd::Play(PathBuf::from("/nonexistent/track.aiff")));
        let track = playable_file("after-a-broken-track", 4);
        player.send(Cmd::Play(track.clone()));
        player.wait_until("the track that does decode", |p| playing(p, &track));
        // The undecodable one was never queued, so it cannot have been counted.
        assert_eq!(appended(&player), 1);
        assert_eq!(player.output.queued(), 1);

        player.finish("after an undecodable track");
        clean_up(&track);
    }

    #[test]
    fn a_burst_of_transport_controls_reaches_the_output_in_order() {
        let a = playable_file("burst-a", 4);
        let b = playable_file("burst-b", 4);
        let c = playable_file("burst-c", 4);
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        // Accepted controls keep their order across track changes: the volume and
        // the seek belong to `a`, and neither may land on `b` or `c`. Every Play
        // and Stop replaces the sink, and each fresh one starts at the level set
        // before it.
        player.send(Cmd::Play(a.clone()));
        player.send(Cmd::SetVolume(0.3));
        player.send(Cmd::Seek(1.0));
        player.send(Cmd::Play(b.clone()));
        player.send(Cmd::Stop);
        player.send(Cmd::Play(c.clone()));
        player.wait_until("the last play", |p| playing(p, &c));

        assert_eq!(
            player.output.calls(),
            vec![
                Call::NewSink(1.0),
                Call::Stop,
                Call::NewSink(1.0),
                Call::Append,
                Call::Volume(0.3),
                Call::Seek(Duration::from_secs(1)),
                Call::Stop,
                Call::NewSink(0.3),
                Call::Append,
                Call::Stop,
                Call::NewSink(0.3),
                Call::Stop,
                Call::NewSink(0.3),
                Call::Append,
            ]
        );
        assert_eq!(player.output.queued(), 1);

        player.finish("after a burst of controls");
        for file in [&a, &b, &c] {
            clean_up(file);
        }
    }

    #[test]
    fn an_idle_player_answers_a_command_immediately() {
        // Nothing is queued, so the thread has no work to tick for; a command
        // must still be picked up at once rather than at the end of a wait.
        let track = playable_file("idle-player", 1);
        let player = TestPlayer::with_a_device();
        player.wait_until("the startup sink", |p| p.output.calls().len() == 1);

        player.send(Cmd::Play(track.clone()));
        player.wait_until("an idle player to answer", |p| playing(p, &track));

        player.finish("after an idle wait");
        clean_up(&track);
    }
}

#[cfg(test)]
mod real_sink_without_hardware {
    use super::*;
    use test_support::{clean_up, playable_file, wait_until};
    struct OutputHarness(router::Handle);
    impl Output for OutputHarness {
        type Sink = DevicePlayer;
        fn sink(&self, volume: f32) -> DevicePlayer {
            DevicePlayer::new(&self.0, volume)
        }
    }
    fn harness() -> (
        mailbox::Sender,
        Arc<Mutex<PlayerStatus>>,
        router::Router,
        std::thread::JoinHandle<()>,
    ) {
        let (handle, router) = router::Router::new(
            rodio::ChannelCount::new(2).unwrap(),
            rodio::SampleRate::new(44_100).unwrap(),
        );
        let (tx, rx) = mailbox::channel();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let shared = status.clone();
        let actor = std::thread::spawn(move || {
            audio_thread(rx, shared, studio::Shared::default(), || {
                Ok(OutputHarness(handle.clone()))
            })
        });
        (tx, status, router, actor)
    }
    #[test]
    fn real_rodio_queue_without_device_consumption_survives_seek_volume_bursts_stop_and_replay() {
        let track = playable_file("real-sink-stall", 4);
        let (tx, status, _router, actor) = harness();
        tx.send(Cmd::Play(track.clone())).unwrap();
        wait_until("real sink to queue the track", || {
            status.lock().unwrap().path.is_some()
        });
        for i in 0..100_000 {
            tx.send(Cmd::Seek((i % 3) as f64)).unwrap();
            tx.send(Cmd::SetVolume((i % 100) as f32 / 100.0)).unwrap();
        }
        wait_until("pending source-bound acknowledgement", || {
            status.lock().unwrap().seek_pending
        });
        let stopped = Instant::now();
        tx.send(Cmd::Stop).unwrap();
        wait_until("Stop with no device samples consumed", || {
            status.lock().unwrap().path.is_none()
        });
        assert!(stopped.elapsed() < Duration::from_secs(1));
        tx.send(Cmd::Play(track.clone())).unwrap();
        wait_until("replay with no old seek inherited", || {
            status.lock().unwrap().path.is_some()
        });
        assert!(!status.lock().unwrap().seek_pending);
        drop(tx);
        actor.join().unwrap();
        clean_up(&track);
    }
    #[test]
    fn real_rodio_queue_acknowledges_seek_while_paused_after_output_consumption_resumes() {
        let track = playable_file("real-sink-paused-seek", 4);
        let (tx, status, mut router, actor) = harness();
        tx.send(Cmd::Play(track.clone())).unwrap();
        tx.send(Cmd::Pause).unwrap();
        wait_until("paused playback", || status.lock().unwrap().is_paused);
        tx.send(Cmd::Seek(2.0)).unwrap();
        wait_until("pending seek without device consumption", || {
            status.lock().unwrap().seek_pending
        });
        assert!(status.lock().unwrap().position_secs < 0.01);
        // Actual Rodio queue, format conversion and actual AIFF decoder. The
        // consumer was absent; it resumes now while playback remains paused.
        for _ in 0..8_192 {
            router.next();
        }
        wait_until("actual decoder acknowledgement while paused", || {
            !status.lock().unwrap().seek_pending
        });
        {
            let value = status.lock().unwrap();
            assert!(value.is_paused);
            assert_eq!(value.position_secs, 2.0);
            assert!(!value.seek_refused);
        }
        tx.send(Cmd::Stop).unwrap();
        wait_until("Stop after a paused seek", || {
            status.lock().unwrap().path.is_none()
        });
        drop(tx);
        actor.join().unwrap();
        clean_up(&track);
    }
    #[test]
    #[ignore = "manual bounded decoding/control throughput measurement"]
    fn decoded_source_control_throughput_probe() {
        fn consume(source: impl Iterator<Item = f32>) -> (u64, usize) {
            let mut hash = 0xcbf29ce484222325u64;
            let mut count = 0;
            for value in source {
                hash = (hash ^ u64::from(std::hint::black_box(value).to_bits()))
                    .wrapping_mul(0x100000001b3);
                count += 1;
            }
            (hash, count)
        }
        let track = playable_file("control-throughput", 12);
        let mut raw = Vec::new();
        let mut controlled = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let expected = consume(rodio::source::UniformSourceIterator::new(
                open_decoder(&track).unwrap(),
                rodio::ChannelCount::new(2).unwrap(),
                rodio::SampleRate::new(48_000).unwrap(),
            ));
            raw.push(start.elapsed().as_secs_f64() * 1_000.0);
            let start = Instant::now();
            let fixed = fixed::Fixed::new(
                open_decoder(&track).unwrap(),
                rodio::ChannelCount::new(2).unwrap(),
                rodio::SampleRate::new(48_000).unwrap(),
            );
            let (source, _control) = Controlled::new_fixed(fixed, Arc::new(AtomicBool::new(false)));
            let actual = consume(source);
            controlled.push(start.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(actual, expected, "source controls changed decoded samples");
            assert!(actual.1 >= 44_100 * 12);
        }
        raw.sort_by(f64::total_cmp);
        controlled.sort_by(f64::total_cmp);
        println!(
            "12s synthetic AIFF, seven warm samples, decode + 48kHz stereo conversion + sample checksum; real converter median {:.3}ms max {:.3}ms; controlled median {:.3}ms max {:.3}ms",
            raw[3], raw[6], controlled[3], controlled[6]
        );
        clean_up(&track);
    }
    #[test]
    #[ignore = "independent generated MP3/FLAC/M4A/AIFF decoder seek probe requires ffmpeg"]
    fn actual_decoders_acknowledge_seeks_after_converter_metadata_and_midstream_reads() {
        use rodio::Source;
        let root =
            std::env::temp_dir().join(format!("tagrex-midstream-seek-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for (extension, codec) in [
            ("mp3", "libmp3lame"),
            ("flac", "flac"),
            ("m4a", "aac"),
            ("aiff", "pcm_s16be"),
        ] {
            let path = root.join(format!("track.{extension}"));
            let result = std::process::Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100:duration=4",
                    "-c:a",
                    codec,
                ])
                .arg(&path)
                .status()
                .unwrap();
            assert!(result.success());
            let pause = Arc::new(AtomicBool::new(false));
            let decoded = open_decoder(&path).unwrap();
            let first_span = decoded.current_span_len();
            let fixed = fixed::Fixed::new(
                decoded,
                rodio::ChannelCount::new(2).unwrap(),
                rodio::SampleRate::new(48_000).unwrap(),
            );
            let (source, control) = Controlled::new_fixed(fixed, pause.clone());
            let mut converted = rodio::source::UniformSourceIterator::new(
                source,
                rodio::ChannelCount::new(2).unwrap(),
                rodio::SampleRate::new(48_000).unwrap(),
            );
            for _ in 0..138 {
                assert!(converted.next().is_some());
            }
            let ticket = control.seek(Duration::from_secs(2));
            let mut consumed = 0;
            while ticket.poll().is_none() && consumed < 96_000 {
                assert!(
                    converted.next().is_some(),
                    "{extension} ended before seek acknowledgement"
                );
                consumed += 1;
            }
            assert!(
                matches!(ticket.poll(), Some(Ok(_))),
                "{extension}: span {first_span:?}, consumed {consumed}, result {:?}",
                ticket.poll()
            );
            pause.store(true, Ordering::Release);
            let paused_ticket = control.seek(Duration::from_secs(1));
            let mut paused_consumed = 0;
            while paused_ticket.poll().is_none() && paused_consumed < 96_000 {
                assert!(converted.next().is_some());
                paused_consumed += 1;
            }
            assert!(
                matches!(paused_ticket.poll(), Some(Ok(_))),
                "{extension} paused seek failed: {:?}",
                paused_ticket.poll()
            );
            println!(
                "{extension}: initial native span {first_span:?}; midstream ack after {consumed} output samples; paused ack after {paused_consumed}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod actor_seek_ownership {
    use super::*;
    use std::sync::{atomic::AtomicUsize, mpsc};
    use test_support::{clean_up, playable_file, wait_until};
    struct Blocking {
        position: usize,
        gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }
    impl Iterator for Blocking {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            let sample = self.position as f32;
            self.position += 1;
            Some(sample)
        }
    }
    impl rodio::Source for Blocking {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(2).unwrap()
        }
        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(8000).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
        fn try_seek(&mut self, target: Duration) -> Result<(), rodio::source::SeekError> {
            if let Some((entered, release)) = self.gate.take() {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.position = target.as_secs() as usize * 16_000;
            Ok(())
        }
    }
    type Source = Controlled<fixed::Fixed<Blocking>>;
    struct Shared {
        gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        source: Mutex<Option<Source>>,
        control: Mutex<Option<Control>>,
        requested: Mutex<Vec<Duration>>,
        queued: AtomicUsize,
        generations: AtomicUsize,
        paused: Arc<AtomicBool>,
    }
    #[derive(Clone)]
    struct Harness(Arc<Shared>);
    impl Output for Harness {
        type Sink = Harness;
        fn sink(&self, _: f32) -> Harness {
            self.clone()
        }
    }
    impl Sink for Harness {
        fn append(&self, source: Decoded) {
            drop(source);
            let fixed = fixed::Fixed::new(
                Blocking {
                    position: 0,
                    gate: self.0.gate.lock().unwrap().take(),
                },
                rodio::ChannelCount::new(2).unwrap(),
                rodio::SampleRate::new(8000).unwrap(),
            );
            let (source, control) = Controlled::new_fixed(fixed, self.0.paused.clone());
            *self.0.source.lock().unwrap() = Some(source);
            *self.0.control.lock().unwrap() = Some(control);
            self.0.generations.fetch_add(1, Ordering::SeqCst);
            self.0.queued.store(1, Ordering::SeqCst);
        }
        fn stop(&self) {
            self.0.queued.store(0, Ordering::SeqCst);
        }
        fn play(&self) {
            self.0.paused.store(false, Ordering::SeqCst);
        }
        fn pause(&self) {
            self.0.paused.store(true, Ordering::SeqCst);
        }
        fn set_volume(&self, _: f32) {}
        fn request_seek(&self, target: Duration, _: usize) -> Result<Ticket, String> {
            let ticket = self
                .0
                .control
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .seek(target);
            self.0.requested.lock().unwrap().push(target);
            Ok(ticket)
        }
        fn len(&self) -> usize {
            self.0.queued.load(Ordering::SeqCst)
        }
    }
    struct Fixture {
        output: Harness,
        tx: mailbox::Sender,
        status: Arc<Mutex<PlayerStatus>>,
        actor: std::thread::JoinHandle<()>,
        entered: mpsc::Receiver<()>,
        release: mpsc::Sender<()>,
        track: PathBuf,
    }
    fn fixture(name: &str) -> Fixture {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let output = Harness(Arc::new(Shared {
            gate: Mutex::new(Some((entered_tx, release_rx))),
            source: Mutex::new(None),
            control: Mutex::new(None),
            requested: Mutex::new(Vec::new()),
            queued: AtomicUsize::new(0),
            generations: AtomicUsize::new(0),
            paused: Arc::new(AtomicBool::new(false)),
        }));
        let (tx, rx) = mailbox::channel();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let out = output.clone();
        let state = status.clone();
        let actor = std::thread::spawn(move || {
            audio_thread(rx, state, studio::Shared::default(), || Ok(out.clone()))
        });
        let track = playable_file(name, 10);
        tx.send(Cmd::Play(track.clone())).unwrap();
        wait_until("actor to install the controlled source", || {
            status.lock().unwrap().path.is_some()
        });
        Fixture {
            output,
            tx,
            status,
            actor,
            entered,
            release,
            track,
        }
    }
    #[test]
    fn actor_latest_timeout_cancels_a_superseded_decoder_call_without_a_late_clock_or_sample() {
        let Fixture {
            output,
            tx,
            status,
            actor,
            entered,
            release,
            track,
        } = fixture("actor-owned-seek-timeout");
        tx.send(Cmd::Seek(3.0)).unwrap();
        wait_until("first seek submitted", || {
            output.0.requested.lock().unwrap().len() == 1
        });
        let mut source = output.0.source.lock().unwrap().take().unwrap();
        let consumer = std::thread::spawn(move || source.next());
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        tx.send(Cmd::Seek(1.0)).unwrap();
        wait_until("replacement seek submitted", || {
            output.0.requested.lock().unwrap().len() == 2
        });
        wait_until("latest actor deadline", || {
            let value = status.lock().unwrap();
            value.seek_refused && !value.seek_pending
        });
        assert!(status.lock().unwrap().position_secs < 2.8);
        release.send(()).unwrap();
        assert_eq!(consumer.join().unwrap(), None);
        output.0.queued.store(0, Ordering::SeqCst);
        wait_until("ended source to leave the actor idle", || {
            status.lock().unwrap().path.is_none()
        });
        assert_eq!(status.lock().unwrap().position_secs, 0.0);
        drop(tx);
        actor.join().unwrap();
        clean_up(&track);
    }
    #[test]
    fn actor_stop_and_replay_keep_new_source_independent_of_a_superseded_blocked_seek() {
        let Fixture {
            output,
            tx,
            status,
            actor,
            entered,
            release,
            track,
        } = fixture("actor-owned-seek-replay");
        tx.send(Cmd::Seek(3.0)).unwrap();
        wait_until("first seek submitted", || {
            output.0.requested.lock().unwrap().len() == 1
        });
        let mut old = output.0.source.lock().unwrap().take().unwrap();
        let consumer = std::thread::spawn(move || old.next());
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        tx.send(Cmd::Seek(1.0)).unwrap();
        wait_until("replacement seek submitted", || {
            output.0.requested.lock().unwrap().len() == 2
        });
        tx.send(Cmd::Stop).unwrap();
        wait_until("Stop with blocked old source", || {
            status.lock().unwrap().path.is_none()
        });
        tx.send(Cmd::Play(track.clone())).unwrap();
        wait_until("replacement source installed", || {
            output.0.generations.load(Ordering::SeqCst) == 2
        });
        let mut fresh = output.0.source.lock().unwrap().take().unwrap();
        assert_eq!(fresh.next(), Some(0.0));
        release.send(()).unwrap();
        assert_eq!(consumer.join().unwrap(), None);
        assert_eq!(fresh.next(), Some(1.0));
        assert!(!status.lock().unwrap().seek_pending);
        drop(tx);
        actor.join().unwrap();
        clean_up(&track);
    }
}
