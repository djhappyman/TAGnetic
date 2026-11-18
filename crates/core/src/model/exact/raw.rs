//! Bounded native envelope scans. Audio extents are sought over, never copied.
use super::*;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};

const RAW_LIMIT: usize = 16 * 1024 * 1024;
const BOX_LIMIT: usize = 65_536;
// Kept inside the existing 64 MiB working admission: only ranges and digests
// are retained, never a second metadata image or any audio payload. The work
// cap also bounds rereads of overlapping observations from nested layouts.
const WITNESS_RANGE_BYTES: usize = 1024 * 1024;
const WITNESS_IO_LIMIT: u64 = 64 * 1024 * 1024;
const WITNESS_CHUNK: usize = 8192;

#[cfg(test)]
#[path = "raw_witness_tests.rs"]
mod witness_tests;

#[derive(Clone, Copy)]
struct ReadSpan {
    at: u64,
    len: u64,
}
const WITNESS_RANGE_LIMIT: usize = WITNESS_RANGE_BYTES / std::mem::size_of::<ReadSpan>();

fn witness_digest() -> Sha256 {
    let mut digest = Sha256::new();
    digest.update(b"tagrex-exact-observed-metadata-v1");
    digest
}

fn append_span(digest: &mut Sha256, span: ReadSpan, bytes: impl AsRef<[u8]>) {
    digest.update(span.at.to_le_bytes());
    digest.update(span.len.to_le_bytes());
    digest.update(bytes);
}

struct WitnessBuilder {
    spans: Vec<ReadSpan>,
    current: Option<ReadSpan>,
    current_digest: Sha256,
    digest: Sha256,
    observed: u64,
}
impl WitnessBuilder {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            current: None,
            current_digest: Sha256::new(),
            digest: witness_digest(),
            observed: 0,
        }
    }
    fn finish_span(&mut self) -> Result<(), ExactReadError> {
        let Some(span) = self.current.take() else {
            return Ok(());
        };
        if self.spans.len() == WITNESS_RANGE_LIMIT {
            return Err(ExactReadError::Limit);
        }
        if self.spans.len() == self.spans.capacity() {
            let capacity = self.spans.capacity().max(1);
            let next = capacity.saturating_mul(2).min(WITNESS_RANGE_LIMIT);
            self.spans
                .try_reserve_exact(next - self.spans.len())
                .map_err(|_| ExactReadError::Limit)?;
        }
        let hash = std::mem::take(&mut self.current_digest).finalize();
        append_span(&mut self.digest, span, hash);
        self.spans.push(span);
        Ok(())
    }
    fn admit(&self, len: u64) -> Result<(), ExactReadError> {
        if self
            .observed
            .checked_add(len)
            .is_none_or(|observed| observed > WITNESS_IO_LIMIT)
        {
            return Err(ExactReadError::Limit);
        }
        Ok(())
    }
    fn observe(&mut self, at: u64, bytes: &[u8]) -> Result<(), ExactReadError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.admit(bytes.len() as u64)?;
        if self
            .current
            .is_some_and(|span| span.at.checked_add(span.len) != Some(at))
        {
            self.finish_span()?;
        }
        if self.current.is_none() {
            if self.spans.len() == WITNESS_RANGE_LIMIT {
                return Err(ExactReadError::Limit);
            }
            self.current = Some(ReadSpan { at, len: 0 });
        }
        let span = self.current.as_mut().ok_or(ExactReadError::Limit)?;
        span.len = span
            .len
            .checked_add(bytes.len() as u64)
            .ok_or(ExactReadError::Limit)?;
        self.current_digest.update(bytes);
        self.observed += bytes.len() as u64;
        Ok(())
    }
    fn finish(mut self) -> Result<ReadWitness, ExactReadError> {
        self.finish_span()?;
        Ok(ReadWitness {
            spans: self.spans,
            digest: self.digest.finalize().into(),
        })
    }
}

struct ReadWitness {
    spans: Vec<ReadSpan>,
    digest: [u8; 32],
}

enum ProofLayout {
    Flac,
    Mpeg,
    Wave,
    Aiff,
    Mp4,
}
impl ReadWitness {
    fn matches(&self, source: &std::fs::File) -> Result<bool, ExactReadError> {
        // Shared-cursor clones are already excluded by the held-reader API.
        // Compatibility passes rewind their own sessions before parsing.
        let mut file = source.try_clone()?;
        let mut digest = witness_digest();
        let mut bytes = [0; WITNESS_CHUNK];
        for span in &self.spans {
            file.seek(SeekFrom::Start(span.at))?;
            let mut remaining = span.len;
            let mut part = Sha256::new();
            while remaining != 0 {
                let count = remaining.min(WITNESS_CHUNK as u64) as usize;
                if let Err(error) = file.read_exact(&mut bytes[..count]) {
                    if error.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Ok(false);
                    }
                    return Err(error.into());
                }
                part.update(&bytes[..count]);
                remaining -= count as u64;
            }
            append_span(&mut digest, *span, part.finalize());
        }
        let digest: [u8; 32] = digest.finalize().into();
        Ok(digest == self.digest)
    }
}

