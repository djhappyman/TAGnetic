# Playback and preview-job readiness checkpoint

## Parent release-profile measurement

After the ownership/router corrections, the parent explicitly ran
`cargo test -p tagrex-player --release decoded_source_control_throughput_probe -- --ignored --nocapture`
with the existing toolchain/cache. The one selected manual test passed. For the
same 12-second synthetic AIFF, seven warm samples and exact sample/count checksum,
the real converter median/max were 9.307/9.496 ms and the controlled pipeline
median/max were 15.575/15.805 ms. These are optimized component CPU timings,
excluding physical output, callback deadlines, UI/IPC and cold-disk behaviour.
The control pipeline has measured overhead; no throughput speedup or physical
device latency guarantee is inferred. The independent ownership recheck found
no further confirmed production defect in its scoped probes; overall backend,
device lifecycle and platform gates remain open.

Date: 18 November 2026. Base `9da71249`, implementation checkout
`/mnt/Games/Development/tagrex/native-field-foundations`. This is a scoped local
backend checkpoint, awaiting parent integration and independent review. It is
not full backend, device, platform or enterprise certification.

## Scope and ownership

Owned changes: `crates/player/src/lib.rs`, new `controlled.rs`, `fixed.rs`, `router.rs` and
`player/tests/control_span_transitions.rs`,
`player/tests/seek_ownership_stress.rs`,
`app/src/blocking.rs` and only preview dispatch in `app/src/main.rs`. No frontend
source changed. The existing dependencies, environment and shared build cache
were reused. The running application, profile, real media and Undo history were
not opened or mutated. Generated media are disposable synthetic AIFFs and FFmpeg-generated MP3, FLAC
and M4A files. They have no user-media provenance.

Reusable player and desktop Rust changes are eligible for curated upstream
`tagrex/tagrex:main`, with the normal fork and hosted gates. No commit, push, PR,
merge or release is claimed by this worker.

## Implemented behaviour

The production actor no longer calls `RodioSink::try_seek`. Locked Rodio 0.23.0
implements that call with an unbounded feedback `recv()`; the callback that
answers depends on output consumption. Instead each decoded source owns one
replaceable seek slot and acknowledgements. The actor submits and polls; no
thread is created for a request or timeout. Requests belong to the source and
cannot move into a primed next track. The older cross-track rebuild workaround
is removed, retaining the primed next source at a seek boundary.

Acknowledgements update the visible clock only while the same tracked source
remains current. The additive serialized `PlayerStatus.seek_pending` reports the
wait. A two-second actor deadline refuses an unanswered request without claiming
success. An unconsumed cancelled request does not alter the decoder. If a decoder
call was already running, its eventual return terminates that old source rather
than resuming audio at the refused position. The actor never holds a request
mutex across decoder I/O. Stop cancels its pending request and remains available;
this does not claim to interrupt a blocked kernel/decoder call or restore a lost
physical device.

The first candidate's fixed `512 * channels` source span was incorrect. The
independent reviewer reproduced wrong channel and rate conversion using the
unchanged real Rodio converter canaries. Those canaries remain unchanged and now
pass. A generic transparent source adapter retains coherent native span metadata
across span/channels/rate getter calls; controls cannot change that snapshot
between getter calls. Paused silence counts toward the exposed span, keeping
resumed real samples inside the native format boundary.

Production takes the stronger path: `fixed::Fixed` owns Rodio's public rate and
channel converters beneath the source controls, using the physical router
format. Native spans are preserved within that conversion. A successful seek
rebuilds the interpolation/channel-repeat buffers before any sought sample is
emitted. Refused seeks retain cached samples. Controls therefore expose a stable
physical format and seek at output-frame boundaries even for long or unknown
native spans. They do not wait for a large native span or introduce a supported
format refusal to avoid the converter defect. Only the test-only generic native
span constructor retains a refusal for an unsafe unbounded advertised span.

Pause is source-local: complete silent interleaved frames retain decoder position
while allowing paused seeks to be acknowledged. Pause changes take effect on a
frame boundary, avoiding left/right sample misalignment. A source boundary while
paused now freezes the new clock too; the prior loop incorrectly restarted a
running clock with a paused status.

Fresh Rodio sinks previously connected individually to its unbounded mixer
channel. Without device consumption, every fresh stopped sink could remain
retained. One permanent mixer router now has a single replaceable pending sink,
plus the source currently held by the consumer. Replacements switch at output
frame boundaries. Superseded pending sources are dropped outside the router lock.
A sink may contain the current and primed next track; the two-sink bound is not a
two-decoder or process-wide RSS claim.

Independent latest lanes now cover rename, filename/text tag import, transform,
move and transformation-group previews. Ordinary write jobs retain FIFO admission
and their serialization permit even if the caller disconnects. Latest-preview
caller loss sets cancellation while the running worker retains its permit until
exit. Queue failures include stable `operation.queue_busy`,
`operation.superseded` and `operation.worker_failed` codes with retryability
metadata through the existing error DTO and English fallback.

