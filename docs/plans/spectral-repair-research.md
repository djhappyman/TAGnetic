# Split Studio spectral repair research

## Direction

Keep the spectrogram as the working surface. The repeatable job is to spot a fault,
select it, hear it on its own and in context, preview a fix, and keep it as an
editable repair. A click on a vinyl rip, a whistle in a crowd recording, and hiss
during a quiet tape intro should all use this loop. Keep Tagnetic’s existing
toolbar, theme and export workflow.

Tagnetic 1.5 already has a zoomable spectrogram, rectangular selection, individual
channel views and targeting, waveform overlay, source and treated views,
time-range and band audition, editable bounds and saved attenuation areas. An
area currently describes a single time and frequency rectangle. Rust applies a
band attenuation filter inside the time bounds with a short edge ramp. That
supports spot reductions today. Irregular shapes and context-based repair need
a processor that can distinguish individual time-frequency cells.

## Selection and editing

These interaction ideas are grounded in the RX editor's selection controls.
Shortcut suggestions should fit Tagnetic’s existing bindings, including **F**
to fit the current selection. [RX 12 Interactive Tools](https://docs.izotope.com/rx12/en/interactive-tools.html)

| Idea | What it does in Tagnetic | Fit |
| --- | --- | --- |
| Rectangle | Keep it the default for small, isolated blemishes. Make the four resize corners easy to hit. | Existing, polish |
| Time stripe | Drag across a pop to select the whole audible range for that instant. | Selection UI, needs a broadband processing path when applying |
| Frequency stripe | Mark a stable tone across the current loop or selected track. Make that time scope explicit. | Fits existing attenuation |
| Brush | Paint over a curved squeak or moving whistle and show the footprint under the cursor. | Weighted mask |
| Adjustable brush | Change width quickly with a wheel modifier. Give time and frequency width separate advanced controls. | Selection UI and weighted mask |
| Lasso | Outline an irregular noise without covering a nearby note or transient. | Weighted mask |
| Add to selection | Shift-drag for several clicks or harmonics under one repair. Preserve gaps between regions. | Selection model, multi-area processing |
| Subtract from selection | Alt/Option-drag to protect an important note inside the target. | Weighted mask or disjoint geometric regions |
| Shift edge | Keep current precision resizing when the pointer starts on an existing handle. Restrict Shift-add to new selection gestures. | Fits current editor |
| Move selection | Drag inside an existing selection. Keep the borders as resize handles. | Existing, polish |
| Move playhead without clearing | Use a modifier-click in the graph or click the ruler. | UI and transport |
| Exact bounds | Enter time, duration, low and high frequencies. Support keyboard nudging and smaller steps when zoomed in. | Existing, polish |
| Feather edges | Show a solid core and lighter edge. Start with the existing time ramp and expose time and frequency widths with weighted masks. | Time first, two dimensions after mask work |
| Harmonic selection | Select a fundamental and suggest a stack of partials. Let each proposed tooth be unticked. | Bounded analysis, works for steady tones |
| Tone wand | Snap a click to a nearby narrow ridge. Mark uncertain suggestions and allow audition first. | Bounded analysis, depends on local spectral resolution |
| Find similar | Use one click or chirp as the seed. Show candidate hits inside the current track before applying anything. | Later detector, keep review explicit |
| Repair history | Clicking a repair returns to its time, band and channel. Undo should restore selection when practical. | Fits current edit history with selection metadata |
| Per-repair bypass and rename | Compare a single edit and recognise it later as “crowd whistle” or “left-channel pop.” | Repair model and preview state |
| Group repairs | Treat one brush/lasso operation or one harmonic stack as one undo step. | Repair model |
| Repeat last action | Apply the last attenuation amount to a new selection with a shortcut. Keep each action undoable. | Fits current attenuation |

[RX 12 Find Similar](https://docs.izotope.com/rx12/en/find-similar.html) · [RX 12 Undo History](https://docs.izotope.com/rx12/en/undo-history.html)

## Looping, preview and comparison

| Idea | What it does in Tagnetic |
| --- | --- |
| Loop Selection and Loop With Context | Use the exact time range to diagnose the fault or include a short lead-in and tail to judge the edit against the music. |
| Separate loop edges | Adjust the audition loop without changing the repair rectangle. |
| Stay in phase during A/B | Switch between original and treated at the same place within playback. Use the same monitor gain so a level change does not bias the decision. |
| Compare two repair strengths | Try a light and stronger pass on the same loop without creating two history entries. |
| Audition the band | Keep the current band-only preview visible next to loop controls while a region is selected. |
| Hear removed audio | Play the time-aligned difference between before and after. Musical content in that monitor is a reason to reduce the strength or tighten the selection. |
| Preview the combined result | One action to leave isolated audition and hear the whole mix with every active repair. |
| Audition in stereo or one channel | Separate what you monitor from which channel the edit will affect. |
| Loop seam handling | Smooth playback seams where appropriate. Offer a hard seam when diagnosing an actual click at a boundary. |
| Preview without closing tools | Leave a selection and its controls intact while moving between the canvas and the tool rail. |
| Return on stop | Optionally return to the loop start or the point where audition began. |
| Follow or pin the view | Keep the current problem on screen while auditioning nearby. Follow playback is useful for scanning long files. |
| Context pre-render | Buffer only the affected excerpt for heavier Rust processing. Keep the previous sound playable and prevent stale work from replacing a newer preview. |
| Before, after and difference image | Keep the same axes and dB colour scale when switching views so changes in colour mean changes in audio. |

[RX 12 Transport Controls](https://docs.izotope.com/rx12/en/transport-controls---displays.html) · [RX 12 Preview and Compare](https://docs.izotope.com/rx12/en/common-module-controls.html)

## Canvas, colour and responsiveness

| Idea | What it does in Tagnetic |
| --- | --- |
| Warm spectrogram palette | Map quiet-to-loud energy from dark through amber into pale highlights. Use luminance as the main cue so intensity is readable without relying on hue. |
| Neutral and colour-vision-friendly palettes | Offer alternatives with a consistent increase in brightness. Retain the user’s chosen palette per window. |
| Fixed dB colour legend | Show what brightness represents. Allow a compact floor and contrast adjustment with reset. |
| Clear independent overlays | Theme accent for the active selection, quiet dashed outline for a saved repair, a distinct playhead, and loop brackets along the time ruler. Use shapes as well as colour. |
| Transparent selection | Keep fine musical detail visible under the selection. Show a soft ear/preview band when auditioning. |
| Waveform opacity | Let the user blend the waveform continuously over the spectrogram. Keep it as a separate overlay and make clipping easy to inspect in waveform mode. |
| Linked stereo lanes | Stack left and right with the same clock, scale and colour reference. Keep a fast Combined toggle. |
| Detail presets | Balanced for navigation, Transients for short clicks, and Tones for closely spaced hum and harmonics. Time and frequency detail trade off. |
| Pointer readout | Display time, frequency and approximate intensity on a quiet crosshair. Offer note names for steady tonal work. |
| Independent axis zoom | Keep pointer-anchored zoom and middle-drag. Add ruler zoom, fit time only, fit frequency only and zoom back. |
| Overview and landmarks | Reuse Split Studio cuts and add repair marks. Click a repair or cut to jump directly to it. |
| Fast drag feedback | Draw the selection, playhead and loop separately from the spectrogram pixels. Coalesce pointer input to display frames. |
| Coarse-to-fine spectrogram | Move immediately on cached tiles while zooming and panning, then refine settled views. Keep the cache bounded. |
| Smooth but honest scaling | Avoid auto-ranging while comparing. Add a deliberate “fit contrast to view” action for very quiet recordings. |
| Reduced motion and keyboard access | Offer an exact keyboard route to select, resize, audition and apply. Keep shortcuts and contextual actions discoverable. |

[RX 12 Spectrogram and Waveform](https://docs.izotope.com/rx12/en/spectrogram-waveform-display.html) · [RX 12 Interactive Tools](https://docs.izotope.com/rx12/en/interactive-tools.html)

## Audio tools and what can be delivered faithfully

| Tool | Where it fits | Implementation and acceptance |
| --- | --- | --- |
| Paint down selected noise | Whistles, handling noise, a short hiss burst or a squeak that moves in frequency. Highest return. | A weighted time-frequency mask in Rust. Rectangles, lasso and brush must affect their actual shapes. Test isolated and nearby tones, the untouched channel and time/frequency edges. |
| Short click interpolation | A clearly visible isolated vinyl tick or digital click. High return, narrow initial scope. | Limit to short events, blend into adjacent PCM, and verify that an intended snare is preserved. |
| Click finder and review | Lots of short ticks on a record. | Suggest candidates first. Add sensitivity, audition, Accept/Skip and undo. Evaluate false positives on percussion. |
| Learn a noise profile | Steady tape hiss or constant room noise. | Learn from a noise-only selection, use modest reduction and smoothing, offer removed-audio preview. Listen for musical noise and lost detail. |
| Detect hum and harmonics | Identify where mains hum sits, including recordings slightly off 50/60 Hz. | Reuse existing hum/notch controls. Suggest fundamental and harmonics and allow selection, audition and manual correction. |
| Reduce a low thump | Local handling or mic plosive. | A low-band selection and feathered attenuation cover basic cases. Use a dedicated detector later if needed. |
| Fill a very short gap | A dropout with steady audio on either side. | Small offline reconstruction with a clearly shown context region and a preview. Test joins, stereo alignment and nearby notes. |
| Borrow a nearby passage | Repeating beats where one hit was damaged. | Start with an explicit donor region, then try local pattern matching. Keep the source overlay and audition available. |
| Match surrounding background | Reduce an isolated sound toward nearby background rather than creating a conspicuously quiet patch. | Context-aware attenuation is more work than gain reduction. Let the user hear the context and weight before/after only once the processor actually uses it. |
| De-clip | Flat-topped peaks from overloaded transfers. | R&D. Distinguish clipping detection from waveform reconstruction. Test lightly clipped signals first. |
| Dense de-crackle | A record with many closely spaced defects. | R&D after single-click repair. Evaluate against textures and transients before offering a whole-track setting. |
| Harmonic resynthesis | Damaged sustained notes, chords or vibrato. | Longer-term DSP research. Establish stereo coherence and sound quality on difficult material before exposing it. |
| Restore missing bandwidth, remove reverb or separate stems | Heavier restoration outside the first Repair update. | Separate R&D projects with different quality, CPU and preview requirements. |

[RX 12 Spectral Repair](https://docs.izotope.com/rx12/en/spectral-repair.html) · [RX 12 Interpolate](https://docs.izotope.com/rx12/en/interpolate.html) · [RX 12 Spectral De-noise](https://docs.izotope.com/rx12/en/spectral-de-noise.html) · [RX 12 feature guide](https://www.izotope.com/en/products/rx/features)

## Suggested order

1. **Make the existing Repair canvas quicker to use.** Improve palettes, the
   legend and overlays. Add time/frequency stripes for selection and audition,
   a persistent contextual loop, exact A/B, and discoverable selection controls.
2. **Make drawn shapes affect the right audio.** Introduce a bounded,
   versioned time-frequency mask model. Add Brush, Lasso, Add/Subtract, feathering
   and multiple repair regions. Keep 1.5 rectangle recipes readable and preserve
   their existing sound. Make preview and export use the same Rust processing.
3. **Add specialised tools after listening tests.** Start with isolated click
   repair, hum suggestions and steady-noise profiles. Prototype short-gap fill
   and repeat matching separately before adding them to the main rail.

For the first pass, give people three choices within easy reach **Select**,
**Listen** and **Reduce**. Keep advanced options near the selected repair.
Measure drag latency with long recordings, verify dry/wet alignment with test
tones and channel-isolated signals, and listen to percussion and sustained
music before calling the processing ready.
