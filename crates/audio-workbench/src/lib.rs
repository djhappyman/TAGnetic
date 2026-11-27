//! Local recording analysis and export. A frame is one sample per channel.

mod codec;
mod frame_copy;
pub mod markers;

pub use codec::{
    check_output, compare_preview_levels, flac_output_bits, inspect, inspect_file,
    inspect_file_with_progress, learn_eq_threshold, match_reference, prepare_treatment, probe_file,
    probe_treatment, probe_treatment_with_options, render, render_file, render_file_with_outputs,
    render_treated_file, waveform, waveform_channels_file_indexed, waveform_file,
    waveform_file_indexed, Analysis, AnalysisProgress, AudioTreatment, DynamicEq, EqBand,
    EqBandKind, EqChannel, FrameCount, HumFilter, MaskOperation, MediaError, MonoMode,
    OutputFormat, Peak, PlaybackHistory, PlaybackProbeWindow, PlaybackReadings, Polarity,
    PreparedTreatment, PreviewComparison, PreviewDecoder, PreviewMonitor, ProbedSource,
    RenderProgress, RenderSegment, RepairBlock, RepairChannel, RepairMask, RepairMaskPoint,
    RepairMaskRegion, RepairSource, SeekIndex, SelectionPcm, SourceInfo, SpectralEqResponse,
    SpectralRepair, SpectrogramChannel, SpectrogramDetail, SpectrumView, TreatmentProbe,
    TreatmentProbeOptions, TreatmentProcessor, WaveformIndex, WaveformPeaks, MAX_EQ_BANDS,
    MAX_SPECTRAL_REPAIRS,
};
pub use frame_copy::{CopyCodec, FrameCopyIndex};
pub use markers::FrameRange;