**Legacy-running-preview progress:** latest workers now enter the parent's
`with_preview_cancellation` synchronous worker-local context. Native metadata-read
boundaries check cancellation even inside legacy preview methods. A queue test
supersedes a real worker and verifies `App::read_track` returns cancellation before
a deliberately missing path is read. Ordinary mutation jobs bypass this context.
This supplements queued cancellation and stale-result refusal; it does not
interrupt a syscall or parser already executing, nor claim cancellation points
inside every legacy CPU-only loop. Parent context/command changes and this queue
integration require combined verification.

## Adversarial verification

Commands used the existing `source /mnt/Games/Development/tagrex/dev-env.sh`:

```sh
npm run build
cargo test -p tagrex-player --all-features
cargo test -p tagrex blocking::tests
cargo test -p tagrex-player decoded_source_control_throughput_probe -- --ignored --nocapture
cargo test -p tagrex-player actual_decoders_acknowledge_seeks_after_converter_metadata_and_midstream_reads -- --ignored --nocapture
cargo clippy -p tagrex-player -p tagrex --all-targets --all-features -- -D warnings
rustfmt --edition 2021 --config skip_children=true --check crates/player/src/lib.rs crates/player/src/controlled.rs crates/player/src/fixed.rs crates/player/src/router.rs crates/player/tests/control_span_transitions.rs app/src/main.rs app/src/blocking.rs
git diff --check
```

Observed player library suite after the ownership correction: **57 passed, zero failed, two ignored**. Both
manual probes were explicitly run and passed. Independent adversarial binary:
**8 passed**, including the two unchanged review canaries. Additional transition
binary: **16 passed**, including production fixed-format and seek/reset tests.
Each integration binary includes the six original controlled-source unit tests;
these repeated executions are not six new distinct findings per binary.
Fresh independent fixed-pipeline recheck binary: **11 passed**, including the
unchanged rate/channel matrix and both formerly failing canaries. Ownership stress
binary: **12 passed**, including six repeated original wrapper tests. Desktop
queue scope: **13 passed, zero failed**. Production frontend build and
scoped all-target, all-feature Clippy passed. Parent must run the final combined
gate after other workstreams and fresh independent review corrections.

Coverage includes:

- 100,000 seeks without source consumption retain only the final requested
  target; exactly one decoder seek occurs when consumption starts.
- 200,000 deterministic randomized pause/resume sample decisions preserve every
  emitted real sample and stereo alignment.
- A decoder deliberately blocked inside seek does not hold the request mutex:
  cancellation returns promptly, the test releases and joins the consumer, and
  no late sought sample is emitted.
- Refused seeks preserve the source position, and dropped sources reject new
  seek requests. Unconsumed cancellation does not affect another source.
- **Actual Rodio player/queue, actual AIFF decoder and router without hardware:**
  100,000 seek/volume bursts while nothing consumes output, followed by Stop and
  replay; Stop is observed within a one-second test bound. Paused seek remains
  pending before consumption, then real decoder acknowledgement sets the frozen
  target after the consumer resumes. This is not merely a fake sink test.
- A dependency positive control connects 128 old-style fresh sinks to an
  unconsumed real Rodio mixer and observes all 128 retained tracked sources.
  The router stress makes 20,000 replacements and observes at most an active
  and one pending sink's tracked source; all tracked sources drop afterwards.
- Source-ending and delayed acknowledgement after Stop/replay do not move the
  new track's clock. Unanswered seeks produce refusal at the deadline.
- A paused-boundary test **failed before the fix**: paused position advanced
  from approximately zero to 0.200 seconds. It passes with the pause preserved
  after the clock restart. The failure was not hidden or weakened.
- Dropped running preview caller sets cancellation but keeps admission and
  serialization until worker exit. A full queue returns a stable retryable
  error without cancelling previously accepted work. Existing independent-lane,
  FIFO, 1,000-obsolete-preview, panic/error-release tests pass.

## Converter-correction and actual-adapter gates

`control_span_transitions` compares exact sample vectors/counts with the real
Rodio converter, including pause/resume within an old span followed by both a
channel and rate change, requests between metadata getters, seek into a new
format, fixed-output seek rebuilding resampler buffers, refused seek preserving
prefetched samples, and source exhaustion while paused. The production pipeline
also handles a deterministic 10-second 8 kHz native span and a native `None` span:
normal and paused seeks acknowledge within one two-channel output frame.

The explicit actual-decoder probe generates four-second sine fixtures with
FFmpeg (`libmp3lame`, `flac`, `aac`, `pcm_s16be`). It constructs the actual production
fixed-output control pipeline, queries metadata, consumes 138 output samples,
then seeks. MP3, FLAC, M4A and AIFF all acknowledged after **one output sample**;
the subsequent paused seek acknowledged after **two**. Native initial spans were
47, 1024, 1024 and 1152 samples respectively. Output is stereo 48 kHz. This
establishes that the fix does not merely seek before metadata is cached, or
refuse previously supported common adapters. It is synthetic decoder evidence,
not all-variant or hardware evidence.