pub(super) struct RawBlock {
    pub kind: TagBlockKind,
    pub revision: Option<Id3v2Revision>,
    pub bytes: Vec<u8>,
}
#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
    #[cfg(windows)]
    identity: (u64, [u8; 16], i64),
}
impl Stamp {
    fn of(path: &std::path::Path) -> Result<Self, ExactReadError> {
        let file = open_regular(path)?;
        Self::from_file(&file)
    }
    fn from_file(file: &std::fs::File) -> Result<Self, ExactReadError> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(ExactReadError::Invalid(
                "Exact metadata needs a regular file",
            ));
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(windows)]
        let identity = {
            use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
                FILE_ID_INFO,
            };
            let mut basic = MaybeUninit::<FILE_BASIC_INFO>::uninit();
            let mut identity = MaybeUninit::<FILE_ID_INFO>::uninit();
            // SAFETY: the owned File keeps the handle live; output pointers
            // have the exact required alignment/size and are read only after
            // both Windows calls report successful initialization.
            unsafe {
                if GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileBasicInfo,
                    basic.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                ) == 0
                    || GetFileInformationByHandleEx(
                        file.as_raw_handle(),
                        FileIdInfo,
                        identity.as_mut_ptr().cast(),
                        std::mem::size_of::<FILE_ID_INFO>() as u32,
                    ) == 0
                {
                    return Err(ExactReadError::Unavailable(
                        UnavailableReason::UnsupportedLayout,
                    ));
                }
                let basic = basic.assume_init();
                let identity = identity.assume_init();
                (
                    identity.VolumeSerialNumber,
                    identity.FileId.Identifier,
                    basic.ChangeTime,
                )
            }
        };
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
            #[cfg(windows)]
            identity,
        })
    }
}

