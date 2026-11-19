//! Selective, pathless edits of a trusted disposable candidate.
//!
//! This is not an Apply/Undo entry point. Executor, protected publication and
//! exact journal integration are required before any live-media use. Exact
//! prepared intents deliberately remain `executable() == false`.
use super::{
    exact::{
        ExactReadPolicy, ExactTextState, Id3TextEncoding, Mp4TextKey, Mp4TextKind,
        NativeTextContext, NativeTextTarget, PreparedTextPatch,
    },
    AudioFormat, Id3v2Revision, TagBlockKind,
};
use crate::resources::{Budget, Charge, Lease};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    ops::Range,
};

pub mod codec;

const META: usize = 16 * 1024 * 1024;
const ITEMS: usize = 4096;
const BUFFER: usize = 64 * 1024;
const FILE_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
type Hash = [u8; 32];
fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unsupported native candidate profile: {0}")]
    Unsupported(&'static str),
    #[error("Invalid native candidate metadata: {0}")]
    Invalid(&'static str),
    #[error("Native candidate metadata, context or policy changed")]
    Changed,
    #[error("Native target or restore anchor is ambiguous")]
    Ambiguous,
    #[error("Native candidate limits exceeded")]
    Limit,
    #[error("Native candidate editing cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Resource(#[from] crate::resources::AdmissionError),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Scan,
    ReadChunk,
    BeforeMutation,
    Resized,
    Written(u64),
    Moved,
    AfterMutation,
    Verified,
}
pub trait Control {
    fn event(&mut self, _event: Event) -> io::Result<()> {
        Ok(())
    }
    fn cancelled(&mut self) -> bool {
        false
    }
    fn write_limit(&mut self, requested: usize) -> io::Result<usize> {
        Ok(requested)
    }
}
impl Control for () {}
#[derive(Clone, PartialEq, Eq)]
struct Witness {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    change: (i64, i64),
    #[cfg(windows)]
    change: i64,
}
fn witness(file: &File) -> Result<Witness> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return Err(Error::Limit);
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(windows)]
    let change = {
        use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        };
        let mut info = MaybeUninit::<FILE_BASIC_INFO>::uninit();
        // SAFETY: the borrowed File retains the handle; the correctly sized
        // output is read only after the API reports initialization.
        unsafe {
            if GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            ) == 0
            {
                return Err(Error::Unsupported("native change witness"));
            }
            info.assume_init().ChangeTime
        }
    };
    #[cfg(not(any(unix, windows)))]
    return Err(Error::Unsupported("native change witness"));
    #[cfg(any(unix, windows))]
    Ok(Witness {
        len: metadata.len(),
        modified: metadata.modified()?,
        #[cfg(unix)]
        change: (metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(windows)]
        change,
    })
}
pub(crate) const WORKING_BYTES: usize = 128 * 1024 * 1024;
struct Session<'a> {
    file: &'a mut File,
    control: &'a mut dyn Control,
    expected: Witness,
    policy: ExactReadPolicy,
}
impl Session<'_> {
    fn check(&self) -> Result<()> {
        if witness(self.file)? != self.expected
            || ExactReadPolicy::capture() != self.policy
            || super::read_policy_revision() != self.policy.revision
        {
            return Err(Error::Changed);
        }
        Ok(())
    }
    fn step(&mut self, event: Event) -> Result<()> {
        self.control.event(event)?;
        let cancelled = self.control.cancelled();
        self.check()?;
        if cancelled {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    fn read(&mut self, at: u64, len: usize) -> Result<Vec<u8>> {
        if len > META
            || at
                .checked_add(len as u64)
                .is_none_or(|end| end > self.expected.len)
        {
            return Err(Error::Invalid("metadata extent"));
        }
        let mut bytes = vec![0; len];
        self.file.seek(SeekFrom::Start(at))?;
        self.file.read_exact(&mut bytes)?;
        self.step(Event::Scan)?;
        Ok(bytes)
    }
    fn fixed<const N: usize>(&mut self, at: u64) -> Result<[u8; N]> {
        if at
            .checked_add(N as u64)
            .is_none_or(|end| end > self.expected.len)
        {
            return Err(Error::Invalid("header extent"));
        }
        let mut bytes = [0; N];
        self.file.seek(SeekFrom::Start(at))?;
        self.file.read_exact(&mut bytes)?;
        self.check()?;
        Ok(bytes)
    }
    fn region(&mut self, range: Range<u64>) -> Result<Hash> {
        if range.start > range.end || range.end > self.expected.len {
            return Err(Error::Invalid("digest extent"));
        }
        let mut at = range.start;
        let mut buffer = [0; BUFFER];
        let mut digest = Sha256::new();
        while at < range.end {
            self.step(Event::ReadChunk)?;
            let n = (range.end - at).min(BUFFER as u64) as usize;
            self.file.seek(SeekFrom::Start(at))?;
            self.file.read_exact(&mut buffer[..n])?;
            digest.update(&buffer[..n]);
            at += n as u64;
        }
        self.check()?;
        Ok(digest.finalize().into())
    }
    fn write(&mut self, mut at: u64, bytes: &[u8]) -> Result<()> {
        let mut done = 0;
        while done < bytes.len() {
            let requested = (bytes.len() - done).min(BUFFER);
            let n = self.control.write_limit(requested)?.min(requested);
            self.check()?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero).into());
            }
            self.file.seek(SeekFrom::Start(at))?;
            let written = self.file.write(&bytes[done..done + n])?;
            if written == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero).into());
            }
            self.expected = witness(self.file)?;
            done += written;
            at += written as u64;
            self.step(Event::Written(written as u64))?;
        }
        Ok(())
    }
    fn resize(&mut self, len: u64) -> Result<()> {
        self.check()?;
        self.file.set_len(len)?;
        self.expected = witness(self.file)?;
        self.step(Event::Resized)
    }
}
#[derive(Clone)]
struct Node {
    raw: Vec<u8>,
    key: Hash,
    selected: bool,
    values: Vec<String>,
}
enum Layout {
    Flac {
        flag: u8,
        vendor: Vec<u8>,
    },
    Id3 {
        header: [u8; 10],
        padding: Vec<u8>,
    },
    Mp4 {
        body: Range<usize>,
        ancestors: Vec<AtomSpan>,
        resize: bool,
    },
}
struct Document {
    offset: u64,
    raw: Vec<u8>,
    nodes: Vec<Node>,
    layout: Layout,
    revision: Option<Id3v2Revision>,
    digest: Hash,
}
impl Document {
    fn state(&self) -> Result<ExactTextState> {
        let selected: Vec<_> = self.nodes.iter().filter(|n| n.selected).collect();
        if selected.is_empty() {
            return Ok(ExactTextState::absent());
        }
        let values: Vec<String> = selected
            .into_iter()
            .flat_map(|n| n.values.iter().cloned())
            .collect();
        ExactTextState::try_from(values).map_err(|_| Error::Limit)
    }
    fn target_raw(&self) -> Vec<Vec<u8>> {
        self.nodes
            .iter()
            .filter(|n| n.selected)
            .map(|n| n.raw.clone())
            .collect()
    }
    fn encode(&self, nodes: &[Node]) -> Result<Vec<u8>> {
        let size = nodes
            .iter()
            .try_fold(0usize, |n, node| n.checked_add(node.raw.len()))
            .ok_or(Error::Limit)?;
        if size > META || nodes.len() > ITEMS {
            return Err(Error::Limit);
        }
        let mut out = Vec::new();
        match &self.layout {
            Layout::Flac { flag, vendor } => {
                out.push(*flag);
                out.extend([0; 3]);
                out.extend(vendor);
                out.extend((nodes.len() as u32).to_le_bytes());
                for node in nodes {
                    out.extend(&node.raw);
                }
                let len = out.len() - 4;
                if len > 0xff_ffff {
                    return Err(Error::Limit);
                }
                out[1..4].copy_from_slice(&(len as u32).to_be_bytes()[1..]);
            }
            Layout::Id3 { header, padding } => {
                out.extend(header);
                for node in nodes {
                    out.extend(&node.raw);
                }
                out.extend(padding);
                let length = out.len() - 10;
                out[6..10].copy_from_slice(&syncsafe_encode(length)?);
            }
            Layout::Mp4 {
                body,
                ancestors,
                resize,
            } => {
                out.extend(&self.raw[..body.start]);
                for node in nodes {
                    out.extend(&node.raw);
                }
                out.extend(&self.raw[body.end..]);
                if out.len() != self.raw.len() && !*resize {
                    return Err(Error::Unsupported("front moov relocation"));
                }
                let delta = out.len() as i64 - self.raw.len() as i64;
                for atom in ancestors {
                    let size = (atom.size as i64)
                        .checked_add(delta)
                        .and_then(|n| u64::try_from(n).ok())
                        .ok_or(Error::Limit)?;
                    if atom.header == 8 {
                        let value = u32::try_from(size).map_err(|_| Error::Limit)?;
                        out[atom.start..atom.start + 4].copy_from_slice(&value.to_be_bytes());
                    } else {
                        out[atom.start + 8..atom.start + 16].copy_from_slice(&size.to_be_bytes());
                    }
                }
            }
        }
        if out.len() > META {
            return Err(Error::Limit);
        }
        Ok(out)
    }
}
fn u32le(bytes: &[u8]) -> Result<usize> {
    Ok(u32::from_le_bytes(
        bytes
            .try_into()
            .map_err(|_| Error::Invalid("little-endian length"))?,
    ) as usize)
}
fn u32be(bytes: &[u8]) -> Result<usize> {
    Ok(u32::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| Error::Invalid("big-endian length"))?,
    ) as usize)
}
fn syncsafe(bytes: &[u8]) -> Result<usize> {
    if bytes.len() != 4 || bytes.iter().any(|v| v & 128 != 0) {
        return Err(Error::Invalid("syncsafe length"));
    }
    Ok(bytes.iter().fold(0, |n, v| (n << 7) | *v as usize))
}
fn syncsafe_encode(n: usize) -> Result<[u8; 4]> {
    if n >= 1 << 28 {
        return Err(Error::Limit);
    }
    Ok([
        ((n >> 21) & 127) as u8,
        ((n >> 14) & 127) as u8,
        ((n >> 7) & 127) as u8,
        (n & 127) as u8,
    ])
}
fn text(bytes: &[u8], encoding: Id3TextEncoding) -> Result<String> {
    if bytes.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    let value = match encoding {
        Id3TextEncoding::Utf8 => std::str::from_utf8(bytes)
            .map_err(|_| Error::Invalid("UTF-8 text"))?
            .to_owned(),
        Id3TextEncoding::Latin1 => bytes.iter().map(|v| char::from(*v)).collect(),
    };
    if value.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    Ok(value)
}
fn text_bytes(value: &str, encoding: Id3TextEncoding) -> Result<Vec<u8>> {
    match encoding {
        Id3TextEncoding::Utf8 => Ok(value.as_bytes().to_vec()),
        Id3TextEncoding::Latin1 => value
            .chars()
            .map(|ch| {
                u8::try_from(u32::from(ch))
                    .map_err(|_| Error::Unsupported("Latin1 representability"))
            })
            .collect(),
    }
}

