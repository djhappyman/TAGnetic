//! Decode and resample off the device callback. A superseded seek closes its
//! voice and leaves only one pending decoder request on the worker.
use crate::{fixed::Fixed, router, RodioSink};
use rodio::{ChannelCount, SampleRate, Source};
use std::{
    fs::File,
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
use tagrex_audio_workbench::{PreparedTreatment, PreviewDecoder, SeekIndex, TreatmentProcessor};

const CHUNK_FRAMES: usize = 256;
const BUFFERED_CHUNKS: usize = 12;
const MAX_LOOP_SAMPLES: u64 = 1_048_576;

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
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()))
                .unwrap_or_else(|_| Err("The decoder failed on the recording".into()));
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
    pub(super) seek_index: Option<Arc<SeekIndex>>,
    pub(super) treatment: PreparedTreatment,
}

pub(super) struct Voice {
    sink: RodioSink,
    shared: Arc<Shared>,
}

impl Voice {
    pub(super) fn new(
        router: &router::Handle,
        volume: f32,
        worker: &Worker,
        decode: DecodeRequest,
    ) -> Self {
        let (channels, rate) = router.format();
        let (sender, receiver) = mpsc::sync_channel(BUFFERED_CHUNKS);
        let shared = Arc::new(Shared::default());
        shared.paused.store(true, Ordering::Relaxed);
        shared.position_ns.store(
            native_time(decode.start, decode.source_rate),
            Ordering::Relaxed,
        );
        let source = Speaker {
            receiver,
            shared: shared.clone(),
            channels,
            rate,
            chunk: None,
            channel: 0,
            silent: false,
            ended: false,
        };
        let (sink, output) = RodioSink::new();
        sink.set_volume(volume);
        sink.append(source);
        router.replace(output);
        worker.schedule(Job {
            decode,
            sender,
            shared: shared.clone(),
            channels,
            rate,
            sent: std::cell::Cell::new(0),
        });
        Self { sink, shared }
    }

    pub(super) fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
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
}

impl Iterator for Speaker {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.ended {
            return None;
        }
        if self.channel == 0 {
            self.silent = self.shared.paused.load(Ordering::Acquire);
            if !self.silent && self.chunk.is_none() {
                match self.receiver.try_recv() {
                    Ok(chunk) => {
                        self.chunk = Some((chunk, 0));
                        self.shared.changed.notify_one();
                    }
                    Err(TryRecvError::Empty) => self.silent = true,
                    Err(TryRecvError::Disconnected) => {
                        self.ended = true;
                        self.shared.ended.store(true, Ordering::Release);
                        return None;
                    }
                }
            }
        }
        self.channel = (self.channel + 1) % self.channels.get();
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
        let treatment = self.decode.treatment;
        let decode_start = treatment.preview_start(start, self.decode.source_rate);
        let decoder = PreviewDecoder::open_indexed(
            self.decode.file.try_clone().map_err(|e| e.to_string())?,
            decode_start,
            self.decode.seek_index.as_deref(),
            || self.shared.cancelled(),
        )
        .map_err(|e| e.to_string())?;
        let processor = treatment
            .processor(decoder.source_info())
            .map_err(|e| e.to_string())?;
        let channels = u64::from(processor.output_info().channels);
        let mut remaining = (self.decode.end - start) * channels;
        if decoder.source_info().sample_rate != self.decode.source_rate {
            return Err("The recording sample rate changed".into());
        }
        let mut failure = None;
        let native = Native::new(
            decoder,
            processor,
            (start - decode_start) * channels,
            &self.shared,
            &mut remaining,
            &mut failure,
        )?;
        let mut fixed = Fixed::new(native, self.channels, self.rate);
        let mut cache = cache.then(Vec::new);
        let channels = usize::from(self.channels.get());
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
            match self.sender.try_send(chunk) {
                Ok(()) => {
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
    decoder: PreviewDecoder,
    processor: TreatmentProcessor,
    discard_samples: u64,
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
        decoder: PreviewDecoder,
        processor: TreatmentProcessor,
        discard_samples: u64,
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
            discard_samples,
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
            match self.decoder.next_block(|| self.shared.cancelled()) {
                Ok(Some(block)) => {
                    self.processor.process(block, &mut self.packet);
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

fn convert_frame(frame: u64, from: u32, to: u32) -> u64 {
    ((u128::from(frame) * u128::from(to) + u128::from(from / 2)) / u128::from(from)) as u64
}

fn native_time(frame: u64, rate: u32) -> u64 {
    super::frame_time(frame, rate).as_nanos() as u64
}