fn open_regular(path: &std::path::Path) -> Result<std::fs::File, ExactReadError> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{open, Mode, OFlags};
        std::fs::File::from(
            open(
                path,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(std::io::Error::from)?,
        )
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ExactReadError::Invalid(
            "Exact metadata needs a regular file",
        ));
    }
    Ok(file)
}
pub(in crate::model) struct Input {
    pub(super) blocks: Vec<RawBlock>,
    stamp: Stamp,
    source: std::sync::Arc<std::fs::File>,
    named_path: Option<std::path::PathBuf>,
    witness: ReadWitness,
    layout: ProofLayout,
    _working: crate::resources::Lease,
}
struct Scan<'a> {
    file: std::fs::File,
    len: u64,
    charged: usize,
    visits: usize,
    blocks: Vec<RawBlock>,
    witness: WitnessBuilder,
    metadata_proof_depth: usize,
    leading_ape_span: Option<ReadSpan>,
    cancelled: &'a dyn Fn() -> bool,
}
impl Scan<'_> {
    fn visit(&mut self) -> Result<(), ExactReadError> {
        if (self.cancelled)() {
            return Err(ExactReadError::Cancelled);
        }
        self.visits += 1;
        if self.visits > BOX_LIMIT {
            return Err(ExactReadError::Limit);
        }
        Ok(())
    }
    fn at<const N: usize>(&mut self, at: u64) -> Result<[u8; N], ExactReadError> {
        if (self.cancelled)() {
            return Err(ExactReadError::Cancelled);
        }
        if at.checked_add(N as u64).is_none_or(|end| end > self.len) {
            return Err(ExactReadError::Invalid("Truncated metadata envelope"));
        }
        self.witness.admit(N as u64)?;
        let mut bytes = [0; N];
        self.file.seek(SeekFrom::Start(at))?;
        self.file.read_exact(&mut bytes)?;
        self.witness.observe(at, &bytes)?;
        Ok(bytes)
    }
    fn range(&mut self, at: u64, len: u64) -> Result<Vec<u8>, ExactReadError> {
        if (self.cancelled)() {
            return Err(ExactReadError::Cancelled);
        }
        let len = usize::try_from(len).map_err(|_| ExactReadError::Limit)?;
        self.charged = self.charged.checked_add(len).ok_or(ExactReadError::Limit)?;
        if self.charged > RAW_LIMIT {
            return Err(ExactReadError::Limit);
        }
        if at.checked_add(len as u64).is_none_or(|end| end > self.len) {
            return Err(ExactReadError::Invalid("Truncated metadata payload"));
        }
        self.witness.admit(len as u64)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| ExactReadError::Limit)?;
        bytes.resize(len, 0);
        self.file.seek(SeekFrom::Start(at))?;
        self.file.read_exact(&mut bytes)?;
        self.witness.observe(at, &bytes)?;
        Ok(bytes)
    }
    fn witness_range(&mut self, mut at: u64, mut len: u64) -> Result<(), ExactReadError> {
        if at.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(ExactReadError::Invalid("Metadata witness exceeds file"));
        }
        self.witness.admit(len)?;
        self.file.seek(SeekFrom::Start(at))?;
        let mut bytes = [0; WITNESS_CHUNK];
        while len != 0 {
            if (self.cancelled)() {
                return Err(ExactReadError::Cancelled);
            }
            let count = len.min(WITNESS_CHUNK as u64) as usize;
            // A checkpoint may reenter a held reader and move a shared cursor.
            self.file.seek(SeekFrom::Start(at))?;
            self.file.read_exact(&mut bytes[..count])?;
            self.witness.observe(at, &bytes[..count])?;
            at += count as u64;
            len -= count as u64;
        }
        Ok(())
    }
    fn mp4_container(&mut self, at: u64, end: u64, depth: usize) -> Result<(), ExactReadError> {
        let len = end.checked_sub(at).ok_or(ExactReadError::Limit)?;
        if len > RAW_LIMIT as u64 {
            return Err(ExactReadError::Limit);
        }
        // One outer-container proof covers ignored metadata payloads. Nested
        // containers do not hash that whole extent repeatedly; actual parser
        // observations still enter the witness to detect inconsistent reads.
        if self.metadata_proof_depth == 0 {
            self.witness_range(at, len)?;
        }
        self.metadata_proof_depth += 1;
        let result = self.mp4(at, end, depth);
        self.metadata_proof_depth -= 1;
        result
    }
    fn charge_metadata(&mut self, len: u64) -> Result<(), ExactReadError> {
        let len = usize::try_from(len).map_err(|_| ExactReadError::Limit)?;
        self.charged = self.charged.checked_add(len).ok_or(ExactReadError::Limit)?;
        if self.charged > RAW_LIMIT {
            return Err(ExactReadError::Limit);
        }
        Ok(())
    }
    fn ape_footer(&mut self, end: u64) -> Result<bool, ExactReadError> {
        if end < 32 || self.at::<8>(end - 32)? != *b"APETAGEX" {
            return Ok(false);
        }
        let footer = self.at::<32>(end - 32)?;
        let word = |at: usize| {
            u32::from_le_bytes([footer[at], footer[at + 1], footer[at + 2], footer[at + 3]])
        };
        let version = word(8);
        let size = u64::from(word(12));
        let flags = word(20);
        if !matches!(version, 1000 | 2000) || size < 32 || flags & (1 << 29) != 0 {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let header = if version == 2000 && flags & (1 << 31) != 0 {
            32
        } else {
            0
        };
        let total = size.checked_add(header).ok_or(ExactReadError::Limit)?;
        let at = end
            .checked_sub(total)
            .ok_or(ExactReadError::Invalid("APE metadata exceeds file"))?;
        if header != 0 && self.at::<8>(at)? != *b"APETAGEX" {
            return Err(ExactReadError::Invalid("APE metadata header is absent"));
        }
        if !self
            .leading_ape_span
            .is_some_and(|span| span.at == at && span.len == total)
        {
            self.charge_metadata(total)?;
            self.witness_range(at, total)?;
        }
        Ok(true)
    }
    fn mpeg_trailers(&mut self) -> Result<(), ExactReadError> {
        // These are fixed-size marker probes, not a scan/hash of audio. The
        // legacy reader checks APE at EOF first, then before an ID3v1 tag.
        let id3v1 = if self.len >= 128 && self.at::<3>(self.len - 128)? == *b"TAG" {
            let at = self.len - 128;
            self.charge_metadata(128)?;
            self.witness_range(at, 128)?;
            Some(at)
        } else {
            None
        };
        if self.ape_footer(self.len)? {
            return Ok(());
        }
        let end = id3v1.unwrap_or(self.len);
        if id3v1.is_some() && self.ape_footer(end)? {
            return Ok(());
        }
        if end >= 9 {
            let marker = self.at::<9>(end - 9)?;
            if matches!(&marker, b"LYRICS200" | b"LYRICSEND") {
                // A separate suffix grammar is needed to locate metadata
                // before Lyrics3. Do not return an unproved legacy snapshot.
                return Err(ExactReadError::Unavailable(
                    UnavailableReason::UnsupportedLayout,
                ));
            }
        }
        Ok(())
    }
    fn leading_ape(&mut self, at: u64) -> Result<(), ExactReadError> {
        let header = self.at::<32>(at)?;
        if &header[..8] != b"APETAGEX" {
            return Err(ExactReadError::Invalid("APE metadata header is absent"));
        }
        let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let size = u64::from(u32::from_le_bytes([
            header[12], header[13], header[14], header[15],
        ]));
        if version != 2000 || size < 32 {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let total = size.checked_add(32).ok_or(ExactReadError::Limit)?;
        self.charge_metadata(total)?;
        self.witness_range(at, total)?;
        self.leading_ape_span = Some(ReadSpan { at, len: total });
        Ok(())
    }
    fn id3(&mut self, at: u64, extent: u64) -> Result<u64, ExactReadError> {
        let header = self.at::<10>(at)?;
        if &header[..3] != b"ID3" {
            return Err(ExactReadError::Invalid("ID3 envelope has no ID3 header"));
        }
        let revision = match header[3] {
            3 => Id3v2Revision::V3,
            4 => Id3v2Revision::V4,
            _ => {
                return Err(ExactReadError::Unavailable(
                    UnavailableReason::UnsupportedLayout,
                ))
            }
        };
        // Unsynchronisation, compression, footer and extended-header variants
        // need their own raw proof. Never send them through a lossy exact path.
        if header[5] != 0 {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let size = syncsafe(&header[6..10])? as u64;
        let total = size.checked_add(10).ok_or(ExactReadError::Limit)?;
        if total > extent {
            return Err(ExactReadError::Invalid(
                "ID3 size exceeds its container extent",
            ));
        }
        let bytes = self.range(at + 10, size)?;
        validate_id3_frames(&bytes, revision)?;
        self.blocks.push(RawBlock {
            kind: TagBlockKind::Id3v2,
            revision: Some(revision),
            bytes,
        });
        Ok(total)
    }
    fn flac(&mut self, mut at: u64) -> Result<(), ExactReadError> {
        if self.at::<4>(at)? != *b"fLaC" {
            return Err(ExactReadError::Invalid("Missing FLAC marker"));
        }
        at += 4;
        loop {
            self.visit()?;
            let header = self.at::<4>(at)?;
            let size = u32::from_be_bytes([0, header[1], header[2], header[3]]) as u64;
            let end = at.checked_add(4 + size).ok_or(ExactReadError::Limit)?;
            if end > self.len {
                return Err(ExactReadError::Invalid("FLAC metadata exceeds file"));
            }
            // Bound all metadata, including pictures/unknown blocks, before the
            // compatibility parser can allocate its own representation.
            if self
                .charged
                .checked_add(size as usize)
                .is_none_or(|n| n > RAW_LIMIT)
            {
                return Err(ExactReadError::Limit);
            }
            if header[0] & 0x7f == 4 {
                let bytes = self.range(at + 4, size)?;
                validate_vorbis(&bytes)?;
                self.blocks.push(RawBlock {
                    kind: TagBlockKind::VorbisComments,
                    revision: None,
                    bytes,
                });
            } else {
                self.charged += size as usize;
                self.witness_range(at + 4, size)?;
            }
            at = end;
            if header[0] & 0x80 != 0 {
                return Ok(());
            }
        }
    }
    fn riff(&mut self, big: bool) -> Result<(), ExactReadError> {
        let header = self.at::<12>(0)?;
        let size = if big {
            u32::from_be_bytes(
                header[4..8]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("RIFF size"))?,
            )
        } else {
            u32::from_le_bytes(
                header[4..8]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("RIFF size"))?,
            )
        };
        let end = u64::from(size)
            .checked_add(8)
            .ok_or(ExactReadError::Limit)?;
        if end > self.len {
            return Err(ExactReadError::Invalid(
                "RIFF/AIFF declared size exceeds file",
            ));
        }
        let mut at = 12u64;
        while at < end {
            self.visit()?;
            let chunk = self.at::<8>(at)?;
            let length = if big {
                u32::from_be_bytes(
                    chunk[4..]
                        .try_into()
                        .map_err(|_| ExactReadError::Invalid("Chunk size"))?,
                )
            } else {
                u32::from_le_bytes(
                    chunk[4..]
                        .try_into()
                        .map_err(|_| ExactReadError::Invalid("Chunk size"))?,
                )
            } as u64;
            let next = at
                .checked_add(8 + length + (length & 1))
                .ok_or(ExactReadError::Limit)?;
            if next > end {
                return Err(ExactReadError::Invalid("Chunk exceeds its container"));
            }
            if chunk[..4].eq_ignore_ascii_case(b"ID3 ") {
                self.id3(at + 8, length)?;
            } else if !matches!(&chunk[..4], b"data" | b"SSND") {
                self.charged = self
                    .charged
                    .checked_add(length as usize)
                    .ok_or(ExactReadError::Limit)?;
                if self.charged > RAW_LIMIT {
                    return Err(ExactReadError::Limit);
                }
                self.witness_range(at + 8, length + (length & 1))?;
            }
            at = next;
        }
        Ok(())
    }
    fn mp4(&mut self, mut at: u64, end: u64, depth: usize) -> Result<(), ExactReadError> {
        if depth > 6 {
            return Err(ExactReadError::Limit);
        }
        while at < end {
            self.visit()?;
            let header = self.at::<8>(at)?;
            let (size, width) = match u32::from_be_bytes(
                header[..4]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("Atom size"))?,
            ) {
                0 => {
                    return Err(ExactReadError::Unavailable(
                        UnavailableReason::UnsupportedLayout,
                    ))
                }
                1 => (u64::from_be_bytes(self.at::<8>(at + 8)?), 16),
                size => (u64::from(size), 8),
            };
            let next = at.checked_add(size).ok_or(ExactReadError::Limit)?;
            if size < width || next > end {
                return Err(ExactReadError::Invalid("MP4 atom exceeds its parent"));
            }
            match &header[4..8] {
                b"ftyp" => {
                    if size > RAW_LIMIT as u64 {
                        return Err(ExactReadError::Limit);
                    }
                    self.witness_range(at + width, size - width)?;
                }
                b"moov" | b"udta" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" => {
                    if size > RAW_LIMIT as u64 {
                        return Err(ExactReadError::Limit);
                    }
                    self.mp4_container(at + width, next, depth + 1)?;
                }
                b"meta" => {
                    if size < width + 4 || self.at::<4>(at + width)? != [0; 4] {
                        return Err(ExactReadError::Unavailable(
                            UnavailableReason::UnsupportedLayout,
                        ));
                    }
                    self.mp4_container(at + width + 4, next, depth + 1)?;
                }
                b"ilst" => {
                    let bytes = self.range(at + width, size - width)?;
                    validate_ilst(&bytes)?;
                    self.blocks.push(RawBlock {
                        kind: TagBlockKind::Mp4Ilst,
                        revision: None,
                        bytes,
                    });
                }
                b"stco" | b"co64" | b"stsc" | b"stts" | b"ctts" | b"stss" => {
                    if size < width + 8 {
                        return Err(ExactReadError::Invalid("Truncated MP4 sample table"));
                    }
                    let count = u32::from_be_bytes(self.at::<4>(at + width + 4)?) as u64;
                    let record = match &header[4..8] {
                        b"co64" | b"stts" | b"ctts" => 8,
                        b"stsc" => 12,
                        _ => 4,
                    };
                    if count
                        .checked_mul(record)
                        .and_then(|bytes| bytes.checked_add(8))
                        .is_none_or(|needed| needed > size - width)
                    {
                        return Err(ExactReadError::Invalid(
                            "MP4 sample table count exceeds its extent",
                        ));
                    }
                }
                b"stsz" => {
                    if size < width + 12 {
                        return Err(ExactReadError::Invalid("Truncated MP4 sample sizes"));
                    }
                    let common = u32::from_be_bytes(self.at::<4>(at + width + 4)?);
                    let count = u32::from_be_bytes(self.at::<4>(at + width + 8)?) as u64;
                    if common == 0
                        && count
                            .checked_mul(4)
                            .and_then(|bytes| bytes.checked_add(12))
                            .is_none_or(|needed| needed > size - width)
                    {
                        return Err(ExactReadError::Invalid(
                            "MP4 sample sizes exceed their extent",
                        ));
                    }
                }
                b"stsd" => {
                    if size < width + 8 {
                        return Err(ExactReadError::Invalid("Truncated MP4 sample descriptions"));
                    }
                    let count = u32::from_be_bytes(self.at::<4>(at + width + 4)?);
                    if count > MAX_ENTRIES as u32 {
                        return Err(ExactReadError::Limit);
                    }
                    let mut entry = at + width + 8;
                    for _ in 0..count {
                        self.visit()?;
                        let description = self.at::<8>(entry)?;
                        let length = u32::from_be_bytes(
                            description[..4]
                                .try_into()
                                .map_err(|_| ExactReadError::Invalid("Sample description size"))?,
                        ) as u64;
                        let end = entry.checked_add(length).ok_or(ExactReadError::Limit)?;
                        if length < 8 || end > next {
                            return Err(ExactReadError::Invalid(
                                "Sample description exceeds its extent",
                            ));
                        }
                        if matches!(&description[4..8], b"enca" | b"encv") {
                            return Err(ExactReadError::Unavailable(
                                UnavailableReason::UnsupportedLayout,
                            ));
                        }
                        entry = end;
                    }
                    if entry != next {
                        return Err(ExactReadError::Invalid(
                            "Unexpected sample-description suffix",
                        ));
                    }
                }
                // Do not infer safe metadata relocation/interpretation for
                // compressed, fragmented or protected movie representations.
                b"cmov" | b"moof" | b"mvex" => {
                    return Err(ExactReadError::Unavailable(
                        UnavailableReason::UnsupportedLayout,
                    ))
                }
                _ => {}
            }
            at = next;
        }
        Ok(())
    }
}

