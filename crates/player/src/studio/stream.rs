//! Decode and resample off the device callback. A superseded seek closes its
//! voice and leaves only one pending decoder request on the worker.
use super::{
    retune::{LiveProcessor, RetuneSlot},
    StudioAudition, StudioPreviewRange,
};
use crate::{fixed::Fixed, router, RodioSink};
use rodio::{ChannelCount, SampleRate, Source};
use std::{
    fs::File,
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
use tagrex_audio_workbench::{
    PlaybackHistory, PreparedTreatment, RepairSource, SeekIndex, SelectionPcm,
};

const CHUNK_FRAMES: usize = 256;
const BUFFERED_CHUNKS: usize = 12;
const MAX_PREPARED_CHUNKS: usize = 128;
const MAX_LOOP_SAMPLES: u64 = 1_048_576;
const HANDOFF_WAITING: usize = 0;
const HANDOFF_MISSED: usize = 1;
const HANDOFF_SAMPLE: usize = 2;

pub(super) enum Handoff {
    Waiting,
    Missed,
    Switched,
}

pub(super) struct Worker(Arc<Mailbox>);

#[derive(Default)]
struct Mailbox {
    state: Mutex<Requests>,
    changed: Condvar,
}

#[derive(Default)]
struct Requests {
    next: Option<Job>,
    closed: bool,
}

impl Worker {
    pub(super) fn new() -> std::io::Result<Self> {
        let requests = Arc::new(Mailbox::default());
        let thread = requests.clone();
        std::thread::Builder::new()
            .name("split-studio-decode".into())
            .spawn(move || thread.run())?;
        Ok(Self(requests))
    }

    fn schedule(&self, job: Job) {
        self.0.state.lock().unwrap_or_else(|p| p.into_inner()).next = Some(job);
        self.0.changed.notify_one();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        state.next = None;
        self.0.changed.notify_one();
    }
}

impl Mailbox {
    fn run(&self) {
        loop {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let mut state = self
                .changed
                .wait_while(state, |state| !state.closed && state.next.is_none())
                .unwrap_or_else(|p| p.into_inner());
            if state.closed {
                break;
            }
            let job = state.next.take().expect("a requested decoder job");
            drop(state);
            job.shared
                .analysis
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .set_producing(true);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()))
                .unwrap_or_else(|_| Err("The decoder failed on the recording".into()));
            job.shared.retune.close();
            job.shared
                .analysis
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .set_producing(false);
            if !job.shared.cancelled() {
                if let Err(error) = result {
                    eprintln!("audio: Studio preview failed: {error}");
                    job.shared.failed.store(true, Ordering::Release);
                } else {
                    job.shared.ready.store(true, Ordering::Release);
                }
            }
        }
    }
}

pub(super) struct DecodeRequest {
    pub(super) file: File,
    pub(super) source_rate: u32,
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) loop_start: Option<u64>,
    pub(super) smooth_loop: bool,
    pub(super) seek_index: Option<Arc<SeekIndex>>,
    pub(super) treatment: PreparedTreatment,
}

pub(super) struct Voice {
    sink: RodioSink,
    shared: Arc<Shared>,
    output: Option<PreparedOutput>,
}

struct PreparedOutput {
    receiver: Receiver<Chunk>,
    output: rodio::queue::SourcesQueueOutput,
    channels: ChannelCount,
    rate: SampleRate,
}

impl Voice {
    pub(super) fn new(
        router: &router::Handle,
        volume: f32,
        worker: &Worker,
        decode: DecodeRequest,
    ) -> Self {
        let mut voice = Self::buffer(router, volume, worker, decode, BUFFERED_CHUNKS);
        voice.activate(router, voice.position());
        voice
    }

    pub(super) fn prepare(
        router: &router::Handle,
        volume: f32,
        worker: &Worker,
        decode: DecodeRequest,
    ) -> Self {
        let (_, rate) = router.format();
        let chunks = (rate.get() as usize / 4)
            .div_ceil(CHUNK_FRAMES)
            .clamp(BUFFERED_CHUNKS, MAX_PREPARED_CHUNKS);
        Self::buffer(router, volume, worker, decode, chunks)
    }

