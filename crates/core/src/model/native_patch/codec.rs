//! Bounded durable receipt representation. Executor must separately authorize
//! its batch/plan/source binding; this checksum is integrity, not authority.
use super::{Anchor, Error, Hash, Node, Receipt, Result, Slot};
use crate::{
    model::{
        exact::{
            ExactReadPolicy, ExactTextState, Id3TextEncoding, Mp4TextKey, Mp4TextKind,
            NativeTextContext, NativeTextTarget,
        },
        AudioFormat, Id3v2Revision, TagBlockKind,
    },
    resources::{Budget, Charge, Lease},
};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"TRX-NTP1";
const MAX: usize = 16 * 1024 * 1024;
const ITEMS: usize = 4096;
const HEADER: usize = 44;

pub struct EncodedReceipt {
    bytes: Vec<u8>,
    _owner: Lease,
}
impl EncodedReceipt {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
struct Writer<'a> {
    bytes: Option<&'a mut Vec<u8>>,
    len: usize,
}
impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.len = self
            .len
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX)
            .ok_or(Error::Limit)?;
        if let Some(out) = &mut self.bytes {
            out.extend_from_slice(bytes);
        }
        Ok(())
    }
    fn byte(&mut self, byte: u8) -> Result<()> {
        self.put(&[byte])
    }
    fn number(&mut self, value: usize) -> Result<()> {
        self.put(
            &u32::try_from(value)
                .map_err(|_| Error::Limit)?
                .to_le_bytes(),
        )
    }
    fn text(&mut self, text: &str) -> Result<()> {
        self.blob(text.as_bytes())
    }
    fn blob(&mut self, bytes: &[u8]) -> Result<()> {
        self.number(bytes.len())?;
        self.put(bytes)
    }
    fn optional_text(&mut self, text: Option<&str>) -> Result<()> {
        self.byte(u8::from(text.is_some()))?;
        if let Some(text) = text {
            self.text(text)?;
        }
        Ok(())
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(len).ok_or(Error::Limit)?;
        let out = self
            .bytes
            .get(self.at..end)
            .ok_or(Error::Invalid("truncated receipt"))?;
        self.at = end;
        Ok(out)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn flag(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Invalid("receipt flag")),
        }
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::Invalid("receipt fixed value"))
    }
    fn number(&mut self, max: usize) -> Result<usize> {
        let n = u32::from_le_bytes(self.array()?) as usize;
        if n > max {
            return Err(Error::Limit);
        }
        Ok(n)
    }
    fn count(&mut self, max: usize, minimum_record: usize) -> Result<usize> {
        let count = self.number(max)?;
        if count > (self.bytes.len() - self.at) / minimum_record {
            return Err(Error::Invalid("truncated receipt collection"));
        }
        Ok(count)
    }
    fn blob(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = self.number(max)?;
        self.take(len)
    }
    fn text(&mut self, max: usize) -> Result<String> {
        std::str::from_utf8(self.blob(max)?)
            .map(str::to_owned)
            .map_err(|_| Error::Invalid("receipt UTF-8"))
    }
    fn optional_text(&mut self, max: usize) -> Result<Option<String>> {
        if self.flag()? {
            self.text(max).map(Some)
        } else {
            Ok(None)
        }
    }
}
fn block_code(block: TagBlockKind) -> u8 {
    match block {
        TagBlockKind::Id3v1 => 1,
        TagBlockKind::Id3v2 => 2,
        TagBlockKind::VorbisComments => 3,
        TagBlockKind::Ape => 4,
        TagBlockKind::Mp4Ilst => 5,
        TagBlockKind::RiffInfo => 6,
        TagBlockKind::AiffText => 7,
    }
}
fn block(code: u8) -> Result<TagBlockKind> {
    match code {
        1 => Ok(TagBlockKind::Id3v1),
        2 => Ok(TagBlockKind::Id3v2),
        3 => Ok(TagBlockKind::VorbisComments),
        4 => Ok(TagBlockKind::Ape),
        5 => Ok(TagBlockKind::Mp4Ilst),
        6 => Ok(TagBlockKind::RiffInfo),
        7 => Ok(TagBlockKind::AiffText),
        _ => Err(Error::Invalid("receipt block")),
    }
}
fn write_context(w: &mut Writer<'_>, c: &NativeTextContext) -> Result<()> {
    w.byte(match c.container {
        AudioFormat::Flac => 1,
        AudioFormat::Mp3 => 2,
        AudioFormat::Aac => 3,
        AudioFormat::M4a => 4,
        _ => return Err(Error::Unsupported("receipt container")),
    })?;
    w.byte(c.selected_block.map(block_code).unwrap_or(0))?;
    w.byte(match c.id3_revision {
        None => 0,
        Some(Id3v2Revision::V3) => 3,
        Some(Id3v2Revision::V4) => 4,
        Some(Id3v2Revision::V2) => return Err(Error::Unsupported("receipt ID3 revision")),
    })?;
    w.put(&c.policy_revision.to_le_bytes())?;
    w.byte(c.adapter_version)?;
    w.byte(u8::from(c.raw_digest.is_some()))?;
    if let Some(digest) = c.raw_digest {
        w.put(&digest)?;
    }
    let p = &c.read_policy;
    w.put(&p.revision.to_le_bytes())?;
    w.text(&p.multi_value_separator)?;
    w.number(p.read_priority.len())?;
    for priority in &p.read_priority {
        w.byte(block_code(*priority))?;
    }
    w.optional_text(p.legacy_code_page.as_deref())?;
    w.byte(u8::from(p.write_id3v23))
}
fn read_context(r: &mut Reader<'_>) -> Result<NativeTextContext> {
    let container = match r.byte()? {
        1 => AudioFormat::Flac,
        2 => AudioFormat::Mp3,
        3 => AudioFormat::Aac,
        4 => AudioFormat::M4a,
        _ => return Err(Error::Unsupported("receipt container")),
    };
    let selected_block = Some(block(r.byte()?)?);
    let id3_revision = match r.byte()? {
        0 => None,
        3 => Some(Id3v2Revision::V3),
        4 => Some(Id3v2Revision::V4),
        _ => return Err(Error::Unsupported("receipt ID3 revision")),
    };
    let policy_revision = u64::from_le_bytes(r.array()?);
    let adapter_version = r.byte()?;
    let raw_digest = if r.flag()? { Some(r.array()?) } else { None };
    let revision = u64::from_le_bytes(r.array()?);
    let multi_value_separator = r.text(1024)?;
    let count = r.count(256, 1)?;
    let mut read_priority = Vec::with_capacity(count);
    for _ in 0..count {
        read_priority.push(block(r.byte()?)?);
    }
    let legacy_code_page = r.optional_text(64)?;
    let write_id3v23 = r.flag()?;
    if adapter_version != 1
        || revision != policy_revision
        || revision == u64::MAX
        || raw_digest.is_none()
    {
        return Err(Error::Invalid("receipt context"));
    }
    let profile = match container {
        AudioFormat::Flac => {
            selected_block == Some(TagBlockKind::VorbisComments) && id3_revision.is_none()
        }
        AudioFormat::Mp3 | AudioFormat::Aac => {
            selected_block == Some(TagBlockKind::Id3v2) && id3_revision.is_some()
        }
        AudioFormat::M4a => selected_block == Some(TagBlockKind::Mp4Ilst) && id3_revision.is_none(),
        _ => false,
    };
    if !profile {
        return Err(Error::Unsupported("receipt native profile"));
    }
    Ok(NativeTextContext {
        container,
        selected_block,
        id3_revision,
        policy_revision,
        adapter_version,
        raw_digest,
        read_policy: ExactReadPolicy {
            revision,
            multi_value_separator,
            read_priority,
            legacy_code_page,
            write_id3v23,
        },
    })
}
fn write_target(w: &mut Writer<'_>, t: &NativeTextTarget) -> Result<()> {
    match t {
        NativeTextTarget::Vorbis { name } => {
            w.byte(1)?;
            w.text(name)
        }
        NativeTextTarget::Id3 {
            frame,
            description,
            language,
            encoding,
        } => {
            w.byte(2)?;
            w.text(frame)?;
            w.optional_text(description.as_deref())?;
            w.byte(u8::from(language.is_some()))?;
            if let Some(language) = language {
                w.put(language)?;
            }
            w.byte(match encoding {
                None => 0,
                Some(Id3TextEncoding::Latin1) => 1,
                Some(Id3TextEncoding::Utf8) => 2,
            })
        }
        NativeTextTarget::Mp4 { key, kind, locale } => {
            w.byte(3)?;
            match key {
                Mp4TextKey::Fourcc(key) => {
                    w.byte(0)?;
                    w.put(key)?
                }
                Mp4TextKey::Freeform { mean, name } => {
                    w.byte(1)?;
                    w.text(mean)?;
                    w.text(name)?
                }
            }
            w.put(
                &match kind {
                    Mp4TextKind::Utf8 => 1u32,
                    Mp4TextKind::Utf16 => 2,
                    Mp4TextKind::Other(_) => return Err(Error::Unsupported("receipt MP4 kind")),
                }
                .to_le_bytes(),
            )?;
            w.put(&locale.to_le_bytes())
        }
    }
}
fn read_target(r: &mut Reader<'_>) -> Result<NativeTextTarget> {
    let target = match r.byte()? {
        1 => NativeTextTarget::Vorbis {
            name: r.text(1024)?,
        },
        2 => {
            let frame = r.text(4)?;
            let description = r.optional_text(1024)?;
            let language = if r.flag()? { Some(r.array()?) } else { None };
            let encoding = match r.byte()? {
                0 => None,
                1 => Some(Id3TextEncoding::Latin1),
                2 => Some(Id3TextEncoding::Utf8),
                _ => return Err(Error::Unsupported("receipt ID3 encoding")),
            };
            NativeTextTarget::Id3 {
                frame,
                description,
                language,
                encoding,
            }
        }
        3 => {
            let key = match r.byte()? {
                0 => Mp4TextKey::Fourcc(r.array()?),
                1 => Mp4TextKey::Freeform {
                    mean: r.text(1024)?,
                    name: r.text(1024)?,
                },
                _ => return Err(Error::Invalid("receipt MP4 key")),
            };
            let kind = match u32::from_le_bytes(r.array()?) {
                1 => Mp4TextKind::Utf8,
                2 => Mp4TextKind::Utf16,
                _ => return Err(Error::Unsupported("receipt MP4 kind")),
            };
            NativeTextTarget::Mp4 {
                key,
                kind,
                locale: u32::from_le_bytes(r.array()?),
            }
        }
        _ => return Err(Error::Unsupported("receipt target")),
    };
    target
        .validate()
        .map_err(|_| Error::Invalid("receipt native identity"))?;
    Ok(target)
}
fn write_state(w: &mut Writer<'_>, state: &ExactTextState) -> Result<()> {
    w.byte(u8::from(state.values().is_some()))?;
    if let Some(values) = state.values() {
        w.number(values.as_slice().len())?;
        for value in values.as_slice() {
            w.text(value)?;
        }
    }
    Ok(())
}
fn read_state(r: &mut Reader<'_>) -> Result<ExactTextState> {
    if !r.flag()? {
        return Ok(ExactTextState::absent());
    }
    let count = r.count(1024, 4)?;
    let mut values = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        let value = r.text(1024 * 1024)?;
        total = total
            .checked_add(value.len())
            .filter(|n| *n <= 1024 * 1024)
            .ok_or(Error::Limit)?;
        values.push(value);
    }
    ExactTextState::try_from(values).map_err(|_| Error::Invalid("receipt text state"))
}
fn write_anchor(w: &mut Writer<'_>, anchor: Option<&Anchor>) -> Result<()> {
    w.byte(u8::from(anchor.is_some()))?;
    if let Some(a) = anchor {
        w.put(&a.key)?;
        w.number(a.ordinal)?;
        w.number(a.total)?;
        w.byte(u8::from(a.grouped))?;
    }
    Ok(())
}
fn read_anchor(r: &mut Reader<'_>) -> Result<Option<Anchor>> {
    if !r.flag()? {
        return Ok(None);
    }
    let key = r.array()?;
    let ordinal = r.number(ITEMS)?;
    let total = r.number(ITEMS)?;
    let grouped = r.flag()?;
    if total == 0 || ordinal >= total {
        return Err(Error::Invalid("receipt anchor"));
    }
    Ok(Some(Anchor {
        key,
        ordinal,
        total,
        grouped,
    }))
}
fn write_body(w: &mut Writer<'_>, receipt: &Receipt) -> Result<()> {
    write_context(w, &receipt.context)?;
    write_target(w, &receipt.target)?;
    write_state(w, &receipt.old)?;
    write_state(w, &receipt.new)?;
    w.put(&receipt.before_hash)?;
    w.put(&receipt.after_hash)?;
    w.number(receipt.slots.len())?;
    for slot in &receipt.slots {
        write_anchor(w, slot.previous.as_ref())?;
        write_anchor(w, slot.next.as_ref())?;
        w.number(slot.nodes.len())?;
        for node in &slot.nodes {
            w.blob(&node.raw)?;
        }
    }
    w.number(receipt.after_raw.len())?;
    for raw in &receipt.after_raw {
        w.blob(raw)?;
    }
    Ok(())
}
fn values_match<'a>(nodes: impl Iterator<Item = &'a Node>, state: &ExactTextState) -> bool {
    let nodes: Vec<_> = nodes.collect();
    match state.values() {
        None => nodes.is_empty(),
        Some(values) => {
            !nodes.is_empty()
                && nodes
                    .iter()
                    .flat_map(|node| node.values.iter())
                    .eq(values.as_slice().iter())
        }
    }
}
fn read_node(
    r: &mut Reader<'_>,
    context: &NativeTextContext,
    target: &NativeTextTarget,
) -> Result<Node> {
    let node = super::parse_target_record(context, target, r.blob(MAX)?)?;
    if !node.selected || node.values.is_empty() {
        return Err(Error::Invalid("receipt record does not own target"));
    }
    Ok(node)
}
impl Receipt {
    pub fn encode_for_journal(&self) -> Result<EncodedReceipt> {
        self.encode_for_journal_with_budget(Budget::process())
    }
    pub fn encode_for_journal_with_budget(&self, budget: Budget) -> Result<EncodedReceipt> {
        let mut count = Writer {
            bytes: None,
            len: 0,
        };
        write_body(&mut count, self)?;
        let owner = budget.try_acquire(Charge {
            retained: count.len + HEADER + 128,
            ..Default::default()
        })?;
        let mut bytes = Vec::with_capacity(count.len + HEADER);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(count.len as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; 32]);
        write_body(
            &mut Writer {
                bytes: Some(&mut bytes),
                len: 0,
            },
            self,
        )?;
        let digest: Hash = Sha256::digest(&bytes[HEADER..]).into();
        bytes[12..HEADER].copy_from_slice(&digest);
        Ok(EncodedReceipt {
            bytes,
            _owner: owner,
        })
    }
    /// Rebind a validated historical receipt only when semantic interpretation
    /// is unchanged. Restore still checks the actual native file and raw target.
    pub fn rebind_current_policy(mut self) -> Result<Self> {
        let current = ExactReadPolicy::capture();
        let mut previous = self.context.read_policy.clone();
        previous.revision = current.revision;
        if previous != current || super::super::read_policy_revision() != current.revision {
            return Err(Error::Changed);
        }
        self.context.policy_revision = current.revision;
        self.context.read_policy = current;
        Ok(self)
    }
    /// Decode historical evidence only. Executor must validate its complete
    /// plan/source binding before restoration. Stale policy remains readable.
    pub fn decode_for_journal(bytes: &[u8]) -> Result<Self> {
        Self::decode_for_journal_with_budget(bytes, Budget::process())
    }
    pub fn decode_for_journal_with_budget(bytes: &[u8], budget: Budget) -> Result<Self> {
        if bytes.len() < HEADER || bytes.len() > MAX + HEADER || bytes.get(..8) != Some(MAGIC) {
            return Err(Error::Invalid("receipt envelope"));
        }
        let size = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| Error::Invalid("receipt size"))?,
        ) as usize;
        let digest: Hash = Sha256::digest(&bytes[HEADER..]).into();
        if size != bytes.len() - HEADER || digest[..] != bytes[12..HEADER] {
            return Err(Error::Invalid("receipt integrity"));
        }
        let working = size
            .checked_mul(16)
            .and_then(|n| n.checked_add(64 * 1024))
            .ok_or(Error::Limit)?;
        let owner = budget.try_acquire(Charge {
            transient: working,
            jobs: 1,
            ..Default::default()
        })?;
        let mut r = Reader {
            bytes: &bytes[HEADER..],
            at: 0,
        };
        let context = read_context(&mut r)?;
        let target = read_target(&mut r)?;
        let old = read_state(&mut r)?;
        let new = read_state(&mut r)?;
        // Reverse receipts can restore a formerly removed native fourcc. At
        // least one direction must be an admitted text intent; typed/read-only
        // targets are rejected in both directions by the shared validator.
        if super::super::exact::validate_text_intent(&context, &target, &old, &new).is_err()
            && super::super::exact::validate_text_intent(&context, &target, &new, &old).is_err()
        {
            return Err(Error::Unsupported("receipt text capability"));
        }
        let before_hash = r.array()?;
        let after_hash = r.array()?;
        // One slot needs two anchor flags, a count, and at least one
        // valid native record. Even the smallest Vorbis record occupies six
        // raw bytes plus its four-byte envelope length. Reserve vector capacity
        // only for counts backed by that minimum valid representation.
        let count = r.count(ITEMS, 16)?;
        // Admitted ID3/MP4 layouts represent this target in one native record.
        // FLAC alone can preserve multiple interleaved target runs. Reject an
        // impossible historical layout before it can be restored then refused
        // by the writer's post-mutation parser.
        let single_record = context.container != AudioFormat::Flac;
        if single_record && count > 1 {
            return Err(Error::Invalid("receipt duplicate native target"));
        }
        let mut slots = Vec::with_capacity(count);
        let mut total_nodes = 0usize;
        let mut total_values = 0usize;
        let expected_old_values = old.values().map_or(0, |values| values.as_slice().len());
        for _ in 0..count {
            let previous = read_anchor(&mut r)?;
            let next = read_anchor(&mut r)?;
            let count = r.count(ITEMS, 10)?;
            if single_record && count > 1 {
                return Err(Error::Invalid("receipt duplicate native target"));
            }
            if count == 0 {
                return Err(Error::Invalid("empty receipt slot"));
            }
            total_nodes = total_nodes
                .checked_add(count)
                .filter(|n| *n <= ITEMS)
                .ok_or(Error::Limit)?;
            let mut nodes = Vec::with_capacity(count);
            for _ in 0..count {
                let node = read_node(&mut r, &context, &target)?;
                total_values = total_values
                    .checked_add(node.values.len())
                    .filter(|count| *count <= expected_old_values)
                    .ok_or(Error::Invalid("receipt aggregate old values"))?;
                nodes.push(node);
            }
            slots.push(Slot {
                previous,
                next,
                nodes,
            });
        }
        let count = r.count(ITEMS, 10)?;
        if single_record && count > 1 {
            return Err(Error::Invalid("receipt duplicate native target"));
        }
        let mut after_nodes = Vec::with_capacity(count);
        let mut total_values = 0usize;
        let expected_new_values = new.values().map_or(0, |values| values.as_slice().len());
        for _ in 0..count {
            let node = read_node(&mut r, &context, &target)?;
            total_values = total_values
                .checked_add(node.values.len())
                .filter(|count| *count <= expected_new_values)
                .ok_or(Error::Invalid("receipt aggregate new values"))?;
            after_nodes.push(node);
        }
        if r.at != r.bytes.len()
            || !values_match(slots.iter().flat_map(|s| &s.nodes), &old)
            || !values_match(after_nodes.iter(), &new)
        {
            return Err(Error::Invalid("receipt semantic records"));
        }
        let before_raw: Vec<_> = slots
            .iter()
            .flat_map(|s| &s.nodes)
            .map(|n| n.raw.clone())
            .collect();
        let after_raw: Vec<_> = after_nodes.into_iter().map(|n| n.raw).collect();
        if super::raw_digest(&before_raw) != before_hash
            || super::raw_digest(&after_raw) != after_hash
        {
            return Err(Error::Invalid("receipt target digest"));
        }
        drop(before_raw);
        let mut receipt = Receipt {
            context,
            target,
            old,
            new,
            slots,
            after_raw,
            before_hash,
            after_hash,
            _retained: owner,
        };
        let retained = receipt
            .slots
            .iter()
            .flat_map(|s| &s.nodes)
            .map(|n| {
                n.raw.capacity() + 256 + n.values.iter().map(|s| s.capacity() + 32).sum::<usize>()
            })
            .sum::<usize>()
            + receipt.after_raw.iter().map(Vec::capacity).sum::<usize>()
            + receipt.after_raw.capacity() * std::mem::size_of::<Vec<u8>>()
            + receipt
                .slots
                .iter()
                .map(|slot| slot.nodes.capacity() * std::mem::size_of::<Node>())
                .sum::<usize>()
            + [&receipt.old, &receipt.new]
                .iter()
                .filter_map(|s| s.values())
                .flat_map(|v| v.as_slice())
                .map(|s| s.capacity() + 32)
                .sum::<usize>()
            + receipt.slots.capacity() * 256
            + 16 * 1024;
        if retained > MAX {
            return Err(Error::Limit);
        }
        receipt._retained.try_resize(Charge {
            retained,
            ..Default::default()
        })?;
        Ok(receipt)
    }
}

