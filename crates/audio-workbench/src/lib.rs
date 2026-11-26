//! Local recording analysis and export. A frame is one sample per channel.

mod codec;
mod frame_copy;
pub mod markers;

pub use codec::{
    check_output, flac_output_bits, inspect, inspect_file, inspect_file_with_progress,
    prepare_treatment, probe_file, probe_treatment, probe_treatment_with_options, render,
    render_file, render_file_with_outputs, render_treated_file, waveform,
    waveform_channels_file_indexed, waveform_file, waveform_file_indexed, Analysis,
    AnalysisProgress, AudioTreatment, FrameCount, HumFilter, MediaError, MonoMode, OutputFormat,
    Peak, Polarity, PreparedTreatment, PreviewDecoder, ProbedSource, RenderProgress, RenderSegment,
    SeekIndex, SourceInfo, SpectralRepair, TreatmentProbe, TreatmentProbeOptions,
    TreatmentProcessor, WaveformIndex, WaveformPeaks, MAX_SPECTRAL_REPAIRS,
};
pub use frame_copy::{CopyCodec, FrameCopyIndex};
pub use markers::FrameRange;