    fn buffer(
        router: &router::Handle,
        volume: f32,
        worker: &Worker,
        decode: DecodeRequest,
        chunks: usize,
    ) -> Self {
        let (channels, rate) = router.format();
        let (sender, receiver) = mpsc::sync_channel(chunks);
        let shared = Arc::new(Shared::default());
        shared.paused.store(true, Ordering::Relaxed);
        shared.position_ns.store(
            native_time(decode.start, decode.source_rate),
            Ordering::Relaxed,
        );
        let (sink, output) = RodioSink::new();
        sink.set_volume(volume);
        worker.schedule(Job {
            decode,
            sender,
            shared: shared.clone(),
            channels,
            rate,
            sent: std::cell::Cell::new(0),
        });
        Self {
            sink,
            shared,
            output: Some(PreparedOutput {
                receiver,
                output,
                channels,
                rate,
            }),
        }
    }

    pub(super) fn activate(&mut self, router: &router::Handle, position: Duration) {
        if let Some((source, output)) = self.preload(position) {
            self.sink.append(source);
            router.replace(output);
        }
    }

    pub(super) fn submit_handoff(
        &mut self,
        router: &router::Handle,
        position: Duration,
        active: &Voice,
        audition: Option<StudioAudition>,
        target: Option<StudioPreviewRange>,
    ) {
        let Some((mut source, output)) = self.preload(position) else {
            self.shared
                .handoff_sample
                .store(HANDOFF_MISSED, Ordering::Release);
            return;
        };
        let Some((chunk, _)) = &source.chunk else {
            self.shared
                .handoff_sample
                .store(HANDOFF_MISSED, Ordering::Release);
            return;
        };
        // The controller discards earlier queued chunks. The device needs
        // only one index calculation in the prefetched block at the swap.
        let origin = chunk.origin_nanos;
        let first = chunk.first_frame;
        let channels = usize::from(source.channels.get());
        let frames = (chunk.range.len() / channels) as u64;
        let rate = source.rate.get();
        let active = active.shared.clone();
        let incoming = self.shared.clone();
        let faded = incoming.clone();
        source.align_handoff = true;
        self.sink.append(source);
        router.replace_if(
            output,
            move || {
                let mut position = active.position_ns.load(Ordering::Acquire);
                if let Some(audition) = audition {
                    if position >= audition.end.as_nanos() as u64 {
                        position = audition.start.as_nanos() as u64;
                    }
                }
                if let Some(target) = target {
                    position = target.position(Duration::from_nanos(position)).as_nanos() as u64;
                }
                let frame = output_frames(position.saturating_sub(origin), rate);
                let half_frame = 500_000_000 / u64::from(rate) + 1;
                if !active.cancelled()
                    && !incoming.cancelled()
                    && origin <= position.saturating_add(half_frame)
                    && (first..first + frames).contains(&frame)
                {
                    let sample = (frame - first) as usize * channels;
                    incoming.position_ns.store(position, Ordering::Release);
                    incoming
                        .handoff_sample
                        .store(HANDOFF_SAMPLE + sample, Ordering::Release);
                    true
                } else {
                    incoming
                        .handoff_sample
                        .store(HANDOFF_MISSED, Ordering::Release);
                    false
                }
            },
            move || faded.handoff_complete.store(true, Ordering::Release),
        );
    }

    pub(super) fn handoff(&self) -> Handoff {
        match self.shared.handoff_sample.load(Ordering::Acquire) {
            HANDOFF_WAITING => Handoff::Waiting,
            HANDOFF_MISSED => Handoff::Missed,
            _ if self.shared.handoff_complete.load(Ordering::Acquire) => Handoff::Switched,
            _ => Handoff::Waiting,
        }
    }

    fn preload(
        &mut self,
        position: Duration,
    ) -> Option<(Speaker, rodio::queue::SourcesQueueOutput)> {
        let position = position.as_nanos() as u64;
        self.shared.position_ns.store(position, Ordering::Release);
        self.shared.catch_up_ns.store(position, Ordering::Release);
        let PreparedOutput {
            receiver,
            output,
            channels,
            rate,
        } = self.output.take()?;
        let mut source = Speaker {
            receiver,
            shared: self.shared.clone(),
            channels,
            rate,
            chunk: None,
            channel: 0,
            silent: false,
            ended: false,
            align_handoff: false,
        };
        // Catch up before handing the source to the device callback.
        source.load_chunk(MAX_PREPARED_CHUNKS + 1);
        Some((source, output))
    }