/// Trusted preparation data persisted by the Executor before any publication.
/// Integrity checks do not authorize its path, source identity or owning plan.
#[cfg(target_os = "linux")]
pub(crate) struct StoredIntent {
    pub context: NativeTextContext,
    pub target: NativeTextTarget,
    pub old: ExactTextState,
    pub new: ExactTextState,
    _retained: Lease,
}
#[cfg(target_os = "linux")]
pub(crate) fn encode_intent(
    patch: &crate::model::exact::PreparedTextPatch,
) -> Result<EncodedReceipt> {
    let write = |w: &mut Writer<'_>| -> Result<()> {
        write_context(w, patch.context())?;
        write_target(w, patch.target())?;
        write_state(w, patch.old())?;
        write_state(w, patch.new_state())
    };
    let mut measure = Writer {
        bytes: None,
        len: 0,
    };
    write(&mut measure)?;
    let total = HEADER.checked_add(measure.len).ok_or(Error::Limit)?;
    let owner = Budget::process().try_acquire(Charge {
        retained: total,
        ..Default::default()
    })?;
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(b"TRX-NIP1");
    bytes.extend_from_slice(&(measure.len as u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 32]);
    write(&mut Writer {
        bytes: Some(&mut bytes),
        len: 0,
    })?;
    let digest = Sha256::digest(&bytes[HEADER..]);
    bytes[12..HEADER].copy_from_slice(&digest);
    Ok(EncodedReceipt {
        bytes,
        _owner: owner,
    })
}
#[cfg(target_os = "linux")]
pub(crate) fn decode_intent(bytes: &[u8]) -> Result<StoredIntent> {
    if bytes.len() < HEADER || &bytes[..8] != b"TRX-NIP1" || bytes.len() > MAX {
        return Err(Error::Invalid("native intent envelope"));
    }
    let len = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| Error::Limit)?) as usize;
    if len.checked_add(HEADER) != Some(bytes.len())
        || Sha256::digest(&bytes[HEADER..]).as_slice() != &bytes[12..HEADER]
    {
        return Err(Error::Invalid("native intent checksum/length"));
    }
    let _work = Budget::process().try_acquire(Charge {
        transient: bytes
            .len()
            .checked_mul(16)
            .and_then(|n| n.checked_add(65536))
            .ok_or(Error::Limit)?,
        jobs: 1,
        ..Default::default()
    })?;
    let retained = bytes
        .len()
        .checked_mul(4)
        .and_then(|n| n.checked_add(65536))
        .filter(|n| *n <= MAX)
        .ok_or(Error::Limit)?;
    let owner = Budget::process().try_acquire(Charge {
        retained,
        ..Default::default()
    })?;
    let mut r = Reader {
        bytes: &bytes[HEADER..],
        at: 0,
    };
    let context = read_context(&mut r)?;
    let target = read_target(&mut r)?;
    let old = read_state(&mut r)?;
    let new = read_state(&mut r)?;
    if r.at != r.bytes.len() {
        return Err(Error::Invalid("native intent suffix"));
    }
    crate::model::exact::validate_text_intent(&context, &target, &old, &new)
        .map_err(|_| Error::Invalid("native intent capability"))?;
    Ok(StoredIntent {
        context,
        target,
        old,
        new,
        _retained: owner,
    })
}