fn flac(
    session: &mut Session<'_>,
    context: &NativeTextContext,
    target: &NativeTextTarget,
) -> Result<Document> {
    let NativeTextTarget::Vorbis { .. } = target else {
        return Err(Error::Unsupported("target/container mismatch"));
    };
    if session.fixed::<4>(0)? != *b"fLaC" {
        return Err(Error::Unsupported("ordinary FLAC marker"));
    }
    let mut at = 4;
    let mut visits = 0;
    let mut metadata = 0usize;
    let mut selected = None;
    loop {
        session.step(Event::Scan)?;
        visits += 1;
        if visits > 65536 {
            return Err(Error::Limit);
        }
        let header = session.fixed::<4>(at)?;
        let len = u32be(&[0, header[1], header[2], header[3]])?;
        metadata = metadata
            .checked_add(len + 4)
            .filter(|n| *n <= META)
            .ok_or(Error::Limit)?;
        let kind = header[0] & 127;
        if kind == 127 || (at == 4 && (kind != 0 || len != 34)) || (at != 4 && kind == 0) {
            return Err(Error::Invalid("FLAC block chain"));
        }
        let end = at.checked_add(4 + len as u64).ok_or(Error::Limit)?;
        if end > session.expected.len {
            return Err(Error::Invalid("FLAC block extent"));
        }
        if kind == 4 {
            if selected.is_some() {
                return Err(Error::Ambiguous);
            }
            let raw = session.read(at, len + 4)?;
            let body = &raw[4..];
            if body.len() < 8 {
                return Err(Error::Invalid("Vorbis header"));
            }
            let vendor_len = u32le(&body[..4])?;
            if vendor_len > 1024 * 1024 {
                return Err(Error::Limit);
            }
            let count_at = 4usize.checked_add(vendor_len).ok_or(Error::Limit)?;
            let count = u32le(
                body.get(count_at..count_at + 4)
                    .ok_or(Error::Invalid("Vorbis vendor"))?,
            )?;
            if count > ITEMS {
                return Err(Error::Limit);
            }
            let vendor = body[..count_at].to_vec();
            let mut pos = count_at + 4;
            let mut nodes = Vec::new();
            for _ in 0..count {
                session.step(Event::Scan)?;
                let start = pos;
                let length = u32le(
                    body.get(pos..pos + 4)
                        .ok_or(Error::Invalid("Vorbis record length"))?,
                )?;
                pos += 4;
                pos = pos.checked_add(length).ok_or(Error::Limit)?;
                let record = body
                    .get(start..pos)
                    .ok_or(Error::Invalid("Vorbis record extent"))?;
                nodes.push(parse_target_record(context, target, record)?);
            }
            if pos != body.len() {
                return Err(Error::Invalid("Vorbis suffix"));
            }
            let digest = hash(body);
            selected = Some(Document {
                offset: at,
                raw,
                nodes,
                layout: Layout::Flac {
                    flag: header[0],
                    vendor,
                },
                revision: None,
                digest,
            });
        }
        at = end;
        if header[0] & 128 != 0 {
            break;
        }
    }
    let sync = session.fixed::<2>(at)?;
    if sync[0] != 0xff || sync[1] & 0xfe != 0xf8 {
        return Err(Error::Unsupported("FLAC audio following metadata"));
    }
    selected.ok_or(Error::Unsupported("existing Vorbis block"))
}
type Id3Prefix<'a> = (Id3TextEncoding, Option<[u8; 3]>, Option<&'a [u8]>, usize);
fn id3_prefix<'a>(frame: &str, body: &'a [u8], revision: Id3v2Revision) -> Option<Id3Prefix<'a>> {
    let encoding = match body.first()? {
        0 => Id3TextEncoding::Latin1,
        3 if revision == Id3v2Revision::V4 => Id3TextEncoding::Utf8,
        _ => return None,
    };
    let mut at = 1;
    let language = if matches!(frame, "COMM" | "USLT") {
        let lang = body.get(at..at + 3)?.try_into().ok()?;
        at += 3;
        Some(lang)
    } else {
        None
    };
    let description = if frame == "TXXX" || language.is_some() {
        let n = body.get(at..)?.iter().position(|b| *b == 0)?;
        let name = body.get(at..at + n)?;
        at += n + 1;
        Some(name)
    } else {
        None
    };
    Some((encoding, language, description, at))
}
fn id3(
    session: &mut Session<'_>,
    context: &NativeTextContext,
    target: &NativeTextTarget,
) -> Result<Document> {
    let format = context.container;
    let header = session.fixed::<10>(0)?;
    if &header[..3] != b"ID3" || header[4] != 0 || header[5] != 0 {
        return Err(Error::Unsupported("unflagged leading ID3"));
    }
    let revision = match header[3] {
        3 => Id3v2Revision::V3,
        4 => Id3v2Revision::V4,
        _ => return Err(Error::Unsupported("ID3 revision")),
    };
    if context.id3_revision != Some(revision) {
        return Err(Error::Changed);
    }
    let size = syncsafe(&header[6..10])?;
    let raw = session.read(0, size.checked_add(10).ok_or(Error::Limit)?)?;
    let audio = session.fixed::<4>(raw.len() as u64)?;
    let valid = match format {
        AudioFormat::Mp3 => {
            audio[0] == 0xff
                && audio[1] & 0xe0 == 0xe0
                && audio[1] & 6 != 0
                && audio[1] & 0x18 != 8
                && audio[2] >> 4 != 0
                && audio[2] >> 4 != 15
                && audio[2] & 12 != 12
        }
        AudioFormat::Aac => audio[0] == 0xff && audio[1] & 0xf6 == 0xf0,
        _ => false,
    };
    if !valid {
        return Err(Error::Unsupported("declared leading audio container"));
    }
    let mut at = 10;
    let mut nodes = Vec::new();
    let mut selected_seen = false;
    while at < raw.len() && raw[at] != 0 {
        session.step(Event::Scan)?;
        if nodes.len() >= ITEMS {
            return Err(Error::Limit);
        }
        let h = raw
            .get(at..at + 10)
            .ok_or(Error::Invalid("ID3 frame header"))?;
        if h[..4]
            .iter()
            .any(|b| !b.is_ascii_uppercase() && !b.is_ascii_digit())
        {
            return Err(Error::Invalid("frame identifier"));
        }
        if h[8..] != [0, 0] {
            return Err(Error::Unsupported("ID3 frame flags"));
        }
        let n = if revision == Id3v2Revision::V4 {
            syncsafe(&h[4..8])?
        } else {
            u32be(&h[4..8])?
        };
        let end = at
            .checked_add(10)
            .and_then(|n0| n0.checked_add(n))
            .ok_or(Error::Limit)?;
        let record = raw.get(at..end).ok_or(Error::Invalid("ID3 frame extent"))?;
        let node = parse_target_record(context, target, record)?;
        if node.selected {
            if selected_seen {
                return Err(Error::Ambiguous);
            }
            selected_seen = true;
        }
        nodes.push(node);
        at = end;
    }
    if raw[at..].iter().any(|b| *b != 0) {
        return Err(Error::Invalid("ID3 padding"));
    }
    let padding = raw[at..].to_vec();
    let digest = hash(&raw[10..]);
    Ok(Document {
        offset: 0,
        raw,
        nodes,
        layout: Layout::Id3 { header, padding },
        revision: Some(revision),
        digest,
    })
}
#[derive(Clone, Copy)]
struct AtomSpan {
    start: usize,
    size: usize,
    header: usize,
    kind: [u8; 4],
}
fn atoms(bytes: &[u8], extended: bool) -> Result<Vec<AtomSpan>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        if out.len() >= 65536 {
            return Err(Error::Limit);
        }
        let head = bytes.get(at..at + 8).ok_or(Error::Invalid("atom header"))?;
        let size = u32be(&head[..4])?;
        let (size, header) = match size {
            0 => return Err(Error::Unsupported("open-ended atom")),
            1 if extended => {
                let size = u64::from_be_bytes(
                    bytes
                        .get(at + 8..at + 16)
                        .ok_or(Error::Invalid("extended atom"))?
                        .try_into()
                        .map_err(|_| Error::Invalid("extended size"))?,
                );
                (usize::try_from(size).map_err(|_| Error::Limit)?, 16)
            }
            1 => return Err(Error::Unsupported("extended ilst atom")),
            n => (n, 8),
        };
        if size < header || at.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err(Error::Invalid("atom extent"));
        }
        out.push(AtomSpan {
            start: at,
            size,
            header,
            kind: head[4..8]
                .try_into()
                .map_err(|_| Error::Invalid("atom kind"))?,
        });
        at += size;
    }
    Ok(out)
}
fn atom(kind: [u8; 4], body: &[u8]) -> Result<Vec<u8>> {
    let size = body
        .len()
        .checked_add(8)
        .filter(|n| *n <= META)
        .ok_or(Error::Limit)?;
    let mut bytes = (size as u32).to_be_bytes().to_vec();
    bytes.extend(kind);
    bytes.extend(body);
    Ok(bytes)
}
fn mp4_key(fourcc: [u8; 4], body: &[u8]) -> Result<Mp4TextKey> {
    if fourcc != *b"----" {
        return Ok(Mp4TextKey::Fourcc(fourcc));
    }
    let spans = atoms(body, false)?;
    let descriptor = |kind: [u8; 4]| -> Result<String> {
        let mut matching = spans.iter().filter(|s| s.kind == kind);
        let span = matching
            .next()
            .ok_or(Error::Invalid("freeform descriptor"))?;
        if matching.next().is_some() {
            return Err(Error::Ambiguous);
        }
        let bytes = &body[span.start + 8..span.start + span.size];
        if bytes.len() < 4 || bytes[..4] != [0; 4] || bytes.len() > 1028 {
            return Err(Error::Unsupported("freeform descriptor shape"));
        }
        let value =
            std::str::from_utf8(&bytes[4..]).map_err(|_| Error::Invalid("descriptor UTF-8"))?;
        if value.is_empty() || value.contains('\0') {
            return Err(Error::Invalid("descriptor identity"));
        }
        Ok(value.into())
    };
    Ok(Mp4TextKey::Freeform {
        mean: descriptor(*b"mean")?,
        name: descriptor(*b"name")?,
    })
}
fn mp4_text(bytes: &[u8], kind: Mp4TextKind) -> Result<String> {
    if bytes.len()
        > match kind {
            Mp4TextKind::Utf16 => 2 * 1024 * 1024,
            _ => 1024 * 1024,
        }
    {
        return Err(Error::Limit);
    }
    let value = match kind {
        Mp4TextKind::Utf8 => std::str::from_utf8(bytes)
            .map_err(|_| Error::Invalid("MP4 UTF-8"))?
            .to_owned(),
        Mp4TextKind::Utf16 => {
            if !bytes.len().is_multiple_of(2) {
                return Err(Error::Invalid("MP4 UTF-16 length"));
            }
            char::decode_utf16(
                bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|word| u16::from_be_bytes([word[0], word[1]])),
            )
            .collect::<std::result::Result<String, _>>()
            .map_err(|_| Error::Invalid("MP4 UTF-16 surrogate"))?
        }
        Mp4TextKind::Other(_) => return Err(Error::Unsupported("MP4 nontext kind")),
    };
    if value.len() > 1024 * 1024 {
        return Err(Error::Limit);
    }
    Ok(value)
}
struct Movie {
    visits: usize,
    ilst: Option<(Range<usize>, Vec<AtomSpan>)>,
    chunks: usize,
    descriptions: usize,
    local: usize,
}
fn movie_tree(
    session: &mut Session<'_>,
    raw: &[u8],
    range: Range<usize>,
    ancestors: &mut Vec<AtomSpan>,
    movie: &mut Movie,
    media: &[Range<u64>],
) -> Result<()> {
    if ancestors.len() > 8 {
        return Err(Error::Limit);
    }
    for mut span in atoms(&raw[range.clone()], true)? {
        span.start += range.start;
        movie.visits += 1;
        if movie.visits > 65536 {
            return Err(Error::Limit);
        }
        session.step(Event::Scan)?;
        let body_range = span.start + span.header..span.start + span.size;
        let body = &raw[body_range.clone()];
        match &span.kind {
            b"moov" | b"udta" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" => {
                ancestors.push(span);
                movie_tree(session, raw, body_range, ancestors, movie, media)?;
                ancestors.pop();
            }
            b"meta" => {
                if body.len() < 4 || body[..4] != [0; 4] {
                    return Err(Error::Unsupported("meta full-box flags"));
                }
                ancestors.push(span);
                movie_tree(
                    session,
                    raw,
                    body_range.start + 4..body_range.end,
                    ancestors,
                    movie,
                    media,
                )?;
                ancestors.pop();
            }
            b"ilst" => {
                if movie.ilst.is_some() {
                    return Err(Error::Ambiguous);
                }
                let mut path = ancestors.clone();
                path.push(span);
                movie.ilst = Some((body_range, path));
            }
            b"stco" | b"co64" => {
                if body.len() < 8 || body[..4] != [0; 4] {
                    return Err(Error::Invalid("chunk table"));
                }
                let count = u32be(&body[4..8])?;
                let width = if span.kind == *b"stco" { 4 } else { 8 };
                if count.checked_mul(width).and_then(|n| n.checked_add(8)) != Some(body.len()) {
                    return Err(Error::Invalid("chunk count"));
                }
                for (index, chunk) in body[8..].chunks_exact(width).enumerate() {
                    if index % 1024 == 0 {
                        session.step(Event::Scan)?;
                    }
                    let offset = if width == 4 {
                        u32be(chunk)? as u64
                    } else {
                        u64::from_be_bytes(
                            chunk
                                .try_into()
                                .map_err(|_| Error::Invalid("chunk offset"))?,
                        )
                    };
                    let index = media.partition_point(|m| m.end <= offset);
                    if !media.get(index).is_some_and(|m| m.contains(&offset)) {
                        return Err(Error::Unsupported("nonlocal chunk offset"));
                    }
                }
                movie.chunks = movie.chunks.checked_add(count).ok_or(Error::Limit)?;
            }
            b"stsd" => {
                if body.len() < 8 || body[..4] != [0; 4] {
                    return Err(Error::Invalid("sample descriptions"));
                }
                let count = u32be(&body[4..8])?;
                if count > ITEMS {
                    return Err(Error::Limit);
                }
                let entries = atoms(&body[8..], false)?;
                if entries.len() != count {
                    return Err(Error::Invalid("sample description count"));
                }
                for entry in entries {
                    let description = &body[8 + entry.start + 8..8 + entry.start + entry.size];
                    if !matches!(&entry.kind, b"mp4a" | b"alac")
                        || description.len() < 28
                        || description[6..8] != [0, 1]
                        || description[8..10] != [0, 0]
                    {
                        return Err(Error::Unsupported(
                            "ordinary local audio sample description",
                        ));
                    }
                }
                movie.descriptions += count;
            }
            b"dref" => {
                if body.len() != 20
                    || body[..8] != [0, 0, 0, 0, 0, 0, 0, 1]
                    || body[8..] != [0, 0, 0, 12, b'u', b'r', b'l', b' ', 0, 0, 0, 1]
                {
                    return Err(Error::Unsupported("local data reference"));
                }
                movie.local += 1;
            }
            b"stsc" | b"stts" | b"ctts" | b"stss" => {
                if body.len() < 8 || body[..4] != [0; 4] {
                    return Err(Error::Unsupported("classic sample table flags"));
                }
                let width = match &span.kind {
                    b"stsc" => 12,
                    b"stss" => 4,
                    _ => 8,
                };
                let count = u32be(&body[4..8])?;
                if count.checked_mul(width).and_then(|n| n.checked_add(8)) != Some(body.len()) {
                    return Err(Error::Invalid("sample table count"));
                }
            }
            b"stsz" => {
                if body.len() < 12 || body[..4] != [0; 4] {
                    return Err(Error::Invalid("sample size table"));
                }
                let size = u32be(&body[4..8])?;
                let count = u32be(&body[8..12])?;
                let wanted = if size == 0 {
                    count
                        .checked_mul(4)
                        .and_then(|n| n.checked_add(12))
                        .ok_or(Error::Limit)?
                } else {
                    12
                };
                if body.len() != wanted {
                    return Err(Error::Invalid("sample size count"));
                }
                let total = if size == 0 {
                    let mut total = 0u64;
                    for (index, b) in body[12..].as_chunks::<4>().0.iter().enumerate() {
                        if index % 1024 == 0 {
                            session.step(Event::Scan)?;
                        }
                        total = total.checked_add(u32be(b)? as u64).ok_or(Error::Limit)?;
                    }
                    total
                } else {
                    (size as u64)
                        .checked_mul(count as u64)
                        .ok_or(Error::Limit)?
                };
                if total > session.expected.len {
                    return Err(Error::Invalid("sample sizes exceed file"));
                }
            }
            b"cmov" | b"moof" | b"mvex" | b"saio" | b"saiz" | b"iloc" | b"sidx" | b"tfhd"
            | b"trun" | b"sinf" | b"senc" => {
                return Err(Error::Unsupported(
                    "compressed/fragmented/protected or relocatable offsets",
                ))
            }
            _ => {}
        }
    }
    Ok(())
}
fn mp4(
    session: &mut Session<'_>,
    context: &NativeTextContext,
    target: &NativeTextTarget,
) -> Result<Document> {
    let mut at = 0u64;
    let mut visits = 0;
    let mut moov = None;
    let mut media = Vec::new();
    let mut ftyp = false;
    while at < session.expected.len {
        session.step(Event::Scan)?;
        visits += 1;
        if visits > 65536 {
            return Err(Error::Limit);
        }
        let header = session.fixed::<8>(at)?;
        let size = u32be(&header[..4])?;
        let (size, width) = match size {
            0 => return Err(Error::Unsupported("open-ended top atom")),
            1 => (u64::from_be_bytes(session.fixed::<8>(at + 8)?), 16),
            n => (n as u64, 8),
        };
        let next = at
            .checked_add(size)
            .filter(|n| *n <= session.expected.len)
            .ok_or(Error::Invalid("top atom extent"))?;
        if size < width {
            return Err(Error::Invalid("top atom size"));
        }
        match &header[4..8] {
            b"ftyp" => {
                if ftyp || !(16..=4096).contains(&size) {
                    return Err(Error::Unsupported("file type box"));
                }
                ftyp = true;
            }
            b"mdat" => {
                if media.len() >= ITEMS || size == width {
                    return Err(Error::Limit);
                }
                media.push(at + width..next);
            }
            b"moov" => {
                if moov.is_some() {
                    return Err(Error::Ambiguous);
                }
                if size > META as u64 {
                    return Err(Error::Limit);
                }
                moov = Some((at, size as usize, width as usize));
            }
            b"moof" | b"sidx" | b"mfra" | b"styp" => {
                return Err(Error::Unsupported("fragmented movie"))
            }
            _ => {}
        }
        at = next;
    }
    if !ftyp || media.is_empty() {
        return Err(Error::Unsupported("classic local MP4"));
    }
    let (offset, size, width) = moov.ok_or(Error::Unsupported("existing moov"))?;
    let raw = session.read(offset, size)?;
    let root = AtomSpan {
        start: 0,
        size,
        header: width,
        kind: *b"moov",
    };
    let mut movie = Movie {
        visits: 0,
        ilst: None,
        chunks: 0,
        descriptions: 0,
        local: 0,
    };
    movie_tree(
        session,
        &raw,
        width..size,
        &mut vec![root],
        &mut movie,
        &media,
    )?;
    if movie.chunks == 0 || movie.descriptions == 0 || movie.local == 0 {
        return Err(Error::Unsupported("classic local audio tables"));
    }
    let (body_range, ancestors) = movie.ilst.ok_or(Error::Unsupported("existing ilst"))?;
    let body = &raw[body_range.clone()];
    let mut nodes = Vec::new();
    for span in atoms(body, false)? {
        if nodes.len() >= ITEMS {
            return Err(Error::Limit);
        }
        session.step(Event::Scan)?;
        nodes.push(parse_target_record(
            context,
            target,
            &body[span.start..span.start + span.size],
        )?);
    }
    if nodes.iter().filter(|n| n.selected).count() > 1 {
        return Err(Error::Ambiguous);
    }
    let digest = hash(body);
    Ok(Document {
        offset,
        raw,
        nodes,
        layout: Layout::Mp4 {
            body: body_range,
            ancestors,
            resize: offset + size as u64 == session.expected.len,
        },
        revision: None,
        digest,
    })
}

