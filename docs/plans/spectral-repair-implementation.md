# Split Studio spectral repair

This branch starts at the 1.5.0 candidate `99fc6231609382924b5fe9d84d1807f84da52329`. The release runs separately. Native verification for this branch uses disposable stereo recordings and a dedicated app profile.

## Stage 1 — Selection and listening

- Keep the spectrogram central, with Select, Listen and Reduce together. Add full-time and full-frequency shapes, precise bounds, pointer and keyboard edits, an independent playhead gesture, and view fits and history. [RX interactive tools](https://docs.izotope.com/rx12/en/interactive-tools.html)
- Add persistent palettes, a fixed dBFS legend, contrast, waveform blend, distinct overlays, linked stereo lanes, detail presets, readouts, repair landmarks and reduced-motion behavior. Keep a bounded tile cache and separate drag feedback from texture rendering. [RX spectrogram and waveform](https://docs.izotope.com/rx12/en/spectrogram-waveform-display.html)
- Give the audition loop separate bounds. Support exact and contextual loops, phase-aligned Original, Repaired and Removed audio, stereo and channel monitor, band audition, strength trials, loop seams and return-on-stop. Use the same scale for source, repaired and difference images. [RX transport](https://docs.izotope.com/rx12/en/transport-controls---displays.html) · [RX preview and compare](https://docs.izotope.com/rx12/en/common-module-controls.html)

Gate this stage with keyboard and pointer fixtures, and run the integrated flow in a native webview. Use a long recording to check that viewing and drawing stay responsive.

## Stage 2 — Weighted repairs

- Extend the existing recipe with a bounded versioned mask. Rasterize brush strokes and lasso outlines, combine Add and Subtract in order, and feather time and frequency boundaries. Keep the legacy rectangle filter available for existing recipes.
- Use one source-coordinate implementation for playback, analysis and export. Keep frame alignment on seek and limit pre-render work to the audition range. Require synthetic verification for protected time gaps, adjacent tones, unchanged channels and preview/export parity.
- Group drawn regions as one repair. Use history to reselect repairs, rename, bypass, undo or repeat an edit. Keep the source and export format workflow intact. [RX spectral repair](https://docs.izotope.com/rx12/en/spectral-repair.html) · [RX history](https://docs.izotope.com/rx12/en/undo-history.html)

## Stage 3 — Review and reconstruction

- Keep tone, harmonic, hum, similar-event and click suggestions in review until the user selects or applies one. Show the search scope and the limits of time and frequency resolution. [RX Find Similar](https://docs.izotope.com/rx12/en/find-similar.html)
- Verify manually selected short click interpolation with exact PCM comparisons around the selection and percussion checks. For steady-noise reduction, require a learned noise-only interval, conservative gain and removed-signal monitoring. Audition both methods with real recordings before claiming quality across material. [RX Interpolate](https://docs.izotope.com/rx12/en/interpolate.html) · [RX spectral denoise](https://docs.izotope.com/rx12/en/spectral-de-noise.html)
- Prototype explicit donor replacement, short-gap fill and a nearby-repeat search on small regions, with stereo alignment and join tests. Validate context matching against steady noise and sustained audio before offering it.
- Treat de-clip, dense de-crackle and harmonic resynthesis as separate algorithm experiments. Use lightly clipped signals, challenging percussion and sustained stereo material for their first quality gates. Bandwidth reconstruction, dereverberation and stem separation require separate work.

Each completed stage needs focused checks and a combined candidate review. Local verification is the publication boundary for this branch.

## Local verification

- Linux Tauri and WebKitGTK opened a disposable 30-minute stereo FLAC. A scripted brush pass recorded 47 frame intervals, with a 16 ms median, 18 ms p95 and 30 ms maximum. The staged brush appeared in the Removed spectrogram and was undone. The Repair controls stayed keyboard reachable in a 460 × 405 window in light and dark themes.
- A disposable 12-second stereo WAV contained a left-channel click at frame 96,171. The native detector found that exact sample and returned no right-channel click. Context playback reported an active playhead. After interpolation, independent FFmpeg decoding of the exported FLAC found exactly one changed sample in the source's 16-bit precision. The WAV checksum and the 576,000-frame length matched.
- The workbench passed 115 checks, including legacy EQ and rectangle processing, adjacent edit isolation, high-rate rectangle coverage, and seek/export alignment. The Studio command and player tests covered draft compatibility, audition seams, channel routing and MP3 seek/loop. All 277 frontend unit tests passed.
- The combined WebKit pass reported 78 passes, one retry pass for recipe-menu focus, and one waveform-tooltip failure. Both timing failures also occurred on the untouched 1.5.0 candidate.

## Current limits

Find Similar scans the displayed excerpt. A probe spans at most 12 seconds. Very narrow brush and lasso strokes are limited by the FFT resolution, and the drawn feather halo is approximate.

The noise tool estimates a steady display floor. Denoise processing, donor and short-gap reconstruction, nearby-repeat matching and clipping detection are experiments outside the Repair controls. Dense de-crackle and harmonic resynthesis still need algorithm and quality work. Evidence uses synthetic signals. Subjective acceptance on real recordings and native macOS and Windows runs remain open.

## UX and performance follow-up

Time and Frequency are direct tools. Time spans the full source band. Frequency spans the visible window, the audition loop, or the full recording. At recording scale the main graph shows a labelled Source waveform and pauses spectral probes. Anchored zoom restores the selected spectral view as soon as the window is small enough. Sideways scrolling and middle drag pan. Page Up and Page Down move between landmarks from one overview tab stop.

The final independent review rated discovery, visual hierarchy and theming, keyboard and narrow layouts, and edit safety at 9/10 for the supported workflows. The transport review rated live feedback, playback continuity, and latency at 9/10 for the sampled path. The 420 × 320 Linux webview and both WebKit themes fit horizontally with all five shape tools in one row. Vertical scrolling still exposes lower controls at minimum height.

- Zooming from all 30 minutes of the native fixture into spectral detail measured 17 ms p95 across 37 frame intervals. A native brush pass also measured 17 ms p95.
- During a live brush edit, 127 consecutive native status samples reported active playback on the same transport. The retune settled by the 32 ms sample. This measures the transport status path. Buffered retunes use a 4 ms phase-aligned crossfade.
- A dense-mask fixture reduced ordinary playhead packets from 7,816 to 370 bytes. The graph projects a smooth playhead between authoritative updates and holds it when audio is buffering.
- The final gate passed 87 WebKit fixtures, 280 frontend unit tests and 253 Rust checks. Three heavy Rust tests remain opt-in in the standard suite. The separate CBR/VBR MP3 proof also passed. Svelte diagnostics, lint, and formatting passed.
