# Player and resource-queue review

Date: 18 November 2026. Branch `feat/native-field-foundations`, baseline
`0554022c` (code `b657f65d`). This is a source-directed review with measurements
of `crates/player`, `app/src/blocking.rs` and the playback adapters in
`app/src/main.rs`. It is not a live profile of the installed app, not a
real-device measurement and not a platform certification. No user library was
opened and no audio file was written.

Evidence tier: graph Tier 2 for discovery, source and tests for every claim. The
index (generation 2026-11-13) is stale for this branch, so claims were read from
the working tree and from the resolved dependency sources; neither is covered by
the index, and no claim here depends on graph completeness. This revision answers
the parent's review of the first playback pass: the paused-allocation claim, the
seek check/use race, the rodio seek wait, and the report wording.

## Reviewed scope

- `crates/player/src/lib.rs` — command loop, decode path, clock, seek, gapless
  priming, no-device recovery, status publication, shutdown.
- `crates/player/src/mailbox.rs` — bounded admission, coalescing, barriers,
  shutdown, and the seek/volume value contract.
- `crates/player/src/mp4.rs`, `aiff.rs` — pre-decode file checks.
- `crates/player/src/tempo.rs` — cancellation and chunking (read; unchanged).
- `app/src/blocking.rs` — admission, lane cancellation, waker, lock ownership.
- Playback adapters: `player_*` commands in `app/src/main.rs`, the `playback.*`
  block in `app/src/mcp_control.rs`.
- Dependency sources actually resolved by this checkout, read directly:
  `rodio-0.23.0/src/player.rs`, `rodio-0.23.0/src/source/done.rs`,
  `rodio-0.23.0/src/stream.rs`, `cpal-0.18.2/src/host/alsa/mod.rs`.

## Confirmed and fixed

### 1. An idle player woke five times a second for nothing

Trigger: the app is open with the player bar idle (no track, or playback
finished). The audio thread ran `recv_timeout(200 ms)` in a loop and rebuilt the
status snapshot on every timeout, although nothing was queued to advance and
nothing in the snapshot could change.

Evidence: temporary counter in the loop (removed after measuring), same test on
the same build: **10 wakeups in 2 s with the always-tick behaviour, 0 with
nothing queued.**

Fix: the wait is `PLAYING_TICK` (200 ms) only while a track is queued; with an
empty queue the thread parks on its command channel with `IDLE_WAIT` as a safety
timeout. `IDLE_WAIT` is **60 seconds**, not "forever": an idle player wakes at
most **60 times an hour instead of 18,000**. A command wakes it immediately, and
every state that can change while idle is command-driven — the device is only
retried on Play, and the clock is stopped. Since this pass an idle safety wake
also publishes nothing when the snapshot is unchanged, so it costs no allocation
and no status lock either. Tests:
`an_idle_player_answers_a_command_immediately` and the existing no-device retry
tests (a device still opens on the next Play).

User benefit: an app sitting idle in the background stops waking a thread and a
CPU core five times a second. That saving is measured as wakeup count; the CPU
and energy saving itself is an estimate, not a measurement.

### 2. A paused player still rebuilt, and allocated, an identical snapshot

Trigger: pause, or any stretch where the reported position is constant. Each
200 ms tick built a `PlayerStatus`, allocated the track path again and assigned
it while holding the status mutex the UI polls and clones from.

Evidence: the first pass stored the snapshot only when it differed, but still
built it first — `status_snapshot` called `current.path.to_string_lossy()
.into_owned()` before the comparison. So that pass removed identical **stores**
(10 stores in 2 s, then 0) and not the per-tick allocation. Measured allocation
evidence now: a test-only counting allocator on the player thread shows **5
allocations in a one-second paused window before this pass's change, 0 after** —
five ticks, one path string each, exactly the allocation identified here.

Fix: `write_status` now compares a `StatusKey` first. The key carries the track
by shared identity (`Arc<PathBuf>`), the clock position and duration as
`Duration`, and the flags — comparing it allocates nothing. Only a changed key
materialises the `PlayerStatus` (the one place the path string is built) and
takes the status mutex; an unchanged one returns before either. `PlayerStatus`
therefore no longer needs the `PartialEq` derive the first pass added, and the
crate's public API is unchanged from baseline. Poison recovery on the status
lock is kept.

