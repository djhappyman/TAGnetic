//! Native text states and bounded read adapters for the exact-metadata migration.
//!
//! This module never writes media. The narrow Linux in-place FLAC text route
//! uses prepared intents through private Executor/journal preservation checks.
//! General exact staging and other mutation profiles remain separate.

use super::{AudioFormat, Id3v2Revision, TagBlockKind, TagField, TrackRead};
use crate::resources::{Budget, Charge, Lease};
use crate::values::FieldValues;
use std::collections::BTreeMap;

mod raw;
pub(super) use raw::Input;
mod source;
const MAX_ENTRIES: usize = 4096;
const MAX_NAME: usize = 1024;
const MAX_FIELD_BYTES: usize = 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 24 * 1024 * 1024;

/// Absence differs from a present empty string and from a repeated empty value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactTextState(Option<FieldValues>);
impl ExactTextState {
    pub fn absent() -> Self {
        Self(None)
    }
    pub fn present(values: FieldValues) -> Result<Self, ExactReadError> {
        if values.as_slice().is_empty() {
            return Err(ExactReadError::Invalid(
                "Present text needs at least one value",
            ));
        }
        Ok(Self(Some(values)))
    }
    pub fn values(&self) -> Option<&FieldValues> {
        self.0.as_ref()
    }
    fn owned_bytes(&self) -> usize {
        self.values().map_or(0, |values| {
            values
                .as_slice()
                .iter()
                .map(|value| value.capacity().saturating_add(32))
                .sum()
        })
    }
}
impl TryFrom<Vec<String>> for ExactTextState {
    type Error = ExactReadError;
    fn try_from(values: Vec<String>) -> Result<Self, Self::Error> {
        Self::present(
            FieldValues::new(values)
                .map_err(|_| ExactReadError::Invalid("Invalid ordered text values"))?,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Id3TextEncoding {
    Latin1,
    Utf8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mp4TextKind {
    Utf8,
    Utf16,
    Other(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mp4TextKey {
    Fourcc([u8; 4]),
    Freeform { mean: String, name: String },
}

/// Native identity is separate from a legacy display field name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum NativeTextTarget {
    Vorbis {
        name: String,
    },
    Id3 {
        frame: String,
        description: Option<String>,
        language: Option<[u8; 3]>,
        encoding: Option<Id3TextEncoding>,
    },
    Mp4 {
        key: Mp4TextKey,
        kind: Mp4TextKind,
        locale: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    UnsupportedLayout,
    UnsupportedEncoding,
    NonText,
    InvalidText,
    AmbiguousIdentity,
    ReadOnly,
    TypedField,
    NoSelectedBlock,
}

#[derive(Debug, thiserror::Error)]
pub enum ExactReadError {
    #[error("Invalid exact metadata: {0}")]
    Invalid(&'static str),
    #[error("Exact metadata exceeds its bounded read limits")]
    Limit,
    #[error("Exact metadata is unavailable: {0:?}")]
    Unavailable(UnavailableReason),
    #[error("Metadata policy or file identity changed during exact reading")]
    Changed,
    #[error("Exact metadata reading was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Legacy(#[from] super::TagIoError),
    #[error(transparent)]
    Resource(#[from] crate::resources::AdmissionError),
}

/// Actual selected native block and the interpretation generation used to read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactReadPolicy {
    pub revision: u64,
    pub multi_value_separator: String,
    pub read_priority: Vec<TagBlockKind>,
    pub legacy_code_page: Option<String>,
    pub write_id3v23: bool,
}
impl ExactReadPolicy {
    pub(super) fn capture() -> Self {
        let (snapshot, revision) = super::policy::capture();
        Self {
            revision,
            multi_value_separator: snapshot.separator.clone(),
            read_priority: snapshot
                .read_priority
                .iter()
                .filter_map(|kind| TagBlockKind::from_tag_type(*kind))
                .collect(),
            legacy_code_page: snapshot.code_page.map(|encoding| encoding.name().into()),
            write_id3v23: snapshot.write_id3v23,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTextContext {
    pub container: AudioFormat,
    pub selected_block: Option<TagBlockKind>,
    pub id3_revision: Option<Id3v2Revision>,
    pub policy_revision: u64,
    pub adapter_version: u8,
    /// Digest of the selected raw metadata block; not write authorization.
    pub raw_digest: Option<[u8; 32]>,
    pub read_policy: ExactReadPolicy,
}

#[derive(Debug)]
pub struct ExactTrackRead {
    legacy: TrackRead,
    text: ExactTextSnapshot,
}
impl ExactTrackRead {
    pub fn legacy(&self) -> &TrackRead {
        &self.legacy
    }
    pub fn text(&self) -> &ExactTextSnapshot {
        &self.text
    }
    pub(super) fn new(
        legacy: TrackRead,
        mut text: ExactTextSnapshot,
    ) -> Result<Self, ExactReadError> {
        let legacy_bytes = legacy.file.tags.iter().fold(
            512usize.saturating_add(legacy.file.path.capacity()),
            |total, (key, value)| {
                total
                    .saturating_add(128)
                    .saturating_add(value.capacity())
                    .saturating_add(match key {
                        TagField::Custom(name) => name.capacity(),
                        _ => 0,
                    })
            },
        );
        let total = text
            .bytes_owned
            .saturating_add(legacy_bytes)
            .saturating_add(
                legacy
                    .blocks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<super::TagBlock>()),
            );
        if total > MAX_RESULT_BYTES {
            return Err(ExactReadError::Limit);
        }
        text._retained.try_resize(Charge {
            retained: total,
            ..Default::default()
        })?;
        text.bytes_owned = total;
        Ok(Self { legacy, text })
    }
}

#[derive(Debug)]
pub struct ExactTextSnapshot {
    context: NativeTextContext,
    entries: Vec<ExactTextEntry>,
    unavailable: Option<UnavailableReason>,
    _retained: Lease,
    bytes_owned: usize,
}
impl ExactTextSnapshot {
    pub fn entries(&self) -> &[ExactTextEntry] {
        &self.entries
    }
    pub fn context(&self) -> &NativeTextContext {
        &self.context
    }
    pub fn unavailable(&self) -> Option<UnavailableReason> {
        self.unavailable
    }

    /// Resolve a display field only if one native identity owns it. Different
    /// namespaces, qualifiers or differently cased native keys never collapse.
    pub fn resolve_field(
        &self,
        field: &TagField,
    ) -> Result<Option<&ExactTextEntry>, UnavailableReason> {
        if let Some(reason) = self.unavailable {
            return Err(reason);
        }
        let mut entries = self
            .entries
            .iter()
            .filter(|entry| entry.field.as_ref().is_some_and(|own| own.same_name(field)));
        let first = entries.next();
        if entries.next().is_some() {
            return Err(UnavailableReason::AmbiguousIdentity);
        }
        Ok(first)
    }

    pub fn state(&self, target: &NativeTextTarget) -> Result<ExactTextState, ExactReadError> {
        self.state_ref(target).cloned()
    }
    fn state_ref(&self, target: &NativeTextTarget) -> Result<&ExactTextState, ExactReadError> {
        target.validate()?;
        if let Some(reason) = self.unavailable {
            return Err(ExactReadError::Unavailable(reason));
        }
        if !target.matches_block(self.context.selected_block) {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
        let mut exact = self.entries.iter().filter(|entry| match target {
            NativeTextTarget::Vorbis { .. } | NativeTextTarget::Id3 { .. } => {
                entry.target.same_slot(target)
            }
            NativeTextTarget::Mp4 { .. } => &entry.target == target,
        });
        if let Some(first) = exact.next() {
            if exact.next().is_some() {
                return Err(ExactReadError::Unavailable(
                    UnavailableReason::AmbiguousIdentity,
                ));
            }
            if matches!(target, NativeTextTarget::Id3 { .. }) && &first.target != target {
                return Err(ExactReadError::Unavailable(
                    UnavailableReason::UnsupportedEncoding,
                ));
            }
            return first
                .state
                .as_ref()
                .map_err(|reason| ExactReadError::Unavailable(*reason));
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.target.same_slot(target))
        {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::AmbiguousIdentity,
            ));
        }
        static ABSENT: ExactTextState = ExactTextState(None);
        Ok(&ABSENT)
    }

    /// Validate an explicit semantic mutation without writing anything. The
    /// returned intent cannot be applied by today's scalar writer; Executor,
    /// stage and journal integration is a separate required migration gate.
    pub fn prepare_patch(
        &self,
        target: &NativeTextTarget,
        new: ExactTextState,
    ) -> Result<PreparedTextPatch, ExactReadError> {
        if self.context.read_policy != ExactReadPolicy::capture() {
            return Err(ExactReadError::Changed);
        }
        let old = self.state_ref(target)?;
        validate_text_intent(&self.context, target, old, &new)?;
        let native_target = self
            .entries
            .iter()
            .find(|entry| match target {
                NativeTextTarget::Vorbis { .. } => entry.target.same_slot(target),
                _ => &entry.target == target,
            })
            .map(|entry| entry.target.clone())
            .unwrap_or_else(|| target.clone());
        let retained = Budget::process().try_acquire(Charge {
            retained: old
                .owned_bytes()
                .saturating_add(new.owned_bytes())
                .saturating_add(1024)
                .saturating_add(target.bytes()),
            ..Default::default()
        })?;
        Ok(PreparedTextPatch {
            context: self.context.clone(),
            target: native_target,
            old: old.clone(),
            new,
            _retained: retained,
        })
    }
}

/// Shared semantic capability gate; identity validation alone does not admit
/// typed or unsupported native mutations. Container/raw evidence is separate.
pub(super) fn validate_text_intent(
    context: &NativeTextContext,
    target: &NativeTextTarget,
    old: &ExactTextState,
    new: &ExactTextState,
) -> Result<(), ExactReadError> {
    target.validate()?;
    if let NativeTextTarget::Id3 {
        frame, encoding, ..
    } = target
    {
        if encoding.is_none()
            || (context.id3_revision == Some(Id3v2Revision::V3)
                && *encoding != Some(Id3TextEncoding::Latin1))
        {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedEncoding,
            ));
        }
        if matches!(
            frame.as_str(),
            "TCMP"
                | "TRCK"
                | "TPOS"
                | "TBPM"
                | "TYER"
                | "TDAT"
                | "TIME"
                | "TDRC"
                | "TDOR"
                | "TDRL"
                | "TDEN"
                | "TDTG"
        ) {
            return Err(ExactReadError::Unavailable(UnavailableReason::TypedField));
        }
        if matches!(frame.as_str(), "COMM" | "USLT")
            && new
                .values()
                .is_some_and(|values| values.as_slice().len() > 1)
        {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
    }
    if let NativeTextTarget::Mp4 {
        key: Mp4TextKey::Fourcc(key),
        ..
    } = target
    {
        if matches!(
            key,
            b"covr"
                | b"gnre"
                | b"tmpo"
                | b"trkn"
                | b"disk"
                | b"cpil"
                | b"pgap"
                | b"stik"
                | b"rtng"
                | b"hdvd"
                | b"pcst"
                | b"tves"
                | b"tvsn"
                | b"cnID"
                | b"akID"
                | b"sfID"
                | b"atID"
                | b"plID"
                | b"geID"
        ) {
            return Err(ExactReadError::Unavailable(UnavailableReason::TypedField));
        }
        if old.values().is_none() {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
    }
    let field = logical_field(target);
    if field.as_ref().is_some_and(is_typed_field) {
        return Err(ExactReadError::Unavailable(UnavailableReason::TypedField));
    }
    if matches!(
        target,
        NativeTextTarget::Mp4 {
            kind: Mp4TextKind::Other(_),
            ..
        }
    ) {
        return Err(ExactReadError::Unavailable(UnavailableReason::NonText));
    }
    if let Some(values) = new.values() {
        if matches!(
            target,
            NativeTextTarget::Id3 {
                encoding: Some(Id3TextEncoding::Latin1),
                ..
            }
        ) && values
            .as_slice()
            .iter()
            .any(|value| value.chars().any(|ch| u32::from(ch) > 255))
        {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedEncoding,
            ));
        }
        if context.id3_revision == Some(Id3v2Revision::V3) && values.as_slice().len() > 1 {
            return Err(ExactReadError::Unavailable(
                UnavailableReason::UnsupportedLayout,
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct PreparedTextPatch {
    context: NativeTextContext,
    target: NativeTextTarget,
    old: ExactTextState,
    new: ExactTextState,
    _retained: Lease,
}
impl PreparedTextPatch {
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
    /// Explicitly false until the existing mutation/journal path supports exact intents.
    pub fn executable(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactTextEntry {
    pub target: NativeTextTarget,
    pub field: Option<TagField>,
    pub state: Result<ExactTextState, UnavailableReason>,
}

impl NativeTextTarget {
    #[cfg(target_os = "linux")]
    pub(crate) fn logical_field(&self) -> Option<TagField> {
        logical_field(self)
    }

    pub fn validate(&self) -> Result<(), ExactReadError> {
        let name = |text: &str, empty: bool| {
            if text.len() > MAX_NAME || text.contains('\0') || (!empty && text.is_empty()) {
                Err(ExactReadError::Invalid("Invalid native text identity"))
            } else {
                Ok(())
            }
        };
        match self {
            Self::Vorbis { name: key } => {
                name(key, false)?;
                if !key
                    .bytes()
                    .all(|byte| (0x20..=0x7d).contains(&byte) && byte != b'=')
                {
                    return Err(ExactReadError::Invalid("Invalid Vorbis field name"));
                }
            }
            Self::Id3 {
                frame,
                description,
                language,
                encoding,
            } => {
                if frame.len() != 4
                    || !frame
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                {
                    return Err(ExactReadError::Invalid("Invalid ID3 frame identity"));
                }
                if let Some(description) = description {
                    name(description, true)?;
                }
                if language.is_some_and(|language| !language.iter().all(u8::is_ascii_alphabetic)) {
                    return Err(ExactReadError::Invalid(
                        "ID3 language must be a three-letter code",
                    ));
                }
                let qualified = matches!(frame.as_str(), "COMM" | "USLT");
                if qualified != language.is_some()
                    || (qualified && description.is_none())
                    || (frame == "TXXX" && description.is_none())
                    || (!qualified && frame != "TXXX" && description.is_some())
                    || (encoding.is_some() && !frame.starts_with('T') && !qualified)
                {
                    return Err(ExactReadError::Invalid("Inconsistent ID3 qualifiers"));
                }
            }
            Self::Mp4 {
                key: Mp4TextKey::Freeform { mean, name: key },
                ..
            } => {
                name(mean, false)?;
                name(key, false)?;
            }
            Self::Mp4 { .. } => {}
        }
        Ok(())
    }
    fn matches_block(&self, block: Option<TagBlockKind>) -> bool {
        matches!(
            (self, block),
            (Self::Vorbis { .. }, Some(TagBlockKind::VorbisComments))
                | (Self::Id3 { .. }, Some(TagBlockKind::Id3v2))
                | (Self::Mp4 { .. }, Some(TagBlockKind::Mp4Ilst))
        )
    }
    fn same_slot(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Vorbis { name: a }, Self::Vorbis { name: b }) => a.eq_ignore_ascii_case(b),
            (
                Self::Id3 {
                    frame: a,
                    description: ad,
                    language: al,
                    ..
                },
                Self::Id3 {
                    frame: b,
                    description: bd,
                    language: bl,
                    ..
                },
            ) => a == b && ad == bd && al == bl,
            (Self::Mp4 { key: a, .. }, Self::Mp4 { key: b, .. }) => a == b,
            _ => false,
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Vorbis { name } => name.len(),
            Self::Id3 {
                frame, description, ..
            } => frame.len() + description.as_ref().map_or(0, String::len),
            Self::Mp4 {
                key: Mp4TextKey::Freeform { mean, name },
                ..
            } => mean.len() + name.len(),
            Self::Mp4 { .. } => 4,
        }
    }
}

fn logical_field(target: &NativeTextTarget) -> Option<TagField> {
    use lofty::tag::{ItemKey, TagType};
    let (kind, name) = match target {
        NativeTextTarget::Vorbis { name } => {
            // The compatibility facade carries URL as WWW on Vorbis; native
            // field ownership must use that same established mapping.
            if let Some(field) = super::field_for_name_on_tag(name, TagType::VorbisComments) {
                return Some(field);
            }
            (TagType::VorbisComments, name.clone())
        }
        NativeTextTarget::Id3 {
            frame, description, ..
        } if frame == "TXXX" => {
            return Some(TagField::Custom(description.clone().unwrap_or_default()))
        }
        NativeTextTarget::Id3 { frame, .. } => (TagType::Id3v2, frame.clone()),
        NativeTextTarget::Mp4 {
            key: Mp4TextKey::Freeform { mean, name },
            ..
        } => {
            let key = format!("----:{mean}:{name}");
            return Some(
                ItemKey::from_key(TagType::Mp4Ilst, &key)
                    .map(|key| super::item_key_to_tag_field(key, TagType::Mp4Ilst))
                    .unwrap_or_else(|| TagField::Custom(name.clone())),
            );
        }
        NativeTextTarget::Mp4 {
            key: Mp4TextKey::Fourcc(key),
            ..
        } => (
            TagType::Mp4Ilst,
            key.iter().map(|byte| char::from(*byte)).collect(),
        ),
    };
    Some(
        ItemKey::from_key(kind, &name)
            .map(|key| super::item_key_to_tag_field(key, kind))
            .unwrap_or_else(|| TagField::Custom(name)),
    )
}
fn is_typed_field(field: &TagField) -> bool {
    matches!(
        field,
        TagField::TrackNumber
            | TagField::TrackTotal
            | TagField::DiscNumber
            | TagField::DiscTotal
            | TagField::Year
            | TagField::Bpm
    )
}

struct Builder {
    entries: Vec<ExactTextEntry>,
    groups: BTreeMap<NativeTextTarget, usize>,
    bytes: usize,
    retained: Lease,
}
impl Builder {
    fn new() -> Result<Self, ExactReadError> {
        Ok(Self {
            entries: Vec::new(),
            groups: BTreeMap::new(),
            bytes: 512,
            retained: Budget::process().try_acquire(Charge {
                retained: 512,
                ..Default::default()
            })?,
        })
    }
    fn push(
        &mut self,
        target: NativeTextTarget,
        values: Result<Vec<String>, UnavailableReason>,
        append: bool,
    ) -> Result<(), ExactReadError> {
        target.validate()?;
        if values.as_ref().is_ok_and(|values| {
            values.len() > 1024 || values.iter().map(String::len).sum::<usize>() > MAX_FIELD_BYTES
        }) {
            return Err(ExactReadError::Limit);
        }
        let state = values.and_then(|values| {
            ExactTextState::try_from(values).map_err(|_| UnavailableReason::InvalidText)
        });
        let additional = state
            .as_ref()
            .ok()
            .and_then(ExactTextState::values)
            .map_or(0, |values| {
                values
                    .as_slice()
                    .iter()
                    .map(|value| value.capacity() + 32)
                    .sum::<usize>()
            });
        self.bytes = self
            .bytes
            .checked_add(512 + target.bytes().saturating_mul(2) + additional)
            .ok_or(ExactReadError::Limit)?;
        if self.bytes > MAX_SNAPSHOT_BYTES {
            return Err(ExactReadError::Limit);
        }
        self.retained.try_resize(Charge {
            retained: self.bytes,
            ..Default::default()
        })?;
        if append {
            if let Some(index) = self.groups.get(&target).copied() {
                let previous = &mut self.entries[index].state;
                match (previous.as_mut(), state) {
                    (Ok(old), Ok(new)) => old
                        .0
                        .as_mut()
                        .ok_or(ExactReadError::Invalid("Missing native text values"))?
                        .append(
                            new.0
                                .ok_or(ExactReadError::Invalid("Missing appended values"))?,
                        )
                        .map_err(|_| ExactReadError::Limit)?,
                    (_, Err(reason)) => *previous = Err(reason),
                    (Err(_), _) => {}
                }
                return Ok(());
            }
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(ExactReadError::Limit);
        }
        self.groups.insert(target.clone(), self.entries.len());
        self.entries.push(ExactTextEntry {
            field: logical_field(&target),
            target,
            state,
        });
        Ok(())
    }
}

pub(super) fn prepare_input(
    path: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<raw::Input, ExactReadError> {
    raw::Input::prepare(path, cancelled)
}

pub(super) fn prepare_input_file(
    file: std::fs::File,
    cancelled: &dyn Fn() -> bool,
) -> Result<raw::Input, ExactReadError> {
    raw::Input::prepare_file(file, cancelled)
}

pub(super) fn suspend_source() -> source::Scope {
    source::Scope::suspend()
}

pub(super) fn source_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    source::open(path)
}
pub(super) fn source_bytes(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    source::read_all(path)
}

pub(super) fn snapshot(
    input: &raw::Input,
    container: AudioFormat,
    selected: Option<TagBlockKind>,
    read_policy: ExactReadPolicy,
) -> Result<ExactTextSnapshot, ExactReadError> {
    use sha2::{Digest, Sha256};
    let mut builder = Builder::new()?;
    let mut matches = input
        .blocks
        .iter()
        .filter(|block| Some(block.kind) == selected);
    let block = matches.next();
    let mut unavailable = if matches.next().is_some() {
        Some(UnavailableReason::AmbiguousIdentity)
    } else if block.is_none() {
        Some(if selected.is_none() {
            UnavailableReason::NoSelectedBlock
        } else {
            UnavailableReason::UnsupportedLayout
        })
    } else {
        None
    };
    let id3_revision = block.and_then(|block| block.revision);
    if unavailable.is_none() {
        if let Some(block) = block {
            match block.kind {
                TagBlockKind::VorbisComments => {
                    for (name, value) in raw::vorbis_items(&block.bytes)? {
                        let values = if matches!(
                            name.to_ascii_uppercase().as_str(),
                            "METADATA_BLOCK_PICTURE" | "COVERART" | "COVERARTMIME"
                        ) {
                            Err(UnavailableReason::NonText)
                        } else {
                            Ok(vec![value.to_owned()])
                        };
                        builder.push(
                            NativeTextTarget::Vorbis { name: name.into() },
                            values,
                            true,
                        )?;
                    }
                }
                TagBlockKind::Id3v2 => read_id3(block, &mut builder, &mut unavailable)?,
                TagBlockKind::Mp4Ilst => read_mp4(&block.bytes, &mut builder)?,
                _ => unavailable = Some(UnavailableReason::UnsupportedLayout),
            }
        }
    }
    Ok(ExactTextSnapshot {
        context: NativeTextContext {
            container,
            selected_block: selected,
            id3_revision,
            policy_revision: read_policy.revision,
            adapter_version: 1,
            raw_digest: block.map(|block| Sha256::digest(&block.bytes).into()),
            read_policy,
        },
        entries: builder.entries,
        unavailable,
        _retained: builder.retained,
        bytes_owned: builder.bytes,
    })
}

fn decode_id3(bytes: &[u8], encoding: Id3TextEncoding) -> Result<String, ExactReadError> {
    if bytes.len() > MAX_FIELD_BYTES {
        return Err(ExactReadError::Limit);
    }
    let text = match encoding {
        Id3TextEncoding::Utf8 => std::str::from_utf8(bytes)
            .map_err(|_| ExactReadError::Invalid("Invalid UTF-8 ID3 text"))?
            .to_owned(),
        Id3TextEncoding::Latin1 => match super::legacy_code_page() {
            Some(encoding) => encoding
                .decode_without_bom_handling_and_without_replacement(bytes)
                .ok_or(ExactReadError::Invalid(
                    "Invalid configured ID3 codepage text",
                ))?
                .into_owned(),
            None => bytes.iter().map(|byte| char::from(*byte)).collect(),
        },
    };
    if text.len() > MAX_FIELD_BYTES {
        return Err(ExactReadError::Limit);
    }
    Ok(text)
}
fn read_id3(
    block: &raw::RawBlock,
    builder: &mut Builder,
    unavailable: &mut Option<UnavailableReason>,
) -> Result<(), ExactReadError> {
    let mut at = 0;
    while at < block.bytes.len() && block.bytes[at] != 0 {
        let header = &block.bytes[at..at + 10]; // raw envelope preflight validated bounds.
        let frame = std::str::from_utf8(&header[..4])
            .map_err(|_| ExactReadError::Invalid("Invalid frame name"))?
            .to_owned();
        let size = if block.revision == Some(Id3v2Revision::V4) {
            raw::syncsafe(&header[4..8])?
        } else {
            u32::from_be_bytes(
                header[4..8]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("Frame size"))?,
            )
        } as usize;
        let bytes = &block.bytes[at + 10..at + 10 + size];
        at += 10 + size;
        if !frame.starts_with('T') && !matches!(frame.as_str(), "COMM" | "USLT") {
            builder.push(
                NativeTextTarget::Id3 {
                    frame,
                    description: None,
                    language: None,
                    encoding: None,
                },
                Err(UnavailableReason::NonText),
                false,
            )?;
            continue;
        }
        let encoding = match bytes.first() {
            Some(0) => Some(Id3TextEncoding::Latin1),
            Some(3) if block.revision == Some(Id3v2Revision::V4) => Some(Id3TextEncoding::Utf8),
            _ => None,
        };
        let qualified = matches!(frame.as_str(), "COMM" | "USLT");
        let mut description = None;
        let mut language = None;
        let mut payload = bytes.get(1..).unwrap_or_default();
        if qualified {
            let lang = payload
                .get(..3)
                .ok_or(ExactReadError::Invalid("Truncated qualified ID3 frame"))?;
            language = Some(
                lang.try_into()
                    .map_err(|_| ExactReadError::Invalid("ID3 language"))?,
            );
            payload = &payload[3..];
        }
        if qualified || frame == "TXXX" {
            if let Some(encoding) = encoding {
                let end = payload
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(ExactReadError::Invalid("Unterminated ID3 description"))?;
                if end > MAX_NAME {
                    return Err(ExactReadError::Limit);
                }
                description = Some(decode_id3(&payload[..end], encoding)?);
                payload = &payload[end + 1..];
            } else {
                // A description whose encoding we cannot prove could alias any
                // qualified target. Keep the diagnostic and refuse block-wide
                // absence/patch claims rather than inventing a description.
                description = Some(String::new());
                *unavailable = Some(UnavailableReason::UnsupportedEncoding);
            }
        }
        let target = NativeTextTarget::Id3 {
            frame,
            description,
            language,
            encoding,
        };
        let values = match encoding {
            None => Err(UnavailableReason::UnsupportedEncoding),
            Some(encoding) => {
                let value = decode_id3(payload, encoding)?;
                if block.revision == Some(Id3v2Revision::V4) && !qualified {
                    let count = value.bytes().filter(|byte| *byte == 0).count() + 1;
                    if count > 1024 {
                        return Err(ExactReadError::Limit);
                    }
                    Ok(value.split('\0').map(str::to_owned).collect())
                } else {
                    Ok(vec![value])
                }
            }
        };
        builder.push(target, values, false)?;
    }
    Ok(())
}

fn decode_mp4(bytes: &[u8], kind: Mp4TextKind) -> Result<Vec<String>, ExactReadError> {
    if bytes.len() > MAX_FIELD_BYTES {
        return Err(ExactReadError::Limit);
    }
    let value = match kind {
        Mp4TextKind::Utf8 => std::str::from_utf8(bytes)
            .map_err(|_| ExactReadError::Invalid("Invalid MP4 UTF-8"))?
            .to_owned(),
        Mp4TextKind::Utf16 => {
            if !bytes.len().is_multiple_of(2) {
                return Err(ExactReadError::Invalid("Odd MP4 UTF-16 length"));
            }
            let words = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|word| u16::from_be_bytes([word[0], word[1]]));
            let mut value = String::new();
            for ch in char::decode_utf16(words) {
                let ch =
                    ch.map_err(|_| ExactReadError::Invalid("Unpaired MP4 UTF-16 surrogate"))?;
                if value.len() + ch.len_utf8() > MAX_FIELD_BYTES {
                    return Err(ExactReadError::Limit);
                }
                value.push(ch);
            }
            value
        }
        Mp4TextKind::Other(_) => {
            return Err(ExactReadError::Unavailable(UnavailableReason::NonText))
        }
    };
    Ok(vec![value])
}
fn read_mp4(bytes: &[u8], builder: &mut Builder) -> Result<(), ExactReadError> {
    for (fourcc, body) in raw::atoms(bytes)? {
        let children = raw::atoms(body)?;
        let unhandled = children
            .iter()
            .any(|(kind, _)| !matches!(kind, b"data" | b"mean" | b"name"));
        let key = if fourcc == *b"----" {
            let named = |wanted: [u8; 4]| -> Result<String, ExactReadError> {
                let mut values = children.iter().filter(|(key, _)| *key == wanted);
                let (_, bytes) = values
                    .next()
                    .ok_or(ExactReadError::Invalid("Missing MP4 namespace/name"))?;
                if values.next().is_some() || bytes.get(..4) != Some(&[0; 4]) {
                    return Err(ExactReadError::Unavailable(
                        UnavailableReason::AmbiguousIdentity,
                    ));
                }
                let value = bytes
                    .get(4..)
                    .ok_or(ExactReadError::Invalid("MP4 namespace header"))?;
                if value.len() > MAX_NAME {
                    return Err(ExactReadError::Limit);
                }
                Ok(std::str::from_utf8(value)
                    .map_err(|_| ExactReadError::Invalid("MP4 namespace UTF-8"))?
                    .into())
            };
            Mp4TextKey::Freeform {
                mean: named(*b"mean")?,
                name: named(*b"name")?,
            }
        } else {
            Mp4TextKey::Fourcc(fourcc)
        };
        let mut targets = BTreeMap::<NativeTextTarget, Vec<String>>::new();
        let mut unavailable = Vec::new();
        for (child, data) in children {
            if child != *b"data" {
                continue;
            }
            if data.len() < 8 {
                return Err(ExactReadError::Invalid("Truncated MP4 data qualifier"));
            }
            let code = u32::from_be_bytes(
                data[..4]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("MP4 data kind"))?,
            );
            let locale = u32::from_be_bytes(
                data[4..8]
                    .try_into()
                    .map_err(|_| ExactReadError::Invalid("MP4 locale"))?,
            );
            let kind = match code {
                1 => Mp4TextKind::Utf8,
                2 => Mp4TextKind::Utf16,
                other => Mp4TextKind::Other(other),
            };
            let target = NativeTextTarget::Mp4 {
                key: key.clone(),
                kind,
                locale,
            };
            if matches!(kind, Mp4TextKind::Other(_)) {
                unavailable.push(target);
                continue;
            }
            let values = targets.entry(target).or_default();
            values.extend(decode_mp4(&data[8..], kind)?);
            if values.len() > 1024
                || values.iter().map(String::len).sum::<usize>() > MAX_FIELD_BYTES
            {
                return Err(ExactReadError::Limit);
            }
        }
        if targets.is_empty() && unavailable.is_empty() {
            unavailable.push(NativeTextTarget::Mp4 {
                key: key.clone(),
                kind: Mp4TextKind::Other(0),
                locale: 0,
            });
        }
        for (target, values) in targets {
            builder.push(
                target,
                if unhandled {
                    Err(UnavailableReason::UnsupportedLayout)
                } else {
                    Ok(values)
                },
                false,
            )?;
        }
        for target in unavailable {
            builder.push(target, Err(UnavailableReason::NonText), false)?;
        }
    }
    Ok(())
}