## Throughput scope and costs

The corrected manual probe compares the bare real Rodio converter with the full
production fixed-output controlled source. Both decode the same 12-second
synthetic AIFF to **stereo 48 kHz**, seven warm samples, calculating the same sample
checksum/count. All counts and checksums agree. On the existing Linux development
profile (own crate unoptimized; dependencies retain their configured optimizations),
the real-converter median was **63.394 ms**, maximum **63.917 ms**; the controlled
pipeline median was **128.147 ms**, maximum **129.564 ms**. These are component
costs, not device latency or release-profile CPU measurements. The added control
work is measured and is not claimed as a playback throughput speedup.

Earlier 16.250/25.085 ms figures measured decoder-only versus the first controlled
wrapper, without equivalent stereo/rate conversion. That implementation also
failed the independent span canaries. Those superseded measurements must not be
used as the current production performance evidence.

## Fresh ownership and router recheck correction

The independent fixed-output recheck found two further real defects. Its
unchanged test run was reproduced before edits: one converter-matrix test passed,
while the in-flight ownership and active-source destructor tests failed. The
ownership failure returned a sample from an old two-second target after the
latest ticket had been cancelled. The router failure blocked another control
replacement for the test's 200 ms interval while the old source's Drop waited.
All threads were released and joined; no stalled test worker was abandoned.

The source now owns seek resolution across actor ticket handoffs. One latest weak
ticket, one pending slot and the consumer's current ticket replace per-request
ownership that could disappear when the actor dropped its previous handle.
Ordinary supersession is explicitly different from terminal cancellation:

- A newer request marks the old request superseded, including one already inside
  decoder I/O. It does not terminate a usable track merely because a seek was
  replaced. At most two decoder calls are attempted in one consumer turn; if
  more supersessions arrive, unresolved moved audio is silent until the newest
  request can be resolved on a later output frame.
- A latest success remains playable and can be sought again. Only a current
  completed result can update the actor clock. Actor acceptance releases ownership
  of that adopted position, so cancelling a later unconsumed request can preserve
  the acknowledged original audio.
- A latest timeout or Stop invalidates any earlier entered, unaccepted decoder
  work in that source. Late return cannot release old-position samples or clock
  updates. The invalidation cannot be overwritten by another request; fresh Play
  uses an independent source and ownership group.
- If an earlier superseded successful seek already moved the decoder and the
  latest seek refuses, there is no verified old position to resume. The ambiguous
  source ends rather than emitting the obsolete target. This fail-closed case is
  distinct from ordinary successful seek coalescing. A failure with no unaccepted
  earlier move retains the existing decoder/converter buffers.

The router now takes the pending value in its own inner scope and releases that
mutex before replacing/dropping the old active output. Its active and latest
pending slots remain bounded. Destruction can still take time on the thread
performing Drop, and a source currently being destroyed may coexist transiently
with the two slots; this is not an unbounded retirement queue or a hard real-time
destructor guarantee.

New evidence includes 100,000 replacements while one decoder seek is blocked
(only the old and final target are actually sought), repeated successive blocked
seeks with bounded silence between turns, newest success/refusal/terminal cancel,
a completed-but-not-yet-accepted old result, and an accepted position followed by
an unconsumed cancellation. Actor tests drive actual ticket handoff/deadline and
Stop/replay ordering with the production fixed/control path behind a deterministic
output adapter. They verify no obsolete sample and no late clock change; the
fresh replay source starts at its own position. The unchanged real-converter
matrix still compares exact samples/counts for 24 physical rate/channel pairs.

Final local commands after these corrections: `cargo test -p tagrex-player
--all-features`, both explicit manual probes, `cargo clippy -p tagrex-player -p
tagrex --all-targets --all-features -- -D warnings`, owned-file formatting and
`git diff --check` passed. An intermediate Clippy run correctly failed while
parent core helpers were still being wired; no foreign edits were suppressed.
Parent owns the combined gate and the next independent recheck.

## Remaining gates and limits

Physical device loss/reconnection, platform sound stacks, audible/gapless quality
and Windows/macOS runtime checks have not run. Source seeks and decoder opens may
still block their executing thread in filesystem/library calls; no false claim
of forcible interruption is made. Device retry rate limiting and a complete
runtime output-loss lifecycle remain separate work. Legacy CPU-only loops and in-progress parser/syscall cancellation
remain limited as described above. Global aggregate byte admission
belongs to the resource workstream. Full UI exposure must retain these limits.

Discovery used Tier 2 `tagrex-personal-app`, generation
`2026-11-13T16:10:57Z`, rooted at the other checkout. The graph was stale; exact
coverage checks mark player/app sources changed and new modules missing. Source
was read at the implementation checkout, including the locked Rodio source.
Graph snippets were used for discovery only, not current-source proof.