fn document(
    session: &mut Session<'_>,
    context: &NativeTextContext,
    target: &NativeTextTarget,
) -> Result<Document> {
    target
        .validate()
        .map_err(|_| Error::Invalid("native target"))?;
    match context.container {
        AudioFormat::Flac if context.selected_block == Some(TagBlockKind::VorbisComments) => {
            flac(session, context, target)
        }
        AudioFormat::Mp3 | AudioFormat::Aac
            if context.selected_block == Some(TagBlockKind::Id3v2) =>
        {
            id3(session, context, target)
        }
        AudioFormat::M4a if context.selected_block == Some(TagBlockKind::Mp4Ilst) => {
            mp4(session, context, target)
        }
        _ => Err(Error::Unsupported("native container adapter")),
    }
}
/// Derive identity, selection and values from one complete native record.
/// Kept pure so future receipt decoding uses the exact scanner parser rather
/// than accepting serialized identity/value assertions. Callers separately
/// validate whole-container structure, aggregate limits and target uniqueness.
fn parse_target_record(
    context: &NativeTextContext,
    target: &NativeTextTarget,
    raw: &[u8],
) -> Result<Node> {
    if raw.len() > META {
        return Err(Error::Limit);
    }
    target
        .validate()
        .map_err(|_| Error::Invalid("native target"))?;
    match (context.container, context.selected_block, target) {
        (
            AudioFormat::Flac,
            Some(TagBlockKind::VorbisComments),
            NativeTextTarget::Vorbis { name: wanted },
        ) => {
            let length = u32le(raw.get(..4).ok_or(Error::Invalid("Vorbis record length"))?)?;
            if length != raw.len() - 4 {
                return Err(Error::Invalid("Vorbis record extent"));
            }
            let item = &raw[4..];
            let equals = item
                .iter()
                .position(|b| *b == b'=')
                .ok_or(Error::Invalid("Vorbis separator"))?;
            let name = &item[..equals];
            if name.is_empty()
                || name.len() > 1024
                || name
                    .iter()
                    .any(|b| !(0x20..=0x7d).contains(b) || *b == b'=')
            {
                return Err(Error::Invalid("Vorbis name"));
            }
            let same = name.eq_ignore_ascii_case(wanted.as_bytes());
            if same && name != wanted.as_bytes() {
                return Err(Error::Ambiguous);
            }
            let values = if same {
                vec![std::str::from_utf8(&item[equals + 1..])
                    .map_err(|_| Error::Invalid("Vorbis text"))?
                    .into()]
            } else {
                Vec::new()
            };
            Ok(Node {
                raw: raw.to_vec(),
                key: hash(name),
                selected: same,
                values,
            })
        }
        (
            AudioFormat::Mp3 | AudioFormat::Aac,
            Some(TagBlockKind::Id3v2),
            NativeTextTarget::Id3 {
                frame: wanted,
                description: want_desc,
                language: want_lang,
                encoding: want_encoding,
            },
        ) => {
            let revision = context
                .id3_revision
                .ok_or(Error::Unsupported("ID3 revision"))?;
            if *want_encoding == Some(Id3TextEncoding::Latin1)
                && context.read_policy.legacy_code_page.is_some()
            {
                return Err(Error::Unsupported("configured Latin1 codepage"));
            }
            let h = raw.get(..10).ok_or(Error::Invalid("ID3 frame header"))?;
            if h[..4]
                .iter()
                .any(|b| !b.is_ascii_uppercase() && !b.is_ascii_digit())
            {
                return Err(Error::Invalid("frame identifier"));
            }
            if h[8..] != [0, 0] {
                return Err(Error::Unsupported("ID3 frame flags"));
            }
            let length = if revision == Id3v2Revision::V4 {
                syncsafe(&h[4..8])?
            } else {
                u32be(&h[4..8])?
            };
            if length != raw.len() - 10 {
                return Err(Error::Invalid("ID3 frame extent"));
            }
            let body = &raw[10..];
            let frame = std::str::from_utf8(&h[..4]).map_err(|_| Error::Invalid("frame name"))?;
            let prefix = if frame.starts_with('T') || matches!(frame, "COMM" | "USLT") {
                id3_prefix(frame, body, revision)
            } else {
                None
            };
            let mut key = frame.as_bytes().to_vec();
            if let Some((_, _, _, prefix_len)) = &prefix {
                key.extend(&body[..*prefix_len]);
            } else if frame.starts_with('T') && frame != "TXXX" {
                key.extend(body.get(..1).unwrap_or_default());
            } else {
                key.extend(hash(body));
            }
            let mut selected = false;
            let mut values = Vec::new();
            if frame == wanted {
                let (encoding, lang, desc, prefix_len) =
                    prefix.ok_or(Error::Unsupported("target ID3 encoding/qualifier"))?;
                let desc = desc.map(|bytes| text(bytes, encoding)).transpose()?;
                if lang == *want_lang && desc == *want_desc {
                    if Some(encoding) != *want_encoding {
                        return Err(Error::Unsupported("target encoding mismatch"));
                    }
                    selected = true;
                    let value = text(&body[prefix_len..], encoding)?;
                    if revision == Id3v2Revision::V4 && !matches!(frame, "COMM" | "USLT") {
                        // Count before expanding NUL-dense text into owned
                        // String headers. Duplicated stale frames are parsed
                        // before whole-document ambiguity can be detected.
                        if value.split('\0').take(1025).count() > 1024 {
                            return Err(Error::Limit);
                        }
                        values = value.split('\0').map(str::to_string).collect();
                    } else {
                        values = vec![value];
                    }
                }
            }
            Ok(Node {
                raw: raw.to_vec(),
                key: hash(&key),
                selected,
                values,
            })
        }
        (
            AudioFormat::M4a,
            Some(TagBlockKind::Mp4Ilst),
            NativeTextTarget::Mp4 {
                key: wanted,
                kind: want_kind,
                locale: want_locale,
            },
        ) => {
            let spans = atoms(raw, false)?;
            if spans.len() != 1 {
                return Err(Error::Invalid("single ilst record"));
            }
            let span = spans[0];
            let atom_raw = raw;
            let payload = &atom_raw[8..];
            let key = mp4_key(span.kind, payload)?;
            let same_key = &key == wanted;
            let mut identity = span.kind.to_vec();
            if let Mp4TextKey::Freeform { mean, name } = &key {
                identity.extend((mean.len() as u32).to_be_bytes());
                identity.extend(mean.as_bytes());
                identity.extend((name.len() as u32).to_be_bytes());
                identity.extend(name.as_bytes());
            }
            let mut qualifiers = Vec::new();
            let mut values = Vec::new();
            let mut selected = false;
            for child in atoms(payload, false)? {
                let bytes = &payload[child.start + 8..child.start + child.size];
                match &child.kind {
                    b"data" => {
                        if bytes.len() < 8 {
                            return Err(Error::Invalid("data qualifiers"));
                        }
                        let kind = match u32be(&bytes[..4])? {
                            1 => Mp4TextKind::Utf8,
                            2 => Mp4TextKind::Utf16,
                            n => Mp4TextKind::Other(n as u32),
                        };
                        let locale = u32be(&bytes[4..8])? as u32;
                        qualifiers.push(bytes[..8].to_vec());
                        if same_key {
                            if kind != *want_kind || locale != *want_locale {
                                return Err(Error::Unsupported("mixed targeted data qualifiers"));
                            }
                            selected = true;
                            values.push(mp4_text(&bytes[8..], kind)?);
                        }
                    }
                    b"mean" | b"name" if span.kind == *b"----" => {}
                    _ => {
                        if same_key {
                            return Err(Error::Unsupported("opaque targeted ilst children"));
                        }
                        identity.extend(hash(&payload[child.start..child.start + child.size]));
                    }
                }
            }
            qualifiers.sort();
            qualifiers.dedup();
            for qualifier in qualifiers {
                identity.extend(qualifier);
            }
            if same_key && !selected {
                return Err(Error::Unsupported("target atom without text data"));
            }
            Ok(Node {
                raw: atom_raw.to_vec(),
                key: hash(&identity),
                selected,
                values,
            })
        }
        _ => Err(Error::Unsupported("native record context")),
    }
}