    pub(super) fn covers(&self, position: Duration) -> bool {
        self.output.as_ref().is_some_and(|output| {
            let half_frame = 500_000_000 / u64::from(output.rate.get()) + 1;
            let end = self.shared.buffer_end_ns.load(Ordering::Acquire);
            position.as_nanos() < u128::from(end.saturating_sub(half_frame))
        })
    }

    pub(super) fn full(&self) -> bool {
        self.shared.buffer_full.load(Ordering::Acquire)
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
    }
    pub(super) fn volume(&self) -> f32 {
        self.sink.volume()
    }
    pub(super) fn pause(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
    }
    pub(super) fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire)
    }
    pub(super) fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }
    pub(super) fn ended(&self) -> bool {
        self.shared.ended.load(Ordering::Acquire)
    }
    pub(super) fn retune_eq(&self, target: PreparedTreatment) -> bool {
        self.shared.retune.submit(target)
    }

    pub(super) fn playback_history(&self) -> Arc<Mutex<PlaybackHistory>> {
        self.shared.analysis.clone()
    }

    pub(super) fn position(&self) -> Duration {
        Duration::from_nanos(self.shared.position_ns.load(Ordering::Acquire))
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.pause(true);
        self.shared.cancelled.store(true, Ordering::Release);
        self.shared.changed.notify_one();
        self.sink.stop();
    }
}

#[derive(Default)]
struct Shared {
    paused: AtomicBool,
    cancelled: AtomicBool,
    ready: AtomicBool,
    failed: AtomicBool,
    ended: AtomicBool,
    position_ns: AtomicU64,
    catch_up_ns: AtomicU64,
    handoff_sample: AtomicUsize,
    handoff_complete: AtomicBool,
    buffer_end_ns: AtomicU64,
    buffer_full: AtomicBool,
    analysis: Arc<Mutex<PlaybackHistory>>,
    retune: RetuneSlot,
    gate: Mutex<()>,
    changed: Condvar,
}

impl Shared {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct Chunk {
    samples: Arc<Vec<f32>>,
    range: Range<usize>,
    first_frame: u64,
    origin_nanos: u64,
    repeat: bool,
}

struct Speaker {
    receiver: Receiver<Chunk>,
    shared: Arc<Shared>,
    channels: ChannelCount,
    rate: SampleRate,
    chunk: Option<(Chunk, usize)>,
    channel: u16,
    silent: bool,
    ended: bool,
    align_handoff: bool,
}

impl Iterator for Speaker {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.ended {
            return None;
        }
        if self.channel == 0 {
            if self.align_handoff {
                let sample = self.shared.handoff_sample.load(Ordering::Acquire);
                self.chunk.as_mut().expect("prefetched handoff chunk").1 = sample - HANDOFF_SAMPLE;
                self.align_handoff = false;
            }
            self.silent = self.shared.paused.load(Ordering::Acquire);
            if !self.silent && self.chunk.is_none() {
                self.load_chunk(BUFFERED_CHUNKS + 1);
                if self.ended {
                    return None;
                }
            }
        }
        self.channel += 1;
        if self.channel == self.channels.get() {
            self.channel = 0;
        }
        if self.silent {
            return Some(0.0);
        }
        let (chunk, index) = self
            .chunk
            .as_mut()
            .expect("a chunk for a playing output frame");
        let sample = chunk.samples[chunk.range.start + *index];
        *index += 1;
        if self.channel == 0 {
            let elapsed = chunk.first_frame + (*index / usize::from(self.channels.get())) as u64;
            let rate = u64::from(self.rate.get());
            let position = chunk.origin_nanos
                + elapsed / rate * 1_000_000_000
                + elapsed % rate * 1_000_000_000 / rate;
            self.shared.position_ns.store(position, Ordering::Release);
            if *index == chunk.range.len() {
                if chunk.repeat {
                    *index = 0;
                } else {
                    self.chunk = None;
                }
            }
        }
        Some(sample)
    }
}

