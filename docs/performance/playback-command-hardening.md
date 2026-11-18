# Playback command admission

Playback has a bounded 64-command mailbox. Repeated trailing seek/volume updates
keep only the latest value of each kind. Play, Stop, Pause, Resume and SetNext
form ordering barriers; state updates never move across them. Adjacent SetNext
requests keep the latest intended next track. Accepted transport controls retain
FIFO order. A full transport queue returns an explicit busy error through native
desktop and connected-agent adapters, without blocking the UI thread or silently
discarding an accepted control. Closed workers also return an explicit error.

NaN, infinity and seek values outside Duration's range are refused before they
can reach the audio thread. Negative seeks and volume ranges retain their normal
clamping behavior. Queued path storage has a per-command bound. Sender shutdown
drains accepted commands before disconnecting; device retry, gapless decoding
and the existing playback clock remain the existing implementation.

Verification includes a 100,000-update seek/volume burst retaining two commands,
transport barrier ordering, capacity rejection/retry, invalid numbers and
shutdown. The existing device-free retry/clock and decoder tests passed. This
does not measure real-device latency or establish a production-optimal queue
size. Platform/audio hardware verification remains separate. Public Rust player
methods now return Result; every in-repository production adapter handles it.

Route: reusable player/desktop Rust changes for curated upstream `main`, with
public API adaptation required when curating. The running handoff app is still
the earlier verified build and was not restarted for this checkpoint.
