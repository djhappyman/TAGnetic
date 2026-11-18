# Native playlist loading and text conversion

`App::preview_playlist_load(playlist, allowed_roots, options)` resolves local
M3U/M3U8 entries into an ordered candidate file list. It does not replace the
live file list, discard staged changes, write audio or modify Undo history.
The UI can present the candidate and apply its order through the current file
workflow when UI integration is implemented.

`PlaylistLoadOptions` has an explicit `encoding` label, defaulting to UTF-8.
M3U8 requires UTF-8. M3U also accepts explicit UTF-16LE/BE or supported named
codepages, such as Windows-1252. There is no host-dependent ANSI guess.

Paths are relative to the playlist's directory unless absolute. Input order is
retained and duplicate canonical path references are reported and omitted after
their first occurrence. Empty lines and lines beginning with `#`, including
EXTINF comments, are ignored. Use `./#name.flac` for a filename beginning with
`#`. Spaces in path lines are significant. Backslashes are treated as portable
directory separators, not escapes. Windows drive/UNC references on non-Windows
systems are reported as unsupported rather than treated as relative filenames.

The result contains `paths` and structured `issues`, including physical line
number, original reference and reason: duplicate, missing, outside scope, unsafe,
unsupported reference/extension or unreadable. Unsupported extensions include
nested playlists. Media extension eligibility uses the same native scanner
filter as the app; resolving a filename does not certify its media contents.

The playlist itself is an explicitly chosen local input. Referenced files must
stay within the separately supplied approved roots. References cannot grant
access to other folders. Lexical root checks precede filesystem lookup, and
pinned directory reads reject final symlinks, special files and escaping parent
links. URLs and `file:` URIs are reported unsupported; no network fetch occurs.
HLS `#EXT-X-` directives reject the playlist as a streaming document. These are
deliberate local-file semantics, not an HLS or nested-playlist implementation.

The raw file and decoded text each have a 1 MiB limit. At most 100,000 entries and
256 approved roots are accepted. Estimated resolved state, including both path
copies in the output/deduplication index and issue storage, is limited to 64 MiB.
Entries with control characters or references longer than 4,096 bytes are
reported unsafe. A cancellation variant checks between roots, decode chunks and
entries; errors or cancellation return no partial candidate. Filesystem calls
themselves are not advertised as interruptible. No persistent playlist parse
cache is introduced for this explicit, bounded load operation.

Mp3tag documents M3U/M3U8, preserved load order and omitted duplicate references
in its [playlist guide](https://docs.mp3tag.de/playlists/). Tagnetic retains its
own explicit scope, read-only preview and current staging workflow.

## Shared strict text codec

`tagrex_core::text_codec` provides `TextEncoding::from_label`, `decode`, `encode`
and cancellation variants. Newline bytes/content are preserved; no newline or
whitespace cleanup is implicit. Existing text-file/sidecar reads use this codec
with their original input limits, BOM detection and NUL rejection.

UTF-8 and UTF-16LE/BE support an optional matching BOM. Conflicting BOM/encoding,
malformed UTF-8, odd UTF-16 input, unpaired surrogates, ambiguous `ANSI`/`UTF-16`
labels and unsupported encodings fail. UTF-16 output is explicitly emitted as
code units with the selected byte order. The dependency's UTF-16 output mapping
is not used. Named legacy codepages use strict streaming conversion: unmappable
characters fail instead of becoming replacement characters or numeric entities.
Codepage labels follow encoding_rs's documented label mappings.

Input and output have a global 64 MiB ceiling, with a caller-selectable smaller
output limit. Conversion uses 4 KiB scratch chunks, and the byte limit includes
any BOM. Cancellation returns no partial result. These bounds describe text
input/output, not a whole-process RSS guarantee.

The implementation uses the locked encoding_rs 0.8.44 strict conversion APIs.
See its [first-party API/source documentation](https://docs.rs/encoding_rs/0.8.44/encoding_rs/).
This is an encoding foundation for reports and spreadsheet imports; native
export encoding selection, guarded append and report templates remain separate
unfinished work. No graphical controls are added by this checkpoint.