impl Speaker {
    fn load_chunk(&mut self, limit: usize) {
        let catch_up = self.shared.catch_up_ns.load(Ordering::Acquire);
        let channels = usize::from(self.channels.get());
        for _ in 0..limit {
            let chunk = match self.receiver.try_recv() {
                Ok(chunk) => chunk,
                Err(TryRecvError::Empty) => {
                    self.silent = true;
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    self.shared.ended.store(true, Ordering::Release);
                    return;
                }
            };
            self.shared.changed.notify_one();
            // Device-rate frame indices are relative to the decode window.
            let skip = if catch_up == 0 {
                0
            } else {
                let elapsed = catch_up.saturating_sub(chunk.origin_nanos);
                let frame = output_frames(elapsed, self.rate.get());
                frame.saturating_sub(chunk.first_frame)
            };
            if skip < (chunk.range.len() / channels) as u64 {
                self.shared.catch_up_ns.store(0, Ordering::Release);
                self.chunk = Some((chunk, skip as usize * channels));
                return;
            }
        }
        self.silent = true;
    }
}

fn output_frames(nanos: u64, rate: u32) -> u64 {
    ((u128::from(nanos) * u128::from(rate) + 500_000_000) / 1_000_000_000) as u64
}

impl Source for Speaker {
    fn current_span_len(&self) -> Option<usize> {
        self.ended.then_some(0)
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct Job {
    decode: DecodeRequest,
    sender: SyncSender<Chunk>,
    shared: Arc<Shared>,
    channels: ChannelCount,
    rate: SampleRate,
    sent: std::cell::Cell<u8>,
}

impl Job {
    fn run(&self) -> Result<(), String> {
        let mut start = self.decode.start;
        while !self.shared.cancelled() {
            let cache_loop = self.decode.loop_start == Some(start)
                && convert_frame(
                    self.decode.end - start,
                    self.decode.source_rate,
                    self.rate.get(),
                )
                .saturating_mul(u64::from(self.channels.get()))
                    <= MAX_LOOP_SAMPLES;
            if cache_loop {
                self.shared
                    .analysis
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .retain_cached_loop(start, self.decode.end);
            }
            let origin_nanos = native_time(start, self.decode.source_rate);
            let cache = self.decode_window(start, origin_nanos, cache_loop)?;
            let Some(loop_start) = self.decode.loop_start else {
                break;
            };
            if let Some(cache) = cache {
                self.send(Chunk {
                    range: 0..cache.len(),
                    samples: cache,
                    first_frame: 0,
                    origin_nanos,
                    repeat: true,
                });
                break;
            }
            start = loop_start;
        }
        Ok(())
    }

    fn decode_window(
        &self,
        start: u64,
        origin_nanos: u64,
        cache: bool,
    ) -> Result<Option<Arc<Vec<f32>>>, String> {
        let treatment = &self.decode.treatment;
        let input_start = if treatment.preview_selection().is_some() {
            start.saturating_sub(4096)
        } else {
            start
        };
        let decode_start = treatment.preview_start(input_start, self.decode.source_rate);
        let decoder = RepairSource::open(
            self.decode.file.try_clone().map_err(|e| e.to_string())?,
            decode_start,
            self.decode.seek_index.as_deref(),
            treatment.settings(),
            || self.shared.cancelled(),
        )
        .map_err(|e| e.to_string())?;
        let processor = treatment
            .processor_at(decoder.source_info(), decode_start)
            .map_err(|e| e.to_string())?;
        let info = processor.output_info();
        let selection = treatment
            .preview_selection()
            .map(|pick| SelectionPcm::new(info, pick.clone(), input_start, start))
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut remaining = (self.decode.end - input_start) * u64::from(info.channels);
        if decoder.source_info().sample_rate != self.decode.source_rate {
            return Err("The recording sample rate changed".into());
        }
        let mut failure = None;
        let retunable = self.decode.loop_start.is_none() && treatment.can_stream_eq();
        let processor = LiveProcessor::new(
            processor,
            treatment.clone(),
            decoder.source_info(),
            retunable,
        );
        if retunable {
            self.shared.retune.enable();
        }
        let native = Native::new(
            decoder,
            processor,
            input_start..self.decode.end,
            decode_start,
            &self.shared,
            &mut remaining,
            &mut failure,
        )?;
        let mut fixed = Fixed::new(
            SelectedSource { native, selection },
            self.channels,
            self.rate,
        );
        let mut cache = cache.then(Vec::new);
        let channels = usize::from(self.channels.get());
        let seam = self
            .decode
            .loop_start
            .filter(|_| self.decode.smooth_loop)
            .map(|loop_start| {
                let device_frame =
                    |frames| convert_frame(frames, self.decode.source_rate, self.rate.get());
                (
                    device_frame(start - loop_start),
                    device_frame(self.decode.end - loop_start),
                )
            });
        let mut position = 0;
        let mut emitted = false;
        while !self.shared.cancelled() {
            let mut pcm = Vec::with_capacity(CHUNK_FRAMES * channels);
            pcm.extend(fixed.by_ref().take(CHUNK_FRAMES * channels));
            pcm.truncate(pcm.len() / channels * channels);
            if pcm.is_empty() {
                break;
            }
            emitted = true;
            if let Some((offset, length)) = seam {
                soften_loop_join(
                    &mut pcm,
                    channels,
                    offset + position,
                    length,
                    self.rate.get(),
                );
            }
            if let Some(cache) = cache.as_mut() {
                cache.extend_from_slice(&pcm);
            }
            let len = pcm.len();
            if !self.send(Chunk {
                samples: Arc::new(pcm),
                range: 0..len,
                first_frame: position,
                origin_nanos,
                repeat: false,
            }) {
                return Ok(None);
            }
            position += (len / channels) as u64;
        }
        drop(fixed);
        if !self.shared.cancelled() && (!emitted || remaining != 0 || failure.is_some()) {
            return Err(
                failure.unwrap_or_else(|| "The recording ended before this preview range".into())
            );
        }
        Ok(cache.map(Arc::new))
    }

    fn send(&self, mut chunk: Chunk) -> bool {
        while !self.shared.cancelled() {
            let frames = (chunk.range.len() / usize::from(self.channels.get())) as u64;
            let end = chunk
                .origin_nanos
                .saturating_add(native_time(chunk.first_frame + frames, self.rate.get()));
            match self.sender.try_send(chunk) {
                Ok(()) => {
                    self.shared.buffer_end_ns.fetch_max(end, Ordering::Release);
                    let sent = self.sent.get().saturating_add(1);
                    self.sent.set(sent);
                    if sent == 4 {
                        self.shared.ready.store(true, Ordering::Release);
                    }
                    return true;
                }
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(returned)) => {
                    chunk = returned;
                    self.shared.buffer_full.store(true, Ordering::Release);
                    let gate = self.shared.gate.lock().unwrap_or_else(|p| p.into_inner());
                    let _idle = self
                        .shared
                        .changed
                        .wait_timeout(gate, Duration::from_millis(8));
                }
            }
        }
        false
    }
}

struct Native<'a> {
    decoder: RepairSource,
    processor: LiveProcessor,
    discard_samples: u64,
    decoded_frame: u64,
    end_frame: u64,
    shared: &'a Shared,
    remaining: &'a mut u64,
    failure: &'a mut Option<String>,
    packet: Vec<i32>,
    index: usize,
    channels: ChannelCount,
    rate: SampleRate,
}