Measured effect, stated precisely: while paused or idle, **0 path allocations, 0
status stores and 0 producer lock acquisitions per tick**, where the first pass
still had 1 allocation and 1 lock per tick and the baseline had 1 allocation, 1
store and 1 lock. A paused player still wakes on the 200 ms tick — that tick is
what notices a track boundary and a resume, so it is kept — and only its cost
changed. While a track is playing the reported position changes every
tick, so one path string is still built per published snapshot (5/s); that
snapshot genuinely changed, it is the pre-existing cost of the status contract,
and it is unchanged by this pass.

Tests: `a_paused_player_allocates_nothing_while_the_snapshot_is_unchanged` and
the existing pause test. The allocation test uses a `#[cfg(test)]`
`GlobalAlloc` that counts allocations made by opted-in threads (the player thread
only, so parallel tests do not contribute). It measures **calls**, not bytes, in
the test build; it has a positive control — resuming must move the counter — so a
counter that never runs cannot pass it. Red-green was run: with the early return
temporarily removed the test failed with "a paused player allocated 5 times while
nothing changed"; with it restored it passes.

User benefit: a paused player no longer allocates or takes the UI's status lock
once per tick, so a status poll never waits behind this thread's path allocation.

### 3. A seek could move the clock while no audio was behind it

