//! Local recording analysis and export. A frame is one sample per channel.

mod codec;
mod frame_copy;
pub mod markers;

pub use codec::{
    check_output, flac_output_bits, inspect, inspect_file, inspect_file_with_progress, probe_file,
    render, render_file, render_file_with_outputs, waveform, waveform_file, waveform_file_indexed,
    Analysis, AnalysisProgress, FrameCount, MediaError, OutputFormat, Peak, PreviewDecoder,
    ProbedSource, RenderProgress, RenderSegment, SeekIndex, SourceInfo, WaveformIndex,
};
pub use frame_copy::{CopyCodec, FrameCopyIndex};
pub use markers::FrameRange;
