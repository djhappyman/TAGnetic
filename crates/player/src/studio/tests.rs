use super::*;
use crate::{audio_thread, mailbox, Cmd, DevicePlayer, Player, PlayerStatus};
use rodio::stream::DeviceSinkError;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    thread,
};

const RATE: u32 = 8_000;

struct Recording(PathBuf);

impl Recording {
    fn ramp(name: &str, frames: u32, base: i16) -> Self {
        let path =
            std::env::temp_dir().join(format!("tagrex-studio-{name}-{}.wav", std::process::id()));
        let data = frames * 4;
        let mut bytes = Vec::with_capacity((44 + data) as usize);
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(RATE.to_le_bytes());
        bytes.extend((RATE * 4).to_le_bytes());
        bytes.extend(4_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        for frame in 0..frames as i16 {
            let value = base + frame;
            bytes.extend(value.to_le_bytes());
            bytes.extend((-value).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn filename(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).expect("remove disposable Studio fixture");
    }
}

#[derive(Clone)]
struct Routed {
    main: router::Handle,
    studio: router::Handle,
}

impl Output for Routed {
    type Sink = DevicePlayer;

    fn sink(&self, volume: f32) -> DevicePlayer {
        DevicePlayer::new(&self.main, volume)
    }

    fn studio_router(&self) -> Option<router::Handle> {
        Some(self.studio.clone())
    }
}

struct Fixture {
    player: Player,
    main: router::Router,
    studio: router::Router,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn new(has_output: bool) -> Self {
        Self::at_rate(has_output, RATE)
    }

    fn at_rate(has_output: bool, rate: u32) -> Self {
        let channels = rodio::ChannelCount::new(2).unwrap();
        let rate = rodio::SampleRate::new(rate).unwrap();
        let (main_handle, main) = router::Router::new(channels, rate);
        let (studio_handle, studio) = router::Router::new_studio(rate);
        let output = Routed {
            main: main_handle,
            studio: studio_handle,
        };
        let (tx, rx) = mailbox::channel();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let studio_status = Shared::default();
        let main_shared = status.clone();
        let studio_shared = studio_status.clone();
        let handle = thread::spawn(move || {
            audio_thread(rx, main_shared, studio_shared, move || {
                has_output
                    .then(|| output.clone())
                    .ok_or(DeviceSinkError::NoDevice)
            });
        });
        Self {
            player: Player {
                tx,
                status,
                studio_status,
            },
            main,
            studio,
            handle,
        }
    }

    fn wait(&mut self, reason: &str, mut predicate: impl FnMut(&Player) -> bool) {
        crate::test_support::wait_until(reason, || {
            if predicate(&self.player) {
                return true;
            }
            self.studio.next();
            self.studio.next();
            predicate(&self.player)
        });
    }

    fn main_is(player: &Player, file: &Recording) -> bool {
        player
            .status()
            .path
            .as_deref()
            .map(Path::new)
            .map(Path::as_os_str)
            == Some(file.filename())
    }

    fn drain(self) {
        drop(self.player);
        self.handle.join().unwrap();
    }
}

fn pcm(router: &mut router::Router) -> (i32, i32) {
    let frame = (router.next().unwrap(), router.next().unwrap());
    (
        (frame.0 * 32_768.0).round() as i32,
        (frame.1 * 32_768.0).round() as i32,
    )
}

fn first_audio(router: &mut router::Router) -> (i32, i32) {
    (0..1_024)
        .map(|_| pcm(router))
        .find(|sample| *sample != (0, 0))
        .expect("Rodio output advanced past its initial idle padding")
}

fn stable_main_frame(router: &mut router::Router) -> i32 {
    // The main router already existed before Studio and can produce a short
    // mono startup span before its first stereo source is visible.
    (0..1_024)
        .map(|_| pcm(router))
        .find(|&(left, right)| left > 0 && left == -right)
        .expect("main output has reached its stereo source")
        .0
}

fn point(frame: u64) -> Duration {
    frame_time(frame, RATE)
}

#[test]
fn live_treatment_preserves_loop_and_ignores_older_settings_or_owners() {
    use tagrex_audio_workbench::{AudioTreatment, SourceInfo};

    let preview = Recording::ramp("retune", 2_048, 4_000);
    let mut f = Fixture::new(true);
    let audition = StudioAudition {
        start: point(100),
        end: point(108),
        smooth: false,
    };
    let owner = "editor".to_owned();
    f.player
        .studio_play_request(
            owner.clone(),
            3,
            crate::StudioPlayback {
                file: File::open(&preview.0).unwrap(),
                start: audition.start,
                end: point(2_048),
                audition: Some(audition),
                sample_rate: RATE,
                seek_index: None,
                treatment: PreparedTreatment::default(),
            },
        )
        .unwrap();
    f.wait("dry loop playing", |p| p.studio_status("editor").playing);
    let first = first_audio(&mut f.studio).0;
    for _ in first..4_107 {
        pcm(&mut f.studio);
    }
    crate::test_support::wait_until("old output at the exact loop end", || {
        f.player.studio_status("editor").position == audition.end
    });

    let swapped = PreparedTreatment::preview(
        AudioTreatment {
            swap_channels: true,
            ..AudioTreatment::default()
        },
        SourceInfo {
            sample_rate: RATE,
            channels: 2,
            frames: 2_048,
            bits_per_sample: Some(16),
        },
    )
    .unwrap();
    f.player
        .studio_retune(
            owner.clone(),
            3,
            2,
            File::open(&preview.0).unwrap(),
            swapped,
        )
        .unwrap();
    let mut next = 4_100;
    let mut first_treated = 0;
    crate::test_support::wait_until(
        "retune hands off to the same point in the active loop",
        || {
            let (left, right) = pcm(&mut f.studio);
            assert_eq!(left, -right);
            assert!(left.abs() <= next);
            let fully_treated = left == -next;
            next = 4_100 + (next - 4_100 + 1) % 8;
            if fully_treated {
                first_treated = right;
                true
            } else {
                false
            }
        },
    );
    f.player
        .studio_retune(
            owner.clone(),
            3,
            1,
            File::open(&preview.0).unwrap(),
            PreparedTreatment::default(),
        )
        .unwrap();
    f.player
        .studio_retune(
            "stale".into(),
            3,
            6,
            File::open(&preview.0).unwrap(),
            PreparedTreatment::default(),
        )
        .unwrap();
    f.player
        .studio_retune(
            owner,
            2,
            6,
            File::open(&preview.0).unwrap(),
            PreparedTreatment::default(),
        )
        .unwrap();

    assert!((4_100..4_108).contains(&first_treated));
    for step in 1..=16 {
        let expected = 4_100 + (first_treated - 4_100 + step) % 8;
        assert_eq!(pcm(&mut f.studio), (-expected, expected));
    }
    assert_eq!(f.player.studio_status("editor").sequence, 3);
    f.drain();
}

#[test]
fn live_treatment_continues_streaming_and_keeps_current_sound_on_failure() {
    use tagrex_audio_workbench::{AudioTreatment, SourceInfo};

    let recording = Recording::ramp("retune-streaming", 20_000, 4_000);
    let mut f = Fixture::new(true);
    let source = SourceInfo {
        sample_rate: RATE,
        channels: 2,
        frames: 20_000,
        bits_per_sample: Some(16),
    };
    let swapped = PreparedTreatment::preview(
        AudioTreatment {
            swap_channels: true,
            ..AudioTreatment::default()
        },
        source,
    )
    .unwrap();
    f.player
        .studio_play_request(
            "editor".into(),
            9,
            crate::StudioPlayback {
                file: File::open(&recording.0).unwrap(),
                start: point(1_024),
                end: point(source.frames),
                audition: None,
                sample_rate: RATE,
                seek_index: None,
                treatment: PreparedTreatment::default(),
            },
        )
        .unwrap();
    f.wait("streaming dry output", |p| {
        p.studio_status("editor").playing
    });
    let mut previous = first_audio(&mut f.studio).0;
    f.player
        .studio_retune(
            "editor".into(),
            9,
            1,
            File::open(&recording.0).unwrap(),
            swapped,
        )
        .unwrap();
    crate::test_support::wait_until("separate decoder catches the live playhead", || {
        let (left, right) = pcm(&mut f.studio);
        assert_eq!(left, -right);
        previous += 1;
        assert!(left.abs() <= previous);
        left == -previous
    });
    let broken = Recording::ramp("retune-broken", 1, 4_000);
    std::fs::write(&broken.0, b"invalid recording").unwrap();
    f.player
        .studio_retune(
            "editor".into(),
            9,
            2,
            File::open(&broken.0).unwrap(),
            PreparedTreatment::default(),
        )
        .unwrap();
    f.wait(
        "failed change reported without dropping the active voice",
        |p| p.studio_status("editor").retune_failed == Some(2),
    );
    assert!(f.player.studio_status("editor").playing);
    assert_eq!(f.player.studio_status("editor").retune_pending, None);
    assert!(pcm(&mut f.studio).0 < 0);
    f.drain();
}

#[test]
fn retune_handoff_accounts_for_a_frame_rendered_after_the_control_snapshot() {
    use tagrex_audio_workbench::{AudioTreatment, SourceInfo};

    let recording = Recording::ramp("retune-frame-boundary", 12_000, 4_000);
    let file = File::open(&recording.0).unwrap();
    let channels = rodio::ChannelCount::new(2).unwrap();
    let rate = rodio::SampleRate::new(RATE).unwrap();
    let (router, mut output) = router::Router::new(channels, rate);
    let active_worker = Worker::new().unwrap();
    let pending_worker = Worker::new().unwrap();
    let geometry = Geometry {
        sample_rate: RATE,
        end: point(12_000),
        audition: None,
        treatment: PreparedTreatment::default(),
    };
    let active = Voice::new(
        &router,
        1.0,
        &active_worker,
        geometry.request(&file, point(1024), None).unwrap(),
    );
    active.pause(false);
    crate::test_support::wait_until("dry decoder ready", || active.ready());
    let first = first_audio(&mut output);
    let snapshot = active.position();
    let wet = PreparedTreatment::preview(
        AudioTreatment {
            swap_channels: true,
            ..AudioTreatment::default()
        },
        SourceInfo {
            sample_rate: RATE,
            channels: 2,
            frames: 12_000,
            bits_per_sample: Some(16),
        },
    )
    .unwrap();
    let wet_geometry = Geometry {
        treatment: wet.clone(),
        ..geometry.clone()
    };
    let prepare = |position| {
        let pending = Voice::prepare(
            &router,
            1.0,
            &pending_worker,
            wet_geometry.request(&file, position, None).unwrap(),
        );
        crate::test_support::wait_until("treated decoder ready", || pending.ready());
        assert!(pending.covers(position));
        pending.pause(false);
        pending
    };

    // A controller stall can overrun the prefetched block. The router keeps
    // the dry voice attached until a replacement covers the new position.
    let mut expired = prepare(snapshot);
    let mut dry = first;
    for _ in 0..300 {
        let frame = pcm(&mut output);
        assert_eq!(frame, (dry.0 + 1, dry.1 - 1));
        dry = frame;
    }
    expired.submit_handoff(&router, snapshot, &active, None, None);
    let dry = (dry.0 + 1, dry.1 - 1);
    assert_eq!(pcm(&mut output), dry);
    assert!(matches!(expired.handoff(), Handoff::Missed));
    let mut lease = Lease {
        voice: active,
        file: file.try_clone().unwrap(),
        router: router.clone(),
        geometry,
        playing: true,
        seek_index: None,
        pending: Some(Retune {
            voice: expired,
            target: RetuneTarget {
                file: file.try_clone().unwrap(),
                treatment: wet,
                range: None,
            },
            retries: 0,
            submitted: true,
        }),
        queued: Some(RetuneTarget {
            file: file.try_clone().unwrap(),
            treatment: PreparedTreatment::default(),
            range: None,
        }),
        revision: 2,
    };
    assert!(!lease.advance_retune(&pending_worker).unwrap());
    assert!(
        lease.pending.is_none(),
        "an expired preview must leave the worker for the latest edit"
    );
    assert!(lease.queued.is_some());
    let active = lease.voice;

    // The device completes a full stereo frame while the controller still
    // has the earlier position.
    let snapshot = active.position();
    let mut pending = prepare(snapshot);
    let last_dry = pcm(&mut output);
    assert_eq!(last_dry, (dry.0 + 1, dry.1 - 1));
    // Publish partway through another stereo frame. The right channel still
    // belongs to the old frame, and the new voice starts on the next frame.
    let convert = |sample: f32| (sample * 32_768.0).round() as i32;
    assert_eq!(convert(output.next().unwrap()), last_dry.0 + 1);
    pending.submit_handoff(&router, snapshot, &active, None, None);
    assert_eq!(convert(output.next().unwrap()), last_dry.1 - 1);
    // The replacement starts at the same source frame and blends both
    // channels together for the fade. A hard swap starts with opposite polarity.
    let blend_frames = RATE / 250;
    for offset in 0..blend_frames {
        let dry = last_dry.0 + 2 + offset as i32;
        let mix = (offset + 1) as f32 / blend_frames as f32;
        let expected = (dry as f32 * (1.0 - 2.0 * mix)).round() as i32;
        let (left, right) = pcm(&mut output);
        assert!((left - expected).abs() <= 1 && (right + expected).abs() <= 1);
    }
    assert!(matches!(pending.handoff(), Handoff::Switched));
}

#[test]
fn sustained_retunes_hand_off_an_intermediate_voice_and_the_last_edit() {
    use tagrex_audio_workbench::{AudioTreatment, SourceInfo};

    let recording = Recording::ramp("retune-coalescing", 20_000, 4_000);
    let channels = rodio::ChannelCount::new(2).unwrap();
    let rate = rodio::SampleRate::new(RATE).unwrap();
    let (main, _main_output) = router::Router::new(channels, rate);
    let (studio, mut output) = router::Router::new(channels, rate);
    let routed = Routed { main, studio };
    let mut actor = Actor::new(Shared::default());
    let info = SourceInfo {
        sample_rate: RATE,
        channels: 2,
        frames: 20_000,
        bits_per_sample: Some(16),
    };
    let dry = PreparedTreatment::default();
    let first = PreparedTreatment::preview(
        AudioTreatment {
            swap_channels: true,
            ..AudioTreatment::default()
        },
        info,
    )
    .unwrap();
    let gain = |gain_db| {
        PreparedTreatment::preview(
            AudioTreatment {
                gain_db,
                ..AudioTreatment::default()
            },
            info,
        )
        .unwrap()
    };

    actor.handle(
        Command::Play {
            owner: "editor".into(),
            sequence: 1,
            file: File::open(&recording.0).unwrap(),
            start: point(1_024),
            end: point(info.frames),
            audition: None,
            sample_rate: RATE,
            seek_index: None,
            treatment: Box::new(dry.clone()),
            channel: StudioMonitorChannel::Stereo,
        },
        Some(&routed),
        1.0,
    );
    crate::test_support::wait_until("dry voice is playing", || {
        actor.tick();
        actor.state.playing
    });
    let dry_sample = first_audio(&mut output).0;
    let retune = |revision, treatment| Command::Retune {
        owner: "editor".into(),
        sequence: 1,
        revision,
        file: File::open(&recording.0).unwrap(),
        treatment: Box::new(treatment),
        range: None,
    };
    actor.handle(retune(1, first.clone()), Some(&routed), 1.0);
    for revision in 2..=12 {
        actor.handle(
            retune(revision, gain(-(revision as i8))),
            Some(&routed),
            1.0,
        );
    }
    assert_eq!(actor.shared.for_owner("editor").retune_pending, Some(12));
    fn current(actor: &Actor) -> &PreparedTreatment {
        &actor.lease.as_ref().unwrap().geometry.treatment
    }
    crate::test_support::wait_until("the prepared edit hands off despite newer requests", || {
        actor.tick();
        if same_treatment(current(&actor), &dry) {
            let _ = pcm(&mut output);
            false
        } else {
            true
        }
    });
    assert!(same_treatment(current(&actor), &first));
    assert!(pcm(&mut output).0 < 0);

    let latest = gain(-12);
    crate::test_support::wait_until("the latest edit alone follows the handoff", || {
        actor.tick();
        if same_treatment(current(&actor), &first) {
            let _ = pcm(&mut output);
            false
        } else {
            true
        }
    });
    assert!(same_treatment(current(&actor), &latest));
    assert_eq!(actor.shared.for_owner("editor").retune_pending, None);
    assert!((1..dry_sample / 2).contains(&pcm(&mut output).0));

    actor.handle(retune(13, first), Some(&routed), 1.0);
    actor.handle(retune(14, dry.clone()), Some(&routed), 1.0);
    crate::test_support::wait_until(
        "the final bypass request restores the original sound",
        || {
            actor.tick();
            if same_treatment(current(&actor), &dry) {
                true
            } else {
                let _ = pcm(&mut output);
                false
            }
        },
    );
    assert!(pcm(&mut output).0 >= dry_sample);
    assert_eq!(actor.state.retune_failed, None);
}

#[test]
fn studio_freezes_exact_main_samples_and_keeps_queued_next_track() {
    let recording = Recording::ramp("library", 2_048, 4_000);
    let upcoming = Recording::ramp("library-next", 64, 8_000);
    let preview = Recording::ramp("preview", 256, 12_000);
    let mut f = Fixture::new(true);
    f.player.play(recording.0.clone()).unwrap();
    f.player.set_next(upcoming.0.clone()).unwrap();
    f.wait("the main track and next are queued", |p| {
        Fixture::main_is(p, &recording) && !p.status().wants_next
    });
    let first = stable_main_frame(&mut f.main);
    for value in first + 1..first + 12 {
        assert_eq!(pcm(&mut f.main), (value, -value));
    }

    f.player.studio_seek("editor".into(), point(15)).unwrap();
    f.wait("idle seek is local to the editor", |p| {
        p.studio_status("editor").position == point(15)
    });
    assert!(!f.player.status().is_paused);
    assert_eq!(pcm(&mut f.main), (first + 12, -(first + 12)));

    f.player
        .studio_play(
            "editor".into(),
            File::open(&preview.0).unwrap(),
            point(15),
            point(256),
            None,
            RATE,
        )
        .unwrap();
    f.wait("preview plays without replacing the main queue", |p| {
        p.studio_status("editor").playing && p.status().is_paused
    });
    let frozen = f.player.status().position_secs;
    for _ in 0..64 {
        assert_eq!(pcm(&mut f.main), (0, 0));
    }
    assert_eq!(f.player.status().position_secs, frozen);
    assert!(Fixture::main_is(&f.player, &recording));

    f.player.studio_close("old-editor".into()).unwrap();
    f.player.studio_pause("editor".into()).unwrap();
    f.wait("owner pause keeps main suspended", |p| {
        !p.studio_status("editor").playing && p.status().is_paused
    });
    assert_eq!(pcm(&mut f.main), (0, 0));
    f.player.studio_close("editor".into()).unwrap();
    f.wait("close resumes main only once", |p| !p.status().is_paused);
    for value in first + 13..6_048 {
        assert_eq!(pcm(&mut f.main), (value, -value));
    }
    assert_eq!(pcm(&mut f.main), (8_000, -8_000));
    f.wait("queued gapless source still becomes current", |p| {
        Fixture::main_is(p, &upcoming)
    });
    f.drain();
}

#[test]
fn spectral_selection_plays_to_its_end_once_or_repeats_from_its_start() {
    use tagrex_audio_workbench::{RepairChannel, SpectralRepair};

    let recording = Recording::ramp("spectral-selection", 4096, 8000);
    let mut fixture = Fixture::new(true);
    let start = 500;
    let end = 1500;
    let owner = "editor".to_string();
    let treatment = PreparedTreatment::default().with_preview_selection(Some(SpectralRepair {
        start_frame: start,
        end_frame: end,
        low_hz: 0,
        high_hz: RATE / 2,
        amount_db: 1,
        channel: Some(RepairChannel::Left),
        ..SpectralRepair::default()
    }));
    for (sequence, repeat) in [(1, false), (2, true)] {
        fixture
            .player
            .studio_play_request(
                owner.clone(),
                sequence,
                crate::StudioPlayback {
                    file: File::open(&recording.0).unwrap(),
                    start: point(start),
                    end: point(if repeat { 4096 } else { end }),
                    audition: repeat.then_some(StudioAudition {
                        start: point(start),
                        end: point(end),
                        smooth: false,
                    }),
                    sample_rate: RATE,
                    seek_index: None,
                    treatment: treatment.clone(),
                },
            )
            .unwrap();
        fixture.wait("the spectral selection is audible", |player| {
            let status = player.studio_status(&owner);
            status.sequence == sequence && status.playing
        });
        let (_, right) = first_audio(&mut fixture.studio);
        assert_eq!(right, 0);
        if repeat {
            for _ in 0..2 * (end - start) {
                assert_eq!(pcm(&mut fixture.studio).1, 0);
            }
            assert!(fixture.player.studio_status(&owner).playing);
        } else {
            fixture.wait("one-shot playback ends at the selected frame", |player| {
                let status = player.studio_status(&owner);
                !status.playing && status.position == point(end)
            });
        }
    }
    fixture.drain();
}

#[test]
fn studio_peaks_follow_audible_selection_and_channel_switches_without_restarting() {
    use tagrex_audio_workbench::{RepairChannel, SpectralRepair};

    let recording = Recording::ramp("audible-meter", 4_096, 8_000);
    let mut fixture = Fixture::new(true);
    let owner = "editor".to_string();
    let sequence = 3;
    let treatment = PreparedTreatment::default().with_preview_selection(Some(SpectralRepair {
        start_frame: 500,
        end_frame: 2_000,
        high_hz: RATE / 2,
        amount_db: 1,
        channel: Some(RepairChannel::Left),
        ..SpectralRepair::default()
    }));
    fixture
        .player
        .studio_play_monitored(
            owner.clone(),
            sequence,
            crate::StudioPlayback {
                file: File::open(&recording.0).unwrap(),
                start: point(900),
                end: point(4_096),
                audition: Some(StudioAudition {
                    start: point(500),
                    end: point(2_000),
                    smooth: false,
                }),
                sample_rate: RATE,
                seek_index: None,
                treatment,
            },
            StudioMonitorChannel::Stereo,
        )
        .unwrap();
    fixture.wait("the selected channel is audible", |player| {
        player.studio_status(&owner).playing
    });
    first_audio(&mut fixture.studio);
    for _ in 0..64 {
        assert_eq!(pcm(&mut fixture.studio).1, 0);
    }
    let [left, right] = fixture.player.studio_meter(&owner, sequence, None).unwrap();
    assert!(left > 0.2);
    assert_eq!(
        right, 0.0,
        "reviewing the pre-mask decoder would show the right channel"
    );

    fixture
        .player
        .studio_meter("stale-owner", sequence, Some(StudioMonitorChannel::Right));
    fixture
        .player
        .studio_meter(&owner, sequence - 1, Some(StudioMonitorChannel::Right));
    for _ in 0..64 {
        assert!(pcm(&mut fixture.studio).0 > 0);
    }
    fixture
        .player
        .studio_meter(&owner, sequence, Some(StudioMonitorChannel::Right));
    // Give the short monitor ramp time to finish and clear peaks from before it.
    for _ in 0..96 {
        pcm(&mut fixture.studio);
    }
    fixture.player.studio_meter(&owner, sequence, None);
    for _ in 0..64 {
        assert_eq!(pcm(&mut fixture.studio), (0, 0));
    }
    assert_eq!(
        fixture.player.studio_meter(&owner, sequence, None),
        Some([0.0; 2])
    );

    fixture
        .player
        .studio_meter(&owner, sequence, Some(StudioMonitorChannel::Stereo));
    for _ in 0..64 {
        pcm(&mut fixture.studio);
    }
    let [left, right] = fixture.player.studio_meter(&owner, sequence, None).unwrap();
    assert!(left > 0.2);
    assert_eq!(right, 0.0);
    let status = fixture.player.studio_status(&owner);
    assert!(status.playing);
    assert_eq!(status.sequence, sequence);
    fixture.drain();
}

#[test]
fn editing_a_running_spectral_selection_keeps_its_position_and_uses_the_new_loop() {
    let recording = Recording::ramp("edit-spectral-loop", 12_000, 8_000);
    let mut fixture = Fixture::new(true);
    let owner = "editor".to_string();
    let selection = |start_frame, end_frame| {
        PreparedTreatment::default().with_preview_selection(Some(
            tagrex_audio_workbench::SpectralRepair {
                start_frame,
                end_frame,
                high_hz: RATE / 2,
                amount_db: 1,
                channel: Some(tagrex_audio_workbench::RepairChannel::Left),
                ..Default::default()
            },
        ))
    };
    fixture
        .player
        .studio_play_request(
            owner.clone(),
            1,
            crate::StudioPlayback {
                file: File::open(&recording.0).unwrap(),
                start: point(3_000),
                end: point(12_000),
                audition: Some(StudioAudition {
                    start: point(500),
                    end: point(9_000),
                    smooth: false,
                }),
                sample_rate: RATE,
                seek_index: None,
                treatment: selection(500, 9_000),
            },
        )
        .unwrap();
    fixture.wait("the first selection is playing", |player| {
        player.studio_status(&owner).playing
    });
    let before = first_audio(&mut fixture.studio).0;
    assert!(before >= 11_000);
    fixture
        .player
        .studio_retune_range(
            owner.clone(),
            1,
            1,
            File::open(&recording.0).unwrap(),
            selection(1_500, 6_000),
            Some(StudioPreviewRange {
                start: point(1_500),
                end: point(6_000),
                repeat: true,
                smooth: false,
            }),
        )
        .unwrap();
    fixture.wait("the edited selection was scheduled", |player| {
        player.studio_status(&owner).retune_pending == Some(1)
    });
    fixture.wait("the edited selection is playing", |player| {
        let status = player.studio_status(&owner);
        status.retune_pending.is_none() && status.retune_failed.is_none()
    });
    let status = fixture.player.studio_status(&owner);
    assert_eq!(status.sequence, 1);
    assert!(status.playing && status.position >= point(3_000));

    let mut previous = status.position;
    let (last, first) = (0..100)
        .find_map(|_| {
            for _ in 0..64 {
                assert_eq!(pcm(&mut fixture.studio).1, 0);
            }
            // The fake device advances on demand, but the actor publishes its
            // position on a separate tick. Read it before pulling another block.
            let mut next = previous;
            crate::test_support::wait_until("the Studio clock to report the output block", || {
                next = fixture.player.studio_status(&owner).position;
                next != previous
            });
            let wrap = (next < previous).then_some((previous, next));
            previous = next;
            wrap
        })
        .expect("the edited range should wrap before the original end");
    assert!(
        (point(5_872)..=point(6_000)).contains(&last),
        "last position: {last:?}"
    );
    assert!(
        (point(1_500)..point(1_628)).contains(&first),
        "first position: {first:?}"
    );
    fixture.drain();
}

#[test]
fn audition_repeats_selected_frames_and_latest_paused_seek_is_owner_bound() {
    let preview = Recording::ramp("audition", 256, 12_000);
    let mut f = Fixture::new(true);
    let audition = StudioAudition {
        start: point(18),
        end: point(24),
        smooth: false,
    };
    f.player
        .studio_play(
            "editor".into(),
            File::open(&preview.0).unwrap(),
            audition.start,
            point(256),
            Some(audition),
            RATE,
        )
        .unwrap();
    f.wait("initial audition seek", |p| {
        p.studio_status("editor").playing
    });
    let first = first_audio(&mut f.studio).0;
    assert!((12_018..12_024).contains(&first));
    for i in 1..=12 {
        let expected = 12_018 + (first - 12_018 + i) % 6;
        assert_eq!(pcm(&mut f.studio), (expected, -expected));
    }

    f.player.studio_pause("editor".into()).unwrap();
    f.wait("editor paused", |p| !p.studio_status("editor").playing);
    f.player.studio_seek("editor".into(), point(20)).unwrap();
    f.player.studio_seek("editor".into(), point(22)).unwrap();
    f.player.studio_seek("stale".into(), point(19)).unwrap();
    f.player.studio_close("stale".into()).unwrap();
    f.wait(
        "latest armed seek is acknowledged without rendering real samples",
        |p| {
            p.studio_status("editor")
                == StudioStatus {
                    position: point(22),
                    playing: false,
                    state: StudioPreviewState::Ready,
                    sequence: 0,
                    retune_failed: None,
                    retune_pending: None,
                }
        },
    );
    assert_eq!(pcm(&mut f.studio), (0, 0));
    assert_eq!(
        f.player.studio_status("stale").state,
        StudioPreviewState::Refused
    );

    f.player
        .studio_play(
            "editor".into(),
            File::open(&preview.0).unwrap(),
            point(22),
            point(256),
            None,
            RATE,
        )
        .unwrap();
    f.wait("play from the acknowledged paused position", |p| {
        p.studio_status("editor").playing
    });
    let (left, right) = first_audio(&mut f.studio);
    assert!((12_022..=12_023).contains(&left));
    assert_eq!(right, -left);
    f.drain();
}

#[test]
fn main_transport_takes_precedence_and_closed_owner_cannot_restart_it() {
    let recording = Recording::ramp("user-control", 2_048, 4_000);
    let mut f = Fixture::new(true);
    f.player.play(recording.0.clone()).unwrap();
    f.wait("main start", |p| Fixture::main_is(p, &recording));
    f.player
        .studio_play(
            "editor".into(),
            File::open(&recording.0).unwrap(),
            Duration::ZERO,
            point(256),
            None,
            RATE,
        )
        .unwrap();
    f.wait("preview owns output", |p| {
        p.studio_status("editor").playing && p.status().is_paused
    });
    f.player.pause().unwrap();
    f.player.studio_close("editor".into()).unwrap();
    f.wait("main pause and subsequent editor close", |p| {
        p.studio_status("editor").state == StudioPreviewState::Idle
    });
    assert!(f.player.status().is_paused);
    assert_eq!(pcm(&mut f.main), (0, 0));
    f.player.tx.send(Cmd::Resume).unwrap();
    f.wait("explicit compact resume", |p| !p.status().is_paused);
    assert!((4_000..6_048).contains(&stable_main_frame(&mut f.main)));
    f.drain();
}

#[test]
fn no_device_or_audio_at_an_unreachable_seek_is_reported() {
    let preview = Recording::ramp("failure", 64, 12_000);
    let mut offline = Fixture::new(false);
    offline
        .player
        .studio_play(
            "editor".into(),
            File::open(&preview.0).unwrap(),
            Duration::ZERO,
            point(64),
            None,
            RATE,
        )
        .unwrap();
    offline.wait("headless audio unavailable", |p| {
        p.studio_status("editor").state == StudioPreviewState::Unavailable
    });
    offline.drain();

    let mut f = Fixture::new(true);
    f.player
        .studio_play(
            "editor".into(),
            File::open(&preview.0).unwrap(),
            point(90),
            point(128),
            None,
            RATE,
        )
        .unwrap();
    f.wait("decoder did not emit audio for the unreachable seek", |p| {
        p.studio_status("editor").state == StudioPreviewState::Refused
    });
    assert_eq!(pcm(&mut f.studio), (0, 0));
    f.drain();
}

#[test]
fn preview_reads_padded_aiff_at_its_native_frame() {
    let path =
        std::env::temp_dir().join(format!("tagrex-studio-padded-{}.aiff", std::process::id()));
    std::fs::write(&path, crate::aiff::tests::aiff(256, 2_280)).unwrap();
    let recording = Recording(path);
    let rate = 44_100;
    let mut f = Fixture::at_rate(true, rate);
    f.player
        .studio_play(
            "editor".into(),
            File::open(&recording.0).unwrap(),
            frame_time(17, rate),
            frame_time(256, rate),
            None,
            rate,
        )
        .unwrap();
    f.wait("padded AIFF decodes at the selected frame", |p| {
        p.studio_status("editor").playing
    });
    let (left, right) = first_audio(&mut f.studio);
    assert!([17 * 37, 18 * 37].contains(&left));
    assert_eq!(right, -left);
    f.drain();
}

#[test]
fn paused_seek_cancels_a_full_decoder_queue_and_keeps_exact_native_time() {
    let recording = Recording::ramp("stalled-decode", 8_192, 12_000);
    let mut f = Fixture::at_rate(true, 4_000);
    f.player
        .studio_play(
            "editor".into(),
            File::open(&recording.0).unwrap(),
            point(21),
            point(8_192),
            None,
            RATE,
        )
        .unwrap();
    f.player.studio_pause("editor".into()).unwrap();
    // Leave the device idle until the voice has primed. Its bounded queue
    // cannot accept the whole file. A new owner-bound seek must cancel that
    // producer without waiting for the device to play queued samples.
    crate::test_support::wait_until("ready at the native frame without consuming", || {
        f.player.studio_status("editor")
            == StudioStatus {
                position: point(21),
                playing: false,
                state: StudioPreviewState::Ready,
                sequence: 0,
                retune_failed: None,
                retune_pending: None,
            }
    });
    for position in 100..255 {
        f.player
            .studio_seek("editor".into(), point(position))
            .unwrap();
    }
    crate::test_support::wait_until("latest native seek without consuming older jobs", || {
        f.player.studio_status("editor")
            == StudioStatus {
                position: point(254),
                playing: false,
                state: StudioPreviewState::Ready,
                sequence: 0,
                retune_failed: None,
                retune_pending: None,
            }
    });
    f.player.studio_seek("editor".into(), point(255)).unwrap();
    crate::test_support::wait_until("odd source frame at a lower output rate", || {
        f.player.studio_status("editor")
            == StudioStatus {
                position: point(255),
                playing: false,
                state: StudioPreviewState::Ready,
                sequence: 0,
                retune_failed: None,
                retune_pending: None,
            }
    });
    f.player.studio_close("editor".into()).unwrap();
    f.wait("closed without draining discarded samples", |p| {
        p.studio_status("editor").state == StudioPreviewState::Idle
    });
    assert_eq!(pcm(&mut f.studio), (0, 0));
    f.drain();
}

#[test]
#[ignore = "generated CBR and VBR MP3 preview proof requires ffmpeg"]
fn native_cbr_and_vbr_mp3_seek_and_loop_at_the_recorded_section() {
    for (name, rate_control, xing) in [
        ("cbr", ["-b:a", "128k"], "1"),
        ("vbr", ["-q:a", "3"], "1"),
        ("headerless-vbr", ["-q:a", "3"], "0"),
    ] {
        let path = std::env::temp_dir().join(format!(
            "tagrex-studio-proof-{name}-{}.mp3",
            std::process::id()
        ));
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=sin(2*PI*440*t)*(0.04+0.70*gte(t\\,2)):s=44100:d=4",
                "-c:a",
                "libmp3lame",
            ])
            .args(rate_control)
            .args(["-write_xing", xing])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recording = Recording(path);
        let mut f = Fixture::new(true);
        let info =
            tagrex_audio_workbench::inspect_file(File::open(&recording.0).unwrap(), 8, || false)
                .unwrap()
                .source;
        let duration = frame_time(info.frames, info.sample_rate);
        let start = duration - Duration::from_millis(170);
        let end = duration - Duration::from_millis(70);
        let audition = StudioAudition {
            start,
            end,
            smooth: false,
        };
        f.player
            .studio_play(
                "editor".into(),
                File::open(&recording.0).unwrap(),
                start,
                duration,
                Some(audition),
                info.sample_rate,
            )
            .unwrap();
        f.wait("native MP3 opens and seeks", |p| {
            p.studio_status("editor").playing
        });
        let initial = first_audio(&mut f.studio);
        assert_eq!(
            initial.0, initial.1,
            "mono should reach both output channels"
        );
        for _ in 0..3 {
            let power = (0..800)
                .map(|_| f64::from(pcm(&mut f.studio).0) / 32_768.0)
                .map(|value| value * value)
                .sum::<f64>()
                / 800.0;
            assert!(
                power > 0.1,
                "{name} MP3 jumped into the quiet portion rather than the selected one ({power})"
            );
        }
        f.player.studio_close("editor".into()).unwrap();
        f.wait("preview lease closes", |p| {
            p.studio_status("editor").state == StudioPreviewState::Idle
        });
        f.drain();
    }
}