impl<'a> Native<'a> {
    fn new(
        decoder: RepairSource,
        processor: LiveProcessor,
        playback: Range<u64>,
        decode_start: u64,
        shared: &'a Shared,
        remaining: &'a mut u64,
        failure: &'a mut Option<String>,
    ) -> Result<Self, String> {
        let info = processor.output_info();
        let channels = ChannelCount::new(info.channels).ok_or("No audio channels")?;
        let rate = SampleRate::new(info.sample_rate).ok_or("Invalid audio sample rate")?;
        Ok(Self {
            decoder,
            processor,
            discard_samples: (playback.start - decode_start) * u64::from(channels.get()),
            decoded_frame: decode_start,
            end_frame: playback.end,
            shared,
            remaining,
            failure,
            packet: Vec::new(),
            index: 0,
            channels,
            rate,
        })
    }
}

impl Iterator for Native<'_> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if *self.remaining == 0 || self.failure.is_some() {
            return None;
        }
        while self.index == self.packet.len() {
            let input = self.decoder.source_info();
            match self.decoder.next_block(|| self.shared.cancelled()) {
                Ok(Some(block)) => {
                    let frame = self.decoded_frame;
                    let channels = usize::from(input.channels);
                    let frames = block.dry.len() / channels;
                    self.decoded_frame += frames as u64;
                    self.processor.retune(
                        &self.shared.retune,
                        input,
                        frame,
                        self.decoded_frame >= self.end_frame,
                    );
                    let wanted = self
                        .shared
                        .analysis
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .wanted();
                    if let Some(readings) =
                        self.processor
                            .process_source(block, &mut self.packet, wanted)
                    {
                        let output = self.processor.output_info();
                        let inside = frames.min(self.end_frame.saturating_sub(frame) as usize);
                        self.shared
                            .analysis
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(
                                frame,
                                (input, output),
                                self.processor.prepared().settings().clone(),
                                &block.dry[..inside * channels],
                                &self.packet[..inside * usize::from(output.channels)],
                                readings,
                            );
                    }
                    let skip = self.discard_samples.min(self.packet.len() as u64);
                    self.index = skip as usize;
                    self.discard_samples -= skip;
                }
                Ok(None) => return None,
                Err(failure) => {
                    *self.failure = Some(failure.to_string());
                    return None;
                }
            }
        }
        let sample = *self.packet.get(self.index)? as f32 / 2_147_483_648.0;
        self.index += 1;
        *self.remaining -= 1;
        Some(sample)
    }
}