impl Input {
    pub fn prepare(
        path: &std::path::Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, ExactReadError> {
        Self::prepare_source(Some(path), || open_regular(path), cancelled)
    }
    pub fn prepare_file(
        file: std::fs::File,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, ExactReadError> {
        Self::prepare_source(None, || Ok(file), cancelled)
    }
    fn prepare_source(
        named_path: Option<&std::path::Path>,
        open: impl FnOnce() -> Result<std::fs::File, ExactReadError>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, ExactReadError> {
        if cancelled() {
            return Err(ExactReadError::Cancelled);
        }
        let working =
            crate::resources::Budget::process().try_acquire(crate::resources::Charge {
                transient: 64 * 1024 * 1024,
                jobs: 1,
                ..Default::default()
            })?;
        let source = std::sync::Arc::new(open()?);
        let stamp = Stamp::from_file(&source)?;
        let mut scan = Scan {
            file: source.try_clone()?,
            len: stamp.len,
            charged: 0,
            visits: 0,
            blocks: Vec::new(),
            witness: WitnessBuilder::new(),
            metadata_proof_depth: 0,
            leading_ape_span: None,
            cancelled,
        };
        let header = scan.at::<12>(0)?;
        let layout = if &header[..4] == b"fLaC" {
            scan.flac(0)?;
            ProofLayout::Flac
        } else if &header[..3] == b"ID3" {
            let after = scan.id3(0, scan.len)?;
            let next = if after.checked_add(4).is_some_and(|end| end <= scan.len) {
                Some(scan.at::<4>(after)?)
            } else {
                None
            };
            if next == Some(*b"fLaC") {
                scan.flac(after)?;
                ProofLayout::Flac
            } else {
                if next == Some(*b"APET") {
                    scan.leading_ape(after)?;
                }
                scan.mpeg_trailers()?;
                ProofLayout::Mpeg
            }
        } else if &header[..4] == b"RIFF" && &header[8..] == b"WAVE" {
            scan.riff(false)?;
            ProofLayout::Wave
        } else if &header[..4] == b"FORM" && matches!(&header[8..], b"AIFF" | b"AIFC") {
            scan.riff(true)?;
            ProofLayout::Aiff
        } else if matches!(&header[4..8], b"ftyp" | b"free" | b"moov") {
            let len = scan.len;
            scan.mp4(0, len, 0)?;
            ProofLayout::Mp4
        } else if header[0] == 0xff {
            scan.mpeg_trailers()?;
            ProofLayout::Mpeg
        } else {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        };
        let input = Self {
            blocks: scan.blocks,
            stamp,
            source,
            named_path: named_path.map(std::path::Path::to_path_buf),
            witness: scan.witness.finish()?,
            layout,
            _working: working,
        };
        input.ensure_unchanged()?;
        Ok(input)
    }
    pub fn ensure_format(&self, format: AudioFormat) -> Result<(), ExactReadError> {
        if !matches!(
            (&self.layout, format),
            (ProofLayout::Flac, AudioFormat::Flac)
                | (ProofLayout::Mpeg, AudioFormat::Mp3 | AudioFormat::Aac)
                | (ProofLayout::Wave, AudioFormat::Wav)
                | (ProofLayout::Aiff, AudioFormat::Aiff)
                | (ProofLayout::Mp4, AudioFormat::M4a)
        ) {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        Ok(())
    }
    pub fn ensure_unchanged(&self) -> Result<(), ExactReadError> {
        self.ensure_stamp()?;
        // Windows and coarse-time filesystems may report equal stamps after a
        // same-length write. Recheck the bounded metadata bytes actually used
        // by this request, through its held source, with no public callbacks.
        if !self.witness.matches(&self.source)? {
            return Err(ExactReadError::Changed);
        }
        self.ensure_stamp()
    }
    fn ensure_stamp(&self) -> Result<(), ExactReadError> {
        if Stamp::from_file(&self.source)? != self.stamp {
            return Err(ExactReadError::Changed);
        }
        if let Some(path) = &self.named_path {
            if Stamp::of(path)? != self.stamp {
                return Err(ExactReadError::Changed);
            }
        }
        Ok(())
    }
    pub fn bind_source(&self, path: &std::path::Path) -> super::source::Scope {
        super::source::Scope::enter(path, self.source.clone())
    }
}

pub(super) fn syncsafe(bytes: &[u8]) -> Result<u32, ExactReadError> {
    if bytes.len() != 4 || bytes.iter().any(|byte| byte & 0x80 != 0) {
        return Err(ExactReadError::Invalid("Invalid synchsafe size"));
    }
    Ok(bytes
        .iter()
        .fold(0, |size, byte| (size << 7) | u32::from(*byte)))
}
// Bound compatibility-parser expansion before Lofty turns text lists into
// generic owned TagItems. Native snapshot validation happens later and cannot
// retroactively admit those intermediate allocations or long scalar joins.
fn validate_id3_text_expansion(
    frame: &[u8],
    body: &[u8],
    total_values: &mut usize,
    total_text_bytes: &mut usize,
) -> Result<(), ExactReadError> {
    let qualified = matches!(frame, b"COMM" | b"USLT");
    if frame.first() != Some(&b'T') && !qualified {
        return Ok(());
    }
    let Some((&encoding, mut payload)) = body.split_first() else {
        return Ok(());
    };
    let width = match encoding {
        0 | 3 => 1,
        1 | 2 => 2,
        _ => return Ok(()), // Unsupported encodings keep the existing diagnostic.
    };
    if qualified {
        let Some(rest) = payload.get(3..) else {
            return Ok(());
        };
        payload = rest;
    }
    if qualified || frame == b"TXXX" {
        let end = if width == 1 {
            payload.iter().position(|byte| *byte == 0)
        } else {
            payload
                .as_chunks::<2>()
                .0
                .iter()
                .position(|unit| *unit == [0, 0])
                .map(|at| at * 2)
        };
        let Some(end) = end else {
            if payload.len() > MAX_NAME * 2 + 2 {
                return Err(ExactReadError::Limit);
            }
            return Ok(()); // The native parser diagnoses a short malformed name.
        };
        if end > MAX_NAME * 2 + 2 {
            return Err(ExactReadError::Limit);
        }
        payload = &payload[end + width..];
    }
    // The native reader rejects larger payloads/decoded fields. Do not let
    // compatibility conversion allocate an expanded scalar before that check.
    if payload.len() > MAX_FIELD_BYTES {
        return Err(ExactReadError::Limit);
    }
    let decoded = if encoding == 0 {
        if let Some(encoding) = super::super::legacy_code_page() {
            let mut decoder = encoding.new_decoder_without_bom_handling();
            let mut rest = payload;
            let mut output = [0u8; 512];
            let mut length = 0usize;
            loop {
                let (result, read, written) =
                    decoder.decode_to_utf8_without_replacement(rest, &mut output, true);
                length = length
                    .checked_add(written)
                    .filter(|length| *length <= MAX_FIELD_BYTES)
                    .ok_or(ExactReadError::Limit)?;
                rest = &rest[read..];
                match result {
                    encoding_rs::DecoderResult::InputEmpty => break length,
                    encoding_rs::DecoderResult::OutputFull => {}
                    encoding_rs::DecoderResult::Malformed(_, _) => {
                        return Err(ExactReadError::Invalid(
                            "Invalid configured ID3 codepage text",
                        ));
                    }
                }
            }
        } else {
            payload
                .iter()
                .map(|byte| char::from(*byte).len_utf8())
                .sum()
        }
    } else if width == 2 {
        // UTF-16 is diagnostic-only in the native profile; reserve its largest
        // possible compatibility UTF-8 expansion without allocating a string.
        payload.len().div_ceil(2) * 3
    } else {
        payload.len()
    };
    if decoded > MAX_FIELD_BYTES {
        return Err(ExactReadError::Limit);
    }
    *total_text_bytes = total_text_bytes
        .checked_add(decoded)
        .filter(|bytes| *bytes <= MAX_SNAPSHOT_BYTES)
        .ok_or(ExactReadError::Limit)?;
    let values = if qualified {
        1
    } else if width == 1 {
        payload.iter().filter(|byte| **byte == 0).take(1024).count() + 1
    } else {
        // Count aligned UTF-16 NUL code units, not individual zero bytes in
        // ordinary ASCII/surrogate text. A BOM is a non-NUL code unit.
        payload
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|unit| **unit == [0, 0])
            .take(1024)
            .count()
            + 1
    };
    if values > 1024 {
        return Err(ExactReadError::Limit);
    }
    *total_values = total_values
        .checked_add(values)
        .filter(|total| *total <= 16_384)
        .ok_or(ExactReadError::Limit)?;
    Ok(())
}

fn validate_id3_frames(bytes: &[u8], revision: Id3v2Revision) -> Result<(), ExactReadError> {
    let mut count = 0;
    let mut total_values = 0usize;
    let mut total_text_bytes = 0usize;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == 0 {
            if bytes[at..].iter().any(|byte| *byte != 0) {
                return Err(ExactReadError::Invalid("Nonzero data after ID3 padding"));
            }
            break;
        }
        let header = bytes
            .get(at..at + 10)
            .ok_or(ExactReadError::Invalid("Truncated ID3 frame"))?;
        if !header[..4]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(ExactReadError::Invalid("Invalid ID3 frame identifier"));
        }
        if header[8..] != [0, 0] {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let size = if revision == Id3v2Revision::V4 {
            syncsafe(&header[4..8])?
        } else {
            u32::from_be_bytes(
                header[4..8]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("ID3 size"))?,
            )
        } as usize;
        let start = at;
        at = at
            .checked_add(10)
            .and_then(|at| at.checked_add(size))
            .ok_or(ExactReadError::Limit)?;
        count += 1;
        if count > MAX_ENTRIES || at > bytes.len() {
            return Err(ExactReadError::Limit);
        }
        validate_id3_text_expansion(
            &header[..4],
            &bytes[start + 10..at],
            &mut total_values,
            &mut total_text_bytes,
        )?;
    }
    Ok(())
}
pub(super) fn vorbis_items(bytes: &[u8]) -> Result<Vec<(&str, &str)>, ExactReadError> {
    fn size(bytes: &[u8], at: &mut usize) -> Result<usize, ExactReadError> {
        let length = bytes
            .get(*at..*at + 4)
            .ok_or(ExactReadError::Invalid("Truncated Vorbis length"))?;
        *at += 4;
        Ok(u32::from_le_bytes(
            length
                .try_into()
                .map_err(|_| ExactReadError::Invalid("Vorbis length"))?,
        ) as usize)
    }
    let mut at = 0;
    let vendor = size(bytes, &mut at)?;
    at = at.checked_add(vendor).ok_or(ExactReadError::Limit)?;
    let count = size(bytes, &mut at)?;
    if count > MAX_ENTRIES {
        return Err(ExactReadError::Limit);
    }
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        let length = size(bytes, &mut at)?;
        let end = at.checked_add(length).ok_or(ExactReadError::Limit)?;
        let value = std::str::from_utf8(
            bytes
                .get(at..end)
                .ok_or(ExactReadError::Invalid("Truncated Vorbis item"))?,
        )
        .map_err(|_| ExactReadError::Invalid("Invalid UTF-8 Vorbis item"))?;
        let (name, value) = value
            .split_once('=')
            .ok_or(ExactReadError::Invalid("Vorbis item has no separator"))?;
        if name.len() > MAX_NAME
            || (!matches!(
                name.to_ascii_uppercase().as_str(),
                "METADATA_BLOCK_PICTURE" | "COVERART" | "COVERARTMIME"
            ) && value.len() > MAX_FIELD_BYTES)
        {
            return Err(ExactReadError::Limit);
        }
        items.push((name, value));
        at = end;
    }
    if at != bytes.len() {
        return Err(ExactReadError::Invalid("Unexpected Vorbis packet suffix"));
    }
    Ok(items)
}
fn validate_vorbis(bytes: &[u8]) -> Result<(), ExactReadError> {
    vorbis_items(bytes).map(|_| ())
}