Trigger: a seek arriving as the track reaches its end; first pass checked
`sink.is_empty()` and then called `RodioSink::try_seek`, which is a check/use
race. rodio confirms both halves: `Player::try_seek` loads `sound_count` and
returns `Ok(())` **without seeking** when it is 0 ("No sound is playing, seek
will not be performed", `rodio-0.23.0/src/player.rs:242-245`), and `sound_count`
is decremented by `Done` when a source is exhausted
(`rodio-0.23.0/src/source/done.rs:57-59`). The first pass moved the clock on
`Ok`, so it could show a position nothing was playing from — the failure #190
exists to prevent.

Two distinct outcomes are now handled, judged from the sink after the call:

- **The last track played out during the call.** `try_seek` returned `Ok`
  without seeking; the clock must not move, and the boundary check takes the
  player idle.
- **The track played out while a later track was already queued.** rodio's seek
  order belongs to the sink, not to a track (`Controls::seek`, consumed by
  whichever source's 5 ms `periodic_access` runs first,
  `rodio-0.23.0/src/player.rs:130-156,238-257`), so that seek can land on the
  track that is starting. The player now rebuilds that source on a fresh sink so
  it starts from its beginning, where the clock restarts it. The only loss is
  the gapless boundary the race had already broken; a seek cannot be taken back.

Tests, all deterministic through the fake sink, no hardware:

- `a_seek_into_track_that_played_out_does_not_move_the_clock` — the
  played-out interval itself: the sink drops the only track inside
  `try_seek` and answers `Ok`; the player goes idle with the clock at zero and
  no refusal.
- `a_seek_across_a_boundary_restarts_the_next_track_from_its_beginning` — the
  sink drops the current track inside `try_seek` while a primed track remains;
  the next track is appended again on a new sink and reported from ~zero.
  Red-green: without the boundary outcome the test fails ("the next track was
  not rebuilt after the seek crossed the boundary", 2 appends instead of 3).
- `a_refused_seek_is_reported_and_does_not_move_the_clock`,
  `a_seek_past_the_end_stays_inside_the_track` — refusal and clamping, unchanged.

Unresolved residual, stated rather than papered over: rodio also returns
`Ok(())` when its feedback channel closes without seeking
(`rodio-0.23.0/src/player.rs:247-256`). That needs a second concurrent seek on
the same sink (there is one seeker), or a panicked audio thread; it is not
distinguishable from success at this seam without inferring from
`Player::get_pos`, which rodio sets to the *requested* position on the success
path and would misread a saturated seek once the 5 ms updater runs. Treating the
rare case as success remains a dependency limitation, not a verified fix.

### 4. The whole post-device playback state machine was untested

Trigger: existing player tests could only inject a *failing* device opener, so
everything after a device is opened — gapless priming, the track boundary, pause
and seek accounting, Stop, volume across sinks, accepted-order of a control
burst — had no regression test and needed a sound card to exercise.

Fix: the playback loop depends on a two-method `Output` seam (`sink(volume)`
plus the sink operations it uses), with rodio as the one production
implementation and a fake sink in tests. The loop, clock, mailbox and decoders
are unchanged for production; `log_on_drop(false)` moved into `open_output` so
the retry path cannot forget it.

New tests in the first pass (all with a 10 s deadline, no hardware, real AIFF
fixtures decoded through the real decoder):

- `a_primed_track_crosses_the_boundary_gaplessly_and_primes_the_one_after`
- `a_track_that_plays_out_leaves_the_player_idle`
- `pause_holds_the_clock_and_resume_lets_it_run_again`
- `a_refused_seek_is_reported_and_does_not_move_the_clock`
- `a_seek_past_the_end_stays_inside_the_track`
- `an_accepted_stop_clears_the_queue_the_clock_and_the_refusal`
- `a_new_play_replaces_the_session_instead_of_adding_to_it`
- `the_volume_is_reapplied_to_every_new_sink`
- `control_commands_with_nothing_playing_are_ignored`
- `a_track_that_cannot_be_decoded_leaves_the_player_idle_and_then_plays`
- `a_burst_of_transport_controls_reaches_the_output_in_order`
- `an_idle_player_answers_a_command_immediately`

This pass adds the four seek/quiet-player tests named above, including one that
pins the dependency limitation in (5).

### 5. The MP4 box walk had no bound

Trigger: a file that is not MP4 but holds the `ftyp` marker in its first 64 KiB
is walked as boxes, and a box may be as short as its 8-byte header. Such a walk
could read a large file 8 bytes at a time, on the player thread, for every Play
and every prime.

Fix: `BOX_BUDGET` (100,000 boxes, shared across the recursion). Past it the walk
stops looking and the decoder decides — exactly what already happens for a file
the walk cannot parse, so no playable file is turned away. Test
`the_walk_stops_at_its_budget` asserts the budget stops the scan and that the
oversized table is still found with a real budget.

### 6. A large volume value was an error instead of "loud"

Trigger: desktop `player_set_volume(level: f64)` narrowed f64 to f32 before any
clamping, so a finite out-of-range value such as `1e300` became `f32::INFINITY`
and the player refused it as an invalid value (a user-visible error).

Fix: clamp in the f64 domain, then narrow. `NaN` still has no meaning and is
still refused by the mailbox. The MCP adapter's explicit range check is
unchanged.

## Measured and rejected

- **Per-sample `catch_unwind` in `PanicSafe`.** Measured in release on this
  checkout by pulling 26,460,040 samples from a 30 s AIFF ten times with and
  without the guard: plain 86.9 ms, panic-safe 109.2 ms. That is **+25.7% decode
  time (about 1.26× the CPU per sample), i.e. ~26% slower decoding — an
  execution-cost overhead, not a throughput gain.** The absolute cost is what
  matters: decoding runs at about 0.04% of one core in real time, so the guard
  costs roughly 0.01% of a core. Batching the guard would mean buffering inside
  the source chain and reporting shifted span lengths to rodio's queue, which is
  where gapless is decided. Rejected: real but negligible cost, disproportionate
  risk to the gapless path that #725 protection exists alongside.
- **Second and third file open per decode** (`check_sample_tables`,
  `AudioFile::open`, lofty duration probe). Rejected: a few microseconds against
  a decode that reads the whole file.
- **`Player::status()` clones the path per poll.** Rejected: one short string at
  the UI's ~3.3 Hz poll rate; an `Arc` snapshot would change a public return type
  for no measurable gain.
- **Coalescing repeated idempotent transport commands** (Stop, Stop). Rejected
  without a measurement showing a burst: it would blur the documented
  "accepted controls keep their order" contract.
- **Mailbox defects.** Re-read `mailbox.rs` against its stated invariants:
  coalescing only across trailing non-barrier `Seek`/`SetVolume`, replacement
  before the capacity check, `Busy` without losing an accepted control,
  `Closed` after sender drop, drain-before-disconnect, poison recovery on every
  lock, and absolute (not relative) seek/volume values — checked in
  `app/ui/js/player.js`, which sends the slider position, so keeping only the
  latest is correct. No defect found.
- **Queue defects.** Re-read the cancellation protocol: waker registered before
  the cancel check, `Waiting` clears it on drop, admission taken before the lane
  is touched (a rejected request cannot cancel an admitted preview), the worker
  owns its permit after the caller is dropped, and a cancelled waiter leaves
  tokio's FIFO lock queue so regular work is not held up. No defect found; two
  tests were added rather than a change.

## Unresolved and deferred, with the concrete next step

### 1. A device that stops pulling hangs the seek (High, source-confirmed)

Trigger: the output device stops consuming samples — unplugged, a sound server
that died, a suspended stream.

Evidence from the resolved source, not inference:

- `Player::try_seek` stores a `SeekOrder` and then waits on
  `feedback.recv()` with **no timeout** (`rodio-0.23.0/src/player.rs:238-257`).
  Its doc comment promises "this blocks between 0 and ~5 milliseconds"
  (`player.rs:222`), but that promise relies on the audio thread pulling.
- Feedback is sent from a `periodic_access` closure that only runs while the
  source chain is being polled, every 5 ms (`player.rs:130-158`). No pull, no
  feedback.
- The only device-failure signal is cpal's error callback, exposed as
  `DeviceSinkBuilder::with_error_callback` (`rodio-0.23.0/src/stream.rs:368`).
  On ALSA it runs on the stream thread inside the stream's error path
  (`cpal-0.18.2/src/host/alsa/mod.rs:913-960`), not on the player thread. It
  cannot unblock a `recv()` already waiting in that thread, and no code of ours
  runs there while it waits.

Consequence: one blocked seek also blocks Stop and every later command, because
this one thread serialises them. That is the worst property in this review: it
is the only path where an accepted control cannot run at all.

What was done, and what was not: no speculative dependency change, no per-seek
detached thread, no unbounded worker, no guessed timeout reported as success, no
callback-only "recovery" that cannot run while the thread is blocked. The
current code still calls the blocking API, so reliability is **not** certified.
A deterministic test now pins the behaviour at the seam:
`a_seek_the_output_never_answers_holds_up_every_later_command` parks a fake sink
inside `try_seek`, shows that Stop cannot reach the output while it is parked,
then releases it and shows the accepted controls run in order. That reproduces
the *consequence* deterministically; it does not reproduce a real device stall.

Why the dependency does not support a bounded fix: `Player` exposes no
non-blocking or bounded seek, no seek status, and no way to abandon a seek.
Every "recovery" that can run while the player thread is blocked would have to
come from another thread that cannot cancel the call.

Minimal design that would close it, needing a dependency or API decision and
real-device validation:

1. Wrap the decoder in a source owned by this crate that carries an
   `Arc<SeekRequest>`: target plus a one-shot acknowledgement. The player thread
   sets the target and returns — it never calls `Sink::try_seek`, so a stalled
   device cannot block it, and Stop still runs.
2. Service the request inside that source's `next()` on the audio thread (the
   same place rodio performs decoder seeks today), and send the result back.
3. Poll the acknowledgement on the following ticks and only then move the clock
   (or set `seek_refused`), keeping #190 honest. A never-acknowledged request is
   simply still pending; the track boundary or Stop drops it, so no timeout is
   guessed.
4. Costs: the seek is acknowledged up to one 200 ms tick later (the UI already
   updates the bar from a ~300 ms status poll, so the visible latency change is
   small but not zero), the seek request is bound to the track rather than the
   sink (removing the stale-order race in (3) as a side effect), and it must be
   checked on real hardware for gapless behaviour across a seek and for decoder
   seek time on slow media.

Closing it needs either that change with a device-loss hardware test, or a
rodio API with a bounded/non-blocking seek. Until then this stays High and open.

### 2. Device loss while a sink exists is invisible

`output_unavailable` is only recomputed on Play/Stop, so a removed device leaves
the bar showing a frozen track with no reason. Same fix as (1); the error
callback can only annotate, not recover, while the thread is blocked.

### 3. A next track that cannot be decoded is retried every poll

`wants_next` stays true, so the UI asks again every ~300 ms; each attempt
re-opens the file and writes a line to stderr. Deduplicating the refusal risks
never retrying a file on a network share that was asleep, so it needs a rate
limit rather than a cache — deferred pending evidence that it appears in
practice.

### 4. The 5 s prime lead can miss auto-advance

If the UI is unresponsive for the last five seconds of a track, the next source
is never appended. Documented design tradeoff (an appended source cannot be
taken back, which is what makes a mid-track Repeat change work).

### 5. A device stream is opened at launch and held for the session

Even if the user never plays anything, so `output_unavailable` can be reported
before the first Play. Opening it on demand would save one idle stream (and one
permission/usage indicator on some platforms) but changes what the player bar
says at startup, which is a UI-visible contract — deferred to a change that may
touch the interface.

## Verification

Commands (shared build lock, existing toolchain and caches from
`dev-env.sh`):

- `cargo test -p tagrex-player` — **41 passed, 0 failed** (24 before the first
  pass, 37 after it, 41 now). Stable across three consecutive runs and across
  four concurrent runs of the test binary at 16 threads.
- `cargo clippy -p tagrex-player --all-targets --all-features -- -D warnings` —
  clean.
- `cargo test -p tagrex --bin tagrex` — **69 passed, 3 ignored** (the ignored
  cases are the two D-Bus MPRIS tests and the GitHub release lookup, unchanged).
  Includes the two new `blocking` tests and the existing eight. This ran against
  the real `app/dist` built by the parent (`tauri.conf.json` `frontendDist:
  "dist"`), with no `TAURI_CONFIG` override and no UI source pointed at.
- `cargo clippy -p tagrex --all-targets --all-features -- -D warnings` — clean.
- `rustfmt` on the owned player source.

Red-green evidence (temporary edits, reverted before the runs above): removing
the status early return failed the allocation test with 5 allocations in the
one-second paused window; removing the boundary seek outcome failed the rebuild
test with 2 appends instead of 3.

## Changed files

- `crates/player/src/lib.rs` — output/sink seam, idle park, allocation-free
  unchanged status (`StatusKey`, shared track path), seek outcomes and
  boundary rebuild, 16 new playback tests and the fixtures plus the counting
  allocator they share.
- `crates/player/src/mp4.rs` — bounded box walk and its test.
- `app/src/blocking.rs` — two tests: waker fires for a parked waiter, and a
  superseded preview does not hold up a regular job.
- `app/src/main.rs` — `player_set_volume` clamps before narrowing.
- `docs/performance/deepseek-player-queue-review.md` — this report.

No other worker's file was edited; nothing was committed, pushed, restarted or
run against user media.

## Known limits

No real-device playback, gapless-audio, seek-latency or device-loss measurement
was made; those need hardware and a platform matrix. The wakeup and allocation
counts come from in-tree counters and a test-build counting allocator on this
machine, not from a profiler: they are event counts, not energy or CPU
measurements, and the allocation counter counts calls rather than bytes. The
player tests use an injected device and a fake sink, so they prove the loop's
logic, not that rodio plays a file on a given OS; the blocked-seek test proves
the consequence of a stalled sink, not that a given device stalls it. Saving
claims for CPU and energy are estimates derived from the measured wakeup and
allocation counts.

## Upstream route

Reusable backend fixes: curated upstream `tagrex/tagrex:main` — the idle park,
allocation-free unchanged status, seek postconditions and boundary rebuild, and
the `Output`/`Sink` seam are all platform-neutral Rust. The `Player` API is
unchanged (`PlayerStatus` keeps its baseline derives). The remaining High issue
in (1) is a dependency-level design change, not a drop-in fix, and should not be
proposed upstream as a reliability fix before it has hardware evidence. The
desktop adapter clamp belongs with the desktop layer, not the player crate.


## Final parent integration corrections

The expected source count used by seek is now the player's tracked queue length,
not a length read after the sink may already have advanced. A direct regression
checks that the next source is not sought as though it were the old source when
the boundary preceded the call. During-seek boundary tests remain in place.

The original MP4 budget implementation fell through to decoding on exhaustion.
Parent rejected that fallback: padding can hide the very oversized sample table
the pre-decode guard protects against. Exhaustion now produces an explicit
inspection-budget refusal, with a padded-table regression. This deliberately
refuses files that cannot be inspected within the limit rather than weakening
allocation protection. Earlier "decoder decides" claims for budget exhaustion
are superseded; ordinary non-MP4/malformed parsing behavior is otherwise retained.
