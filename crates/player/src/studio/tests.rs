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
        let (studio_handle, studio) = router::Router::new(channels, rate);
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
fn audition_repeats_selected_frames_and_latest_paused_seek_is_owner_bound() {
    let preview = Recording::ramp("audition", 256, 12_000);
    let mut f = Fixture::new(true);
    let audition = StudioAudition {
        start: point(18),
        end: point(24),
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
        let audition = StudioAudition { start, end };
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