type AtomSlice<'a> = ([u8; 4], &'a [u8]);
pub(super) fn atoms(bytes: &[u8]) -> Result<Vec<AtomSlice<'_>>, ExactReadError> {
    let mut at = 0usize;
    let mut atoms = Vec::new();
    while at < bytes.len() {
        let header = bytes
            .get(at..at + 8)
            .ok_or(ExactReadError::Invalid("Truncated ilst atom"))?;
        let size = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| ExactReadError::Invalid("ilst size"))?,
        ) as usize;
        if size < 8 {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let end = at.checked_add(size).ok_or(ExactReadError::Limit)?;
        let body = bytes
            .get(at + 8..end)
            .ok_or(ExactReadError::Invalid("ilst atom exceeds its parent"))?;
        atoms.push((
            header[4..8]
                .try_into()
                .map_err(|_| ExactReadError::Invalid("ilst identifier"))?,
            body,
        ));
        if atoms.len() > MAX_ENTRIES {
            return Err(ExactReadError::Limit);
        }
        at = end;
    }
    Ok(atoms)
}
fn validate_ilst(bytes: &[u8]) -> Result<(), ExactReadError> {
    let mut total_values = 0usize;
    let mut total_text_bytes = 0usize;
    for (key, body) in atoms(bytes)? {
        let mut item_values = 0usize;
        for (kind, data) in atoms(body)? {
            if key == *b"----" && matches!(&kind, b"mean" | b"name") {
                let name = data
                    .get(4..)
                    .ok_or(ExactReadError::Invalid("MP4 namespace header"))?;
                if name.len() > MAX_NAME {
                    return Err(ExactReadError::Limit);
                }
            }
            if kind != *b"data" {
                continue;
            }
            let payload = data
                .get(8..)
                .ok_or(ExactReadError::Invalid("Truncated MP4 data qualifier"))?;
            let code = u32::from_be_bytes(
                data[..4]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("MP4 data kind"))?,
            );
            if !matches!(code, 1 | 2) {
                continue;
            }
            // Native snapshot limits already reject these fields. Apply them
            // before the compatibility decoder can allocate expanded strings.
            if payload.len() > MAX_FIELD_BYTES {
                return Err(ExactReadError::Limit);
            }
            let decoded = if code == 1 {
                std::str::from_utf8(payload)
                    .map_err(|_| ExactReadError::Invalid("MP4 UTF-8"))?
                    .len()
            } else {
                let (words, remainder) = payload.as_chunks::<2>();
                if !remainder.is_empty() {
                    return Err(ExactReadError::Invalid("Odd MP4 UTF-16 length"));
                }
                let words = words.iter().map(|word| u16::from_be_bytes(*word));
                let mut length = 0usize;
                for ch in char::decode_utf16(words) {
                    let ch =
                        ch.map_err(|_| ExactReadError::Invalid("Unpaired MP4 UTF-16 surrogate"))?;
                    length = length
                        .checked_add(ch.len_utf8())
                        .filter(|length| *length <= MAX_FIELD_BYTES)
                        .ok_or(ExactReadError::Limit)?;
                }
                length
            };
            item_values += 1;
            total_values += 1;
            total_text_bytes = total_text_bytes
                .checked_add(decoded)
                .filter(|bytes| *bytes <= MAX_SNAPSHOT_BYTES)
                .ok_or(ExactReadError::Limit)?;
            if item_values > 1024 || total_values > 16_384 {
                return Err(ExactReadError::Limit);
            }
        }
    }
    Ok(())
}