impl Source for Native<'_> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

struct SelectedSource<'a> {
    native: Native<'a>,
    selection: Option<SelectionPcm>,
}

impl Iterator for SelectedSource<'_> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        match &mut self.selection {
            Some(selection) => selection.next_sample(&mut self.native),
            None => self.native.next(),
        }
    }
}

impl Source for SelectedSource<'_> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.native.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.native.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn convert_frame(frame: u64, from: u32, to: u32) -> u64 {
    ((u128::from(frame) * u128::from(to) + u128::from(from / 2)) / u128::from(from)) as u64
}

fn soften_loop_join(samples: &mut [f32], channels: usize, offset: u64, length: u64, rate: u32) {
    let fade = (u64::from(rate) * 3 / 1000).min(length / 4);
    if fade == 0 || (offset >= fade && offset + (samples.len() / channels) as u64 <= length - fade)
    {
        return;
    }
    for (index, frame) in samples.chunks_exact_mut(channels).enumerate() {
        let position = offset + index as u64;
        let edge = position.min(length.saturating_sub(position + 1));
        let ramp = edge.min(fade) as f32 / fade as f32;
        let gain = ramp * ramp * (3.0 - 2.0 * ramp);
        for sample in frame {
            *sample *= gain;
        }
    }
}

fn native_time(frame: u64, rate: u32) -> u64 {
    super::frame_time(frame, rate).as_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::soften_loop_join;

    #[test]
    fn smooth_loop_fades_only_the_linked_boundary_frames() {
        let mut first = [1.0, -1.0].repeat(256);
        let mut last = first.clone();
        soften_loop_join(&mut first, 2, 0, 512, 48_000);
        soften_loop_join(&mut last, 2, 256, 512, 48_000);
        assert_eq!(&first[..2], &[0.0, 0.0]);
        assert_eq!(&last[510..], &[0.0, 0.0]);
        assert_eq!(&first[288..], &[1.0, -1.0].repeat(112));
        assert_eq!(&last[..224], &[1.0, -1.0].repeat(112));
        assert!(first
            .as_chunks::<2>()
            .0
            .iter()
            .zip(last.as_chunks::<2>().0.iter().rev())
            .all(|(a, b)| a == b));
    }
}
