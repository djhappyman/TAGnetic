# Independent descriptor I/O compatibility review

Status: checkpoint ready for integration on Linux. No remaining scoped defect
found in the frozen correction. Actual macOS hosted execution remains required.

## Diagnosis and ranked findings

**P2: Darwin descriptor aliases share cursor state, while model I/O expected a
fresh open; artificial names also discarded logical parser hints.** The prior
macOS job `135098539214` on `96480a82` recorded 54 command failures, predominantly
`BackendWrite(FileEncodingError { format: None })`, with a block-conversion stale
failure. Source evidence is preserved in
`/mnt/Games/Development/tagrex/tmp/flac-ci-macos-96480a82.log`.

Apple's [XNU descriptor filesystem implementation](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/miscfs/devfs/devfs_fdesc_support.c)
routes `/dev/fd` opens through descriptor duplication (`fdesc_open`, lines
368–392). Apple's [duplicate API documentation](https://developer.apple.com/documentation/system/filedescriptor/duplicate%28as%3Aretryoninterrupt%3A%29)
confirms that duplicated references share their file position. This makes the
shared-cursor explanation concrete rather than assuming Linux `/proc` behavior
on Darwin.

The installed Lofty 0.25.5 package identifies source commit
`59a3f5ad28f982102fdfaf8f458efd9eafb38747`. Its
[path writer](https://raw.githubusercontent.com/Serial-ATA/lofty-rs/59a3f5ad28f982102fdfaf8f458efd9eafb38747/lofty/src/tag/tag_ext.rs)
opens the supplied name and passes the handle to
[VerifiedFile](https://raw.githubusercontent.com/Serial-ATA/lofty-rs/59a3f5ad28f982102fdfaf8f458efd9eafb38747/lofty/src/util/io.rs),
which probes from the current cursor. A prior read leaving a duplicated
descriptor at EOF therefore explains the observed unknown-format write failures.
The model's extension-directed tolerant probe also needs the original filename.

An independent component probe reproduces the underlying contract on Linux:
duplicate a held File positioned at EOF, call Lofty's writer, observe failure
with unchanged bytes, rewind and observe a successful write. This reproduces
shared-description semantics; it is not a claim that Linux descriptor pseudo-path
opens emulate the Darwin kernel.

**P2 review iteration: unrelated peer requests must not inherit an outer
Executor's roots.** The author's retained App test exposed this in the first
strict-open candidate. Source review agreed, and a new independent public probe
now verifies peer reads and nested Apply/Undo across separate roots. The final
model helper uses strict authority only for an exact matching metadata binding.
Independent public requests retain their original standalone behavior. A failure
after a matching binding never falls back to an unchecked path.

## Reviewed correction

The parser retains its logical path. Each matching model read/write opens a fresh
file description through existing held-parent authority and compares it to the
held metadata identity before the backend receives it. Writers use Lofty's
File-based API. The final source binding check remains. Exact held candidate
sources take precedence over the original metadata binding and keep their bounded
repair behavior.

The source audit found no new whole-file copying workaround, format-preserving
hardlink workaround, new dependency or expanded native-platform support. Existing
model whole-file compatibility repairs retain their prior behavior and exact-read
limits. There are no remaining production model `save_to_path` calls or raw opens
bypassing the contextual seam. The command test sources are unchanged by this
correction; the original macOS failures were not hidden with changed assertions.

The graph project `tagrex-backend-readiness`, generation
`2026-11-18T12:48:55Z`, indexes the parked source checkout. Exact publication paths
report `outside_project`; review uses actual source/diffs and the installed
dependency source. The reviewer owns only the new integration test and this
report, with no production edits.

## Independent checks

The eleven new integration probes cover:

- Shared-File EOF behavior and untouched bytes on the failed library probe.
- Tolerant MP3 extension fallback through public Executor preflight/write/Undo.
- Mixed MP3, FLAC and WAV tags/artwork Apply/Undo, with independent encoded MP3/
  FLAC audio-tail and WAV data-chunk comparisons.
- Repeated held-source reads from EOF with absent logical labels.
- Whole ID3 block removal/restoration and unchanged encoded audio.
- Foreign file/parent replacement without writes to the foreign object.
- Error, early operation unwind and late exact-reader callback unwind cleanup.
- Same-inode hardlink aliases retaining logical hints and file identity.
- Independent peer reads and nested Apply/Undo across separate roots.

Windows deliberately denies some replacement attempts through held handles. The
new replacement probe accepts that specific PermissionDenied outcome only after
verifying the original file remains present and unchanged, then completes and
undoes the legitimate operation. Unix replacement checks still require refusal
and exact preservation of both original and foreign bytes. Parent-directory
replacement is a Unix-specific probe. These are newly added tests, not weakened
versions of the original command assertions.

Final source SHA-256 values, checked after verification:

| File | SHA-256 |
| --- | --- |
| `crates/core/src/model.rs` | `81e59202aa1c1a47003acdec48a92b0ed308d3e732b7dc4aa1f2f7d6f4f7c749` |
| `crates/core/src/model/exact/source.rs` | `fdba077ad1b37796e4f4d163a01a74ff573e82e362c79212dfc23e76e68204ab` |
| `crates/core/src/plan.rs` | `8cc196f1202bceb2564cce0000da5cae06de8f102617defe2ede8cdfe0eca5be` |
| `crates/core/src/plan_paths.rs` | `d6e0de8cfd1da5a86c707767f30f239785d55cf1ec423069c60337b886618473` |
| `crates/core/src/plan_descriptor_io_tests.rs` | `9db5294af0bf7e016204885cb49e8a836df573b5704327135539559ff1920a22` |

Using `source /mnt/Games/Development/tagrex/dev-env.sh`:

| Check | Result |
| --- | --- |
| `cargo test -p tagrex-core --test descriptor_io_independent` | 11 passed, 0 failed/ignored |
| `cargo test -p tagrex-core --lib plan::descriptor_io_tests` | 4 passed |
| `cargo test -p tagrex-core --test held_exact_read_independent --test exact_source_authority_fresh_review` | 8 + 8 passed |
| `cargo clippy -p tagrex-core --test descriptor_io_independent -- -D warnings` | Passed |
| `rustfmt --check --edition 2021` on the owned integration test | Passed |

That is 31 passing checks on the frozen source with default test concurrency.
Logs under `/mnt/Games/Development/tagrex/tmp` are
`flac-descriptor-independent-final-verified{,-clippy}.log`,
`flac-descriptor-independent-retained-unit.log` and
`flac-descriptor-independent-retained-held.log`. No task-owned fixture directory
remains. Production files were not changed by this reviewer, and no live media,
profile, application runtime or user Undo history was used.

The initial independent integration compile already included the author's
correction; it is not presented as a Linux production red-to-green run. The old
macOS job supplies the real platform failure, and Linux tests reproduce its
library/cursor mechanism. macOS and Windows hosted results on the exact new head
are still separate requirements. This review does not prove power-loss behavior,
all formats, RSS ceilings or macOS native protected FLAC mutation. The broader
roadmap remains paused after this slice; eligible shared backend work follows the
compatible upstream foundation route while preserving upstream inline images and
its stable waveform API.