fn generated(
    doc: &Document,
    target: &NativeTextTarget,
    state: &ExactTextState,
) -> Result<Vec<Node>> {
    let Some(values) = state.values() else {
        return Ok(Vec::new());
    };
    let values = values.as_slice();
    let mut result = Vec::new();
    match (&doc.layout, target) {
        (Layout::Flac { .. }, NativeTextTarget::Vorbis { name }) => {
            for value in values {
                let item = format!("{name}={value}");
                let mut raw = (item.len() as u32).to_le_bytes().to_vec();
                raw.extend(item.as_bytes());
                result.push(Node {
                    raw,
                    key: [0; 32],
                    selected: true,
                    values: vec![value.clone()],
                });
            }
        }
        (
            Layout::Id3 { .. },
            NativeTextTarget::Id3 {
                frame,
                description,
                language,
                encoding: Some(encoding),
            },
        ) => {
            if doc.revision == Some(Id3v2Revision::V3)
                && (*encoding != Id3TextEncoding::Latin1 || values.len() != 1)
            {
                return Err(Error::Unsupported("v2.3 scalar Latin1 target"));
            }
            if matches!(frame.as_str(), "COMM" | "USLT") && values.len() != 1 {
                return Err(Error::Unsupported("qualified scalar ID3 target"));
            }
            let mut body = vec![match encoding {
                Id3TextEncoding::Latin1 => 0,
                Id3TextEncoding::Utf8 => 3,
            }];
            if let Some(lang) = language {
                body.extend(lang);
            }
            if let Some(desc) = description {
                body.extend(text_bytes(desc, *encoding)?);
                body.push(0);
            }
            for (i, value) in values.iter().enumerate() {
                if i != 0 {
                    body.push(0);
                }
                body.extend(text_bytes(value, *encoding)?);
            }
            let mut raw = frame.as_bytes().to_vec();
            raw.extend(if doc.revision == Some(Id3v2Revision::V4) {
                syncsafe_encode(body.len())?
            } else {
                (body.len() as u32).to_be_bytes()
            });
            raw.extend([0, 0]);
            raw.extend(body);
            result.push(Node {
                raw,
                key: [0; 32],
                selected: true,
                values: values.to_vec(),
            });
        }
        (Layout::Mp4 { .. }, NativeTextTarget::Mp4 { key, kind, locale }) => {
            let mut data = Vec::new();
            for value in values {
                let bytes = match kind {
                    Mp4TextKind::Utf8 => value.as_bytes().to_vec(),
                    Mp4TextKind::Utf16 => value.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                    Mp4TextKind::Other(_) => return Err(Error::Unsupported("nontext MP4 data")),
                };
                let mut body = match kind {
                    Mp4TextKind::Utf8 => 1u32,
                    Mp4TextKind::Utf16 => 2u32,
                    Mp4TextKind::Other(_) => 0,
                }
                .to_be_bytes()
                .to_vec();
                body.extend(locale.to_be_bytes());
                body.extend(bytes);
                data.push(atom(*b"data", &body)?);
            }
            let existing = doc.nodes.iter().find(|n| n.selected);
            let mut body = Vec::new();
            let mut inserted = false;
            if let Some(existing) = existing {
                for span in atoms(&existing.raw[8..], false)? {
                    let raw = &existing.raw[8 + span.start..8 + span.start + span.size];
                    if span.kind == *b"data" {
                        if !inserted {
                            for value in &data {
                                body.extend(value);
                            }
                            inserted = true;
                        }
                    } else {
                        body.extend(raw);
                    }
                }
            } else if let Mp4TextKey::Freeform { mean, name } = key {
                for (kind, value) in [(*b"mean", mean), (*b"name", name)] {
                    let mut descriptor = vec![0; 4];
                    descriptor.extend(value.as_bytes());
                    body.extend(atom(kind, &descriptor)?);
                }
            }
            if !inserted {
                for value in data {
                    body.extend(value);
                }
            }
            let fourcc = match key {
                Mp4TextKey::Fourcc(f) => *f,
                Mp4TextKey::Freeform { .. } => *b"----",
            };
            result.push(Node {
                raw: atom(fourcc, &body)?,
                key: [0; 32],
                selected: true,
                values: values.to_vec(),
            });
        }
        _ => return Err(Error::Unsupported("target representation")),
    }
    Ok(result)
}
#[derive(Clone)]
struct Anchor {
    key: Hash,
    ordinal: usize,
    total: usize,
    grouped: bool,
}
#[derive(Clone)]
struct Slot {
    previous: Option<Anchor>,
    next: Option<Anchor>,
    nodes: Vec<Node>,
}
fn slots(nodes: &[Node]) -> Vec<Slot> {
    let mut totals = HashMap::new();
    for (index, node) in nodes.iter().filter(|n| !n.selected).enumerate() {
        let entry = totals.entry(node.key).or_insert((0usize, index, index));
        entry.0 += 1;
        entry.2 = index;
    }
    let mut counts = HashMap::new();
    let mut anchors = Vec::new();
    for node in nodes {
        if node.selected {
            anchors.push(None);
        } else {
            let ordinal = counts.entry(node.key).or_insert(0usize);
            anchors.push(Some(Anchor {
                key: node.key,
                ordinal: *ordinal,
                total: totals[&node.key].0,
                grouped: totals[&node.key].2 - totals[&node.key].1 + 1 == totals[&node.key].0,
            }));
            *ordinal += 1;
        }
    }
    let mut result = Vec::new();
    let mut at = 0;
    while at < nodes.len() {
        if !nodes[at].selected {
            at += 1;
            continue;
        }
        let start = at;
        while at < nodes.len() && nodes[at].selected {
            at += 1;
        }
        result.push(Slot {
            previous: start.checked_sub(1).and_then(|i| anchors[i].clone()),
            next: anchors.get(at).cloned().flatten(),
            nodes: nodes[start..at].to_vec(),
        });
    }
    result
}
fn restored(nodes: &[Node], old_slots: &[Slot]) -> Result<Vec<Node>> {
    let others: Vec<_> = nodes.iter().filter(|n| !n.selected).cloned().collect();
    let mut inserts: HashMap<usize, Vec<Node>> = HashMap::new();
    let mut positions: HashMap<Hash, Vec<usize>> = HashMap::new();
    for (i, node) in others.iter().enumerate() {
        positions.entry(node.key).or_default().push(i);
    }
    let resolve = |anchor: &Anchor, previous: bool| -> Result<usize> {
        let positions = positions.get(&anchor.key).ok_or(Error::Ambiguous)?;
        if anchor.grouped
            && positions.last().copied().unwrap_or(0) - positions[0] + 1 == positions.len()
        {
            if previous && anchor.ordinal + 1 == anchor.total {
                return positions.last().copied().ok_or(Error::Ambiguous);
            }
            if !previous && anchor.ordinal == 0 {
                return Ok(positions[0]);
            }
        }
        if positions.len() != anchor.total {
            return Err(Error::Ambiguous);
        }
        positions
            .get(anchor.ordinal)
            .copied()
            .ok_or(Error::Ambiguous)
    };
    for slot in old_slots {
        let previous = slot
            .previous
            .as_ref()
            .map(|a| resolve(a, true))
            .transpose()?;
        let next = slot.next.as_ref().map(|a| resolve(a, false)).transpose()?;
        if previous.zip(next).is_some_and(|(a, b)| a >= b) {
            return Err(Error::Ambiguous);
        }
        let at = previous.map(|p| p + 1).or(next).unwrap_or(others.len());
        inserts.entry(at).or_default().extend(slot.nodes.clone());
    }
    let mut result = Vec::new();
    for (i, node) in others.into_iter().enumerate() {
        if let Some(items) = inserts.remove(&i) {
            result.extend(items);
        }
        result.push(node);
    }
    if let Some(items) = inserts.remove(&result.iter().filter(|n| !n.selected).count()) {
        result.extend(items);
    }
    if !inserts.is_empty() {
        return Err(Error::Ambiguous);
    }
    Ok(result)
}
fn raw_digest(nodes: &[Vec<u8>]) -> Hash {
    let mut digest = Sha256::new();
    for raw in nodes {
        digest.update((raw.len() as u64).to_le_bytes());
        digest.update(raw);
    }
    digest.finalize().into()
}
fn untouched(nodes: &[Node]) -> Vec<&[u8]> {
    nodes
        .iter()
        .filter(|n| !n.selected)
        .map(|n| n.raw.as_slice())
        .collect()
}
pub struct Receipt {
    context: NativeTextContext,
    target: NativeTextTarget,
    old: ExactTextState,
    new: ExactTextState,
    slots: Vec<Slot>,
    after_raw: Vec<Vec<u8>>,
    before_hash: Hash,
    after_hash: Hash,
    _retained: Lease,
}
impl Receipt {
    pub fn context(&self) -> &NativeTextContext {
        &self.context
    }
    pub fn target(&self) -> &NativeTextTarget {
        &self.target
    }
    pub fn old(&self) -> &ExactTextState {
        &self.old
    }
    pub fn new_state(&self) -> &ExactTextState {
        &self.new
    }
    pub fn before_target_digest(&self) -> Hash {
        self.before_hash
    }
    pub fn after_target_digest(&self) -> Hash {
        self.after_hash
    }
}
enum Replacement<'a> {
    Semantic,
    Raw(&'a Receipt),
    #[cfg(target_os = "linux")]
    FlacValues(&'a Receipt, bool),
}
pub fn apply_prepared(
    file: &mut File,
    patch: &PreparedTextPatch,
    control: &mut dyn Control,
) -> Result<Receipt> {
    apply_prepared_with_budget(file, patch, Budget::process(), control)
}
pub fn apply_prepared_with_budget(
    file: &mut File,
    patch: &PreparedTextPatch,
    budget: Budget,
    control: &mut dyn Control,
) -> Result<Receipt> {
    run(
        file,
        Intent {
            context: patch.context(),
            target: patch.target(),
            old: patch.old(),
            new: patch.new_state(),
        },
        Replacement::Semantic,
        budget,
        control,
    )
}
pub fn restore(file: &mut File, receipt: &Receipt, control: &mut dyn Control) -> Result<Receipt> {
    restore_with_budget(file, receipt, Budget::process(), control)
}
pub fn restore_with_budget(
    file: &mut File,
    receipt: &Receipt,
    budget: Budget,
    control: &mut dyn Control,
) -> Result<Receipt> {
    run(
        file,
        Intent {
            context: &receipt.context,
            target: &receipt.target,
            old: &receipt.new,
            new: &receipt.old,
        },
        Replacement::Raw(receipt),
        budget,
        control,
    )
}
/// Executor-only FLAC conflict restoration: preserve historical raw target
/// records and current unrelated records without claiming their old positions.
/// The returned receipt captures the actual current target for rollback.
#[cfg(target_os = "linux")]
pub(crate) fn restore_flac_values(
    file: &mut File,
    receipt: &Receipt,
    context: &NativeTextContext,
    current: &ExactTextState,
    overwrite: bool,
    control: &mut dyn Control,
) -> Result<Receipt> {
    if context.container != AudioFormat::Flac
        || !matches!(receipt.target, NativeTextTarget::Vorbis { .. })
    {
        return Err(Error::Unsupported("FLAC value restoration"));
    }
    run(
        file,
        Intent {
            context,
            target: &receipt.target,
            old: current,
            new: &receipt.old,
        },
        Replacement::FlacValues(receipt, overwrite),
        Budget::process(),
        control,
    )
}
struct Intent<'a> {
    context: &'a NativeTextContext,
    target: &'a NativeTextTarget,
    old: &'a ExactTextState,
    new: &'a ExactTextState,
}
fn run(
    file: &mut File,
    intent: Intent<'_>,
    replacement: Replacement<'_>,
    budget: Budget,
    control: &mut dyn Control,
) -> Result<Receipt> {
    let Intent {
        context,
        target,
        old,
        new,
    } = intent;
    let _working = budget.try_acquire(Charge {
        transient: WORKING_BYTES,
        jobs: 1,
        ..Default::default()
    })?;
    if context.read_policy.multi_value_separator.len() > 1024
        || context.read_policy.read_priority.len() > 256
    {
        return Err(Error::Limit);
    }
    if context.adapter_version != 1
        || context.policy_revision != context.read_policy.revision
        || context.policy_revision == u64::MAX
        || context.policy_revision != super::read_policy_revision()
        || context.read_policy != ExactReadPolicy::capture()
    {
        return Err(Error::Changed);
    }
    #[cfg(unix)]
    {
        let flags = rustix::fs::fcntl_getfl(&*file).map_err(io::Error::from)?;
        if flags & rustix::fs::OFlags::ACCMODE != rustix::fs::OFlags::RDWR
            || flags.contains(rustix::fs::OFlags::APPEND)
        {
            return Err(Error::Unsupported("read/write nonappend candidate"));
        }
    }
    let expected = witness(file)?;
    let mut session = Session {
        file,
        control,
        expected,
        policy: context.read_policy.clone(),
    };
    session.step(Event::Scan)?;
    let doc = document(&mut session, context, target)?;
    if doc.revision != context.id3_revision || doc.state()? != *old {
        return Err(Error::Changed);
    }
    if !matches!(replacement, Replacement::Raw(_)) && context.raw_digest != Some(doc.digest) {
        return Err(Error::Changed);
    }
    if let Replacement::Raw(receipt) = &replacement {
        if doc.target_raw() != receipt.after_raw {
            return Err(Error::Changed);
        }
    }
    let before_raw = doc.target_raw();
    let old_slots = slots(&doc.nodes);
    let nodes = match replacement {
        Replacement::Semantic => {
            let generated = generated(&doc, target, new)?;
            let mut out = Vec::new();
            let mut inserted = false;
            for node in &doc.nodes {
                if node.selected {
                    if !inserted {
                        out.extend(generated.clone());
                        inserted = true;
                    }
                } else {
                    out.push(node.clone());
                }
            }
            if !inserted {
                out.extend(generated);
            }
            out
        }
        Replacement::Raw(receipt) => restored(&doc.nodes, &receipt.slots)?,
        #[cfg(target_os = "linux")]
        Replacement::FlacValues(receipt, overwrite) => {
            if !overwrite && doc.target_raw() != receipt.after_raw {
                return Err(Error::Changed);
            }
            let historical: Vec<Node> = receipt
                .slots
                .iter()
                .flat_map(|slot| slot.nodes.iter().cloned())
                .collect();
            let mut out = Vec::new();
            let mut inserted = false;
            for node in &doc.nodes {
                if node.selected {
                    if !inserted {
                        out.extend(historical.iter().cloned());
                        inserted = true;
                    }
                } else {
                    out.push(node.clone());
                }
            }
            if !inserted {
                out.extend(historical);
            }
            out
        }
    };
    let raw = doc.encode(&nodes)?;
    let after_raw: Vec<_> = nodes
        .iter()
        .filter(|n| n.selected)
        .map(|n| n.raw.clone())
        .collect();
    let state_bytes = |state: &ExactTextState| {
        state.values().map_or(0, |v| {
            v.as_slice()
                .iter()
                .map(|s| s.capacity() + 32)
                .sum::<usize>()
        })
    };
    let retained_bytes = old_slots
        .iter()
        .flat_map(|s| &s.nodes)
        .map(|n| n.raw.capacity() + 256 + n.values.iter().map(|s| s.capacity() + 32).sum::<usize>())
        .sum::<usize>()
        .saturating_add(after_raw.iter().map(Vec::capacity).sum::<usize>())
        .saturating_add(after_raw.capacity() * std::mem::size_of::<Vec<u8>>())
        .saturating_add(
            old_slots
                .iter()
                .map(|slot| slot.nodes.capacity() * std::mem::size_of::<Node>())
                .sum::<usize>(),
        )
        .saturating_add(state_bytes(old))
        .saturating_add(state_bytes(new))
        .saturating_add(old_slots.capacity() * 256)
        .saturating_add(16 * 1024);
    if retained_bytes > 16 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let retained = budget.try_acquire(Charge {
        retained: retained_bytes,
        ..Default::default()
    })?;
    let old_end = doc
        .offset
        .checked_add(doc.raw.len() as u64)
        .ok_or(Error::Limit)?;
    let old_len = session.expected.len;
    let new_len = old_len
        .checked_sub(doc.raw.len() as u64)
        .and_then(|n| n.checked_add(raw.len() as u64))
        .filter(|n| *n <= FILE_LIMIT)
        .ok_or(Error::Limit)?;
    let prefix = session.region(0..doc.offset)?;
    let suffix = session.region(old_end..old_len)?;
    session.step(Event::BeforeMutation)?;
    let mut buffer = [0; BUFFER];
    if new_len > old_len {
        let delta = new_len - old_len;
        session.resize(new_len)?;
        let mut end = old_len;
        while end > old_end {
            session.step(Event::Moved)?;
            let n = (end - old_end).min(BUFFER as u64) as usize;
            let from = end - n as u64;
            session.file.seek(SeekFrom::Start(from))?;
            session.file.read_exact(&mut buffer[..n])?;
            session.write(from + delta, &buffer[..n])?;
            end = from;
        }
    } else if new_len < old_len {
        let delta = old_len - new_len;
        let mut from = old_end;
        while from < old_len {
            session.step(Event::Moved)?;
            let n = (old_len - from).min(BUFFER as u64) as usize;
            session.file.seek(SeekFrom::Start(from))?;
            session.file.read_exact(&mut buffer[..n])?;
            session.write(from - delta, &buffer[..n])?;
            from += n as u64;
        }
    }
    session.write(doc.offset, &raw)?;
    if new_len < old_len {
        session.resize(new_len)?;
    }
    session.step(Event::AfterMutation)?;
    let after = document(&mut session, context, target)?;
    if after.raw != raw
        || after.state()? != *new
        || after.target_raw() != after_raw
        || untouched(&after.nodes) != untouched(&doc.nodes)
        || session.region(0..doc.offset)? != prefix
        || session.region(doc.offset + raw.len() as u64..new_len)? != suffix
    {
        return Err(Error::Changed);
    }
    session.step(Event::Verified)?;
    Ok(Receipt {
        context: context.clone(),
        target: target.clone(),
        old: old.clone(),
        new: new.clone(),
        slots: old_slots,
        before_hash: raw_digest(&before_raw),
        after_hash: raw_digest(&after_raw),
        after_raw,
        _retained: retained,
    })
}
