//! Structured read-only report previews using Tagnetic masks and supplied rows.
//! Rendering does not probe files or publish output. The caller supplies the
//! staged metadata/technical snapshot and uses the guarded output publisher.
use crate::{
    mask::{BoundedRenderError, FileContext, Mask, MaskError},
    model::{read_policy_revision, TagMap},
    resources::{AdmissionError, Budget, Charge, Lease},
    text_codec::{self, TextEncoding, TextError},
};
use std::collections::HashMap;

const MAX_SOURCE: usize = 64 * 1024;
const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 8;
const MAX_ROWS: usize = 100_000;
const MAX_GROUPS: usize = 4096;
const MAX_GROUP_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_ENCODED: usize = 8 * 1024 * 1024;
const WORK_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid report template: {0}")]
    Invalid(&'static str),
    #[error("Report exceeds its bounded rendering limits")]
    Limit,
    #[error("Report rendering was cancelled")]
    Cancelled,
    #[error("Metadata interpretation changed during report rendering")]
    Changed,
    #[error(transparent)]
    Mask(MaskError),
    #[error(transparent)]
    Text(#[from] TextError),
    #[error(transparent)]
    Resource(#[from] AdmissionError),
}

impl From<MaskError> for Error {
    fn from(mut error: MaskError) -> Self {
        let detail = match &mut error {
            MaskError::UnknownPlaceholder(detail)
            | MaskError::UnknownFunction(detail)
            | MaskError::UnclosedCall(detail)
            | MaskError::BadArgument(detail)
            | MaskError::MissingTag(detail) => Some(detail),
            MaskError::BadArity { expected, .. } => Some(expected),
            _ => None,
        };
        if let Some(detail) = detail {
            if detail.len() > 1024 || detail.capacity() > 2048 {
                let mut end = detail.len().min(1024);
                while !detail.is_char_boundary(end) {
                    end -= 1;
                }
                let mut bounded = detail[..end].to_owned();
                if end < detail.len() {
                    bounded.push('…');
                }
                *detail = bounded;
            }
        }
        Self::Mask(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Escape {
    Plain,
    HtmlText,
    XmlText,
    CsvCell,
}
#[derive(Debug, Clone, Copy)]
pub enum GroupOrder {
    Encounter,
    Lexical,
}
#[derive(Debug, Clone, Copy)]
pub enum Aggregate {
    Count,
    KnownDurationSeconds,
    UnknownDurations,
}

/// Version 1 consumes legacy scalar metadata explicitly. It never infers lists
/// from punctuation or claims to expose exact native repeated-value state.
#[derive(Debug)]
pub struct Template {
    pub version: u8,
    pub nodes: Vec<Node>,
}
#[derive(Debug)]
pub enum Node {
    Literal(String),
    Mask {
        pattern: String,
        escape: Escape,
    },
    EachTrack(Vec<Node>),
    GroupBy {
        key_pattern: String,
        order: GroupOrder,
        body: Vec<Node>,
    },
    GroupKey(Escape),
    Aggregate(Aggregate),
}
#[derive(Debug)]
enum CompiledNode {
    Literal(String),
    Mask(Mask, Escape),
    EachTrack(Vec<Self>),
    GroupBy(Mask, GroupOrder, Vec<Self>),
    GroupKey(Escape),
    Aggregate(Aggregate),
}
#[derive(Debug)]
pub struct CompiledTemplate {
    nodes: Vec<CompiledNode>,
    budget: Budget,
    _owner: Lease,
}

#[derive(Clone, Copy)]
pub struct Row<'a> {
    pub tags: &'a TagMap,
    pub file: FileContext<'a>,
    /// None is unknown. A known zero duration contributes zero and is counted.
    pub duration_seconds: Option<u64>,
}
#[derive(Debug)]
pub struct Rendered {
    bytes: Vec<u8>,
    revision: u64,
    _owner: Lease,
}
impl Rendered {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn policy_revision(&self) -> u64 {
        self.revision
    }
}

impl CompiledTemplate {
    pub fn compile(template: &Template) -> Result<Self, Error> {
        Self::compile_in(template, Budget::process())
    }
    pub fn compile_in(template: &Template, budget: Budget) -> Result<Self, Error> {
        if template.version != 1 {
            return Err(Error::Invalid("unsupported version"));
        }
        let mut count = 0usize;
        let mut source = 0usize;
        validate(&template.nodes, 0, false, &mut count, &mut source)?;
        // Rendering-only masks skip extraction-regex compilation. This
        // conservative source/AST allowance is reserved before compilation.
        let bytes = compile_charge(&template.nodes).ok_or(Error::Limit)?;
        let owner = budget.try_acquire(Charge {
            retained: bytes,
            ..Default::default()
        })?;
        let nodes = compile(&template.nodes)?;
        Ok(Self {
            nodes,
            budget,
            _owner: owner,
        })
    }
    pub fn render(
        &self,
        rows: &[Row<'_>],
        encoding: TextEncoding,
        bom: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<Rendered, Error> {
        if rows.len() > MAX_ROWS
            || rows.iter().any(|row| row.tags.len() > 4096)
            || rows
                .iter()
                .try_fold(0usize, |sum, row| sum.checked_add(row.tags.len()))
                .is_none_or(|sum| sum > 1_000_000)
        {
            return Err(Error::Limit);
        }
        let mut owner = self.budget.try_acquire(Charge {
            transient: WORK_BYTES,
            jobs: 1,
            ..Default::default()
        })?;
        let revision = read_policy_revision();
        let mut state = State {
            rows,
            budget: &self.budget,
            cancelled: &cancelled,
            revision,
            output: String::new(),
            visits: 0,
            group_bytes: 0,
        };
        state.check()?;
        let indices: Vec<_> = (0..rows.len()).collect();
        state.nodes(&self.nodes, &indices, None)?;
        state.check()?;
        let bytes =
            text_codec::encode_with_cancel(&state.output, encoding, bom, MAX_ENCODED, || {
                cancelled() || read_policy_revision() != revision
            });
        state.check()?;
        let bytes = bytes?;
        drop(state);
        drop(indices);
        owner.try_resize(Charge {
            retained: bytes.capacity().saturating_add(512),
            ..Default::default()
        })?;
        Ok(Rendered {
            bytes,
            revision,
            _owner: owner,
        })
    }
}

fn validate(
    nodes: &[Node],
    depth: usize,
    in_group: bool,
    count: &mut usize,
    source: &mut usize,
) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::Limit);
    }
    for node in nodes {
        *count = count.checked_add(1).ok_or(Error::Limit)?;
        if *count > MAX_NODES {
            return Err(Error::Limit);
        }
        let text = match node {
            Node::Literal(text) => Some(text.as_str()),
            Node::Mask { pattern, .. } => Some(pattern.as_str()),
            Node::GroupBy { key_pattern, .. } => Some(key_pattern.as_str()),
            _ => None,
        };
        if let Some(text) = text {
            *source = source.checked_add(text.len()).ok_or(Error::Limit)?;
            if *source > MAX_SOURCE {
                return Err(Error::Limit);
            }
        }
        match node {
            Node::Mask { pattern, .. }
            | Node::GroupBy {
                key_pattern: pattern,
                ..
            } if pattern.len() > 2048 => return Err(Error::Limit),
            Node::EachTrack(body) => validate(body, depth + 1, in_group, count, source)?,
            Node::GroupBy { body, .. } => validate(body, depth + 1, true, count, source)?,
            Node::GroupKey(_) if !in_group => {
                return Err(Error::Invalid("group key outside a group"))
            }
            _ => (),
        }
    }
    Ok(())
}
fn compile_charge(nodes: &[Node]) -> Option<usize> {
    nodes.iter().try_fold(4096usize, |sum, node| {
        let own = match node {
            Node::Literal(text) => text.len().checked_mul(2)?,
            Node::Mask { pattern, .. } => pattern.len().checked_mul(512)?.checked_add(8192)?,
            Node::GroupBy {
                key_pattern, body, ..
            } => key_pattern
                .len()
                .checked_mul(512)?
                .checked_add(8192)?
                .checked_add(compile_charge(body)?)?,
            Node::EachTrack(body) => compile_charge(body)?,
            _ => 0,
        };
        sum.checked_add(own)?.checked_add(512)
    })
}

fn compile_mask(pattern: &str) -> Result<Mask, Error> {
    let mask = Mask::parse_for_render(pattern)?;
    if mask.is_extract_only() {
        return Err(Error::Invalid(
            "extract-only mask cannot produce report text",
        ));
    }
    Ok(mask)
}

fn compile(nodes: &[Node]) -> Result<Vec<CompiledNode>, Error> {
    nodes
        .iter()
        .map(|node| {
            Ok(match node {
                Node::Literal(text) => CompiledNode::Literal(text.clone()),
                Node::Mask { pattern, escape } => {
                    CompiledNode::Mask(compile_mask(pattern)?, *escape)
                }
                Node::EachTrack(body) => CompiledNode::EachTrack(compile(body)?),
                Node::GroupBy {
                    key_pattern,
                    order,
                    body,
                } => CompiledNode::GroupBy(compile_mask(key_pattern)?, *order, compile(body)?),
                Node::GroupKey(escape) => CompiledNode::GroupKey(*escape),
                Node::Aggregate(kind) => CompiledNode::Aggregate(*kind),
            })
        })
        .collect()
}

struct State<'a, 'row, C> {
    rows: &'a [Row<'row>],
    budget: &'a Budget,
    cancelled: &'a C,
    revision: u64,
    output: String,
    visits: usize,
    group_bytes: usize,
}
impl<C: Fn() -> bool> State<'_, '_, C> {
    fn check(&self) -> Result<(), Error> {
        let cancelled = (self.cancelled)();
        if read_policy_revision() != self.revision || self.revision == u64::MAX {
            return Err(Error::Changed);
        }
        if cancelled {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    fn reserve_visits(&mut self, count: usize) -> Result<(), Error> {
        self.check()?;
        self.visits = self
            .visits
            .checked_add(count)
            .filter(|n| *n <= 2_000_000)
            .ok_or(Error::Limit)?;
        Ok(())
    }
    fn mask(&mut self, mask: &Mask, index: Option<usize>) -> Result<String, Error> {
        self.reserve_visits(1)?;
        let empty = TagMap::new();
        let row = index.map(|i| self.rows[i]).unwrap_or(Row {
            tags: &empty,
            file: FileContext::default(),
            duration_seconds: None,
        });
        let work = mask
            .render_work_bytes(row.tags, &row.file)
            .ok_or(Error::Limit)?;
        let _mask_work = self.budget.try_acquire(Charge {
            transient: work,
            ..Default::default()
        })?;
        let rendered = mask
            .render_value_with_budget(row.tags, &row.file, work)
            .map_err(|error| match error {
                BoundedRenderError::TooLarge => Error::Limit,
                BoundedRenderError::Mask(error) => Error::from(error),
            })?;
        self.check()?;
        Ok(rendered)
    }
    fn nodes(
        &mut self,
        nodes: &[CompiledNode],
        indices: &[usize],
        key: Option<&str>,
    ) -> Result<(), Error> {
        for node in nodes {
            self.reserve_visits(1)?;
            match node {
                CompiledNode::Literal(text) => append(&mut self.output, text)?,
                CompiledNode::Mask(mask, escape) => {
                    let value = self.mask(mask, indices.first().copied())?;
                    self.escaped(&value, *escape)?;
                }
                CompiledNode::EachTrack(body) => {
                    self.reserve_visits(indices.len())?;
                    for &index in indices {
                        self.nodes(body, &[index], key)?;
                    }
                }
                CompiledNode::GroupBy(mask, order, body) => {
                    self.groups(mask, *order, body, indices)?
                }
                CompiledNode::GroupKey(escape) => {
                    self.escaped(key.ok_or(Error::Invalid("group key unavailable"))?, *escape)?
                }
                CompiledNode::Aggregate(kind) => {
                    let value = match kind {
                        Aggregate::Count => indices.len().to_string(),
                        Aggregate::UnknownDurations | Aggregate::KnownDurationSeconds => {
                            self.reserve_visits(indices.len())?;
                            let mut total = 0u64;
                            for (position, &index) in indices.iter().enumerate() {
                                if position.is_multiple_of(128) {
                                    self.check()?;
                                }
                                let value = match kind {
                                    Aggregate::UnknownDurations => {
                                        u64::from(self.rows[index].duration_seconds.is_none())
                                    }
                                    _ => self.rows[index].duration_seconds.unwrap_or(0),
                                };
                                total = total.checked_add(value).ok_or(Error::Limit)?;
                            }
                            total.to_string()
                        }
                    };
                    append(&mut self.output, &value)?;
                }
            }
        }
        Ok(())
    }
    fn escaped(&mut self, value: &str, escape: Escape) -> Result<(), Error> {
        if matches!(escape, Escape::Plain) {
            return append(&mut self.output, value);
        }
        let csv = matches!(escape, Escape::CsvCell);
        if csv {
            append(&mut self.output, "\"")?;
        }
        for (index, character) in value.chars().enumerate() {
            if index.is_multiple_of(1024) {
                self.check()?;
            }
            if matches!(escape, Escape::XmlText)
                && !(matches!(character, '\t' | '\r' | '\n')
                    || ('\u{20}'..='\u{d7ff}').contains(&character)
                    || ('\u{e000}'..='\u{fffd}').contains(&character)
                    || ('\u{10000}'..='\u{10ffff}').contains(&character))
            {
                return Err(Error::Invalid("character cannot appear in XML text"));
            }
            let mut buffer = [0u8; 4];
            let part = if csv {
                if character == '"' {
                    "\"\""
                } else {
                    character.encode_utf8(&mut buffer)
                }
            } else {
                match character {
                    '&' => "&amp;",
                    '<' => "&lt;",
                    '>' => "&gt;",
                    '"' => "&quot;",
                    '\'' => "&apos;",
                    '\r' => "&#13;",
                    other => other.encode_utf8(&mut buffer),
                }
            };
            append(&mut self.output, part)?;
        }
        if csv {
            append(&mut self.output, "\"")?;
        }
        Ok(())
    }
    fn groups(
        &mut self,
        mask: &Mask,
        order: GroupOrder,
        body: &[CompiledNode],
        indices: &[usize],
    ) -> Result<(), Error> {
        let previous = self.group_bytes;
        let result = (|| {
            let mut lookup = HashMap::<String, usize>::new();
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            for &index in indices {
                let key = self.mask(mask, Some(index))?;
                let slot = if let Some(slot) = lookup.get(&key) {
                    *slot
                } else {
                    if groups.len() == MAX_GROUPS {
                        return Err(Error::Limit);
                    }
                    self.group_bytes = self
                        .group_bytes
                        .checked_add(key.capacity().saturating_mul(2) + 512)
                        .ok_or(Error::Limit)?;
                    if self.group_bytes > MAX_GROUP_BYTES {
                        return Err(Error::Limit);
                    }
                    let slot = groups.len();
                    lookup.insert(key.clone(), slot);
                    groups.push((key, Vec::new()));
                    slot
                };
                let members = &mut groups[slot].1;
                if members.len() == members.capacity() {
                    let capacity = members.capacity().saturating_mul(2).max(4);
                    self.group_bytes = self
                        .group_bytes
                        .checked_add((capacity - members.capacity()) * std::mem::size_of::<usize>())
                        .ok_or(Error::Limit)?;
                    if self.group_bytes > MAX_GROUP_BYTES {
                        return Err(Error::Limit);
                    }
                    members
                        .try_reserve_exact(capacity - members.len())
                        .map_err(|_| Error::Limit)?;
                }
                members.push(index);
            }
            drop(lookup);
            if matches!(order, GroupOrder::Lexical) {
                groups.sort_by(|a, b| a.0.cmp(&b.0));
            }
            for (key, members) in &groups {
                self.nodes(body, members, Some(key))?;
            }
            Ok(())
        })();
        self.group_bytes = previous;
        result
    }
}
fn append(output: &mut String, part: &str) -> Result<(), Error> {
    let len = output
        .len()
        .checked_add(part.len())
        .filter(|n| *n <= MAX_TEXT)
        .ok_or(Error::Limit)?;
    if len > output.capacity() {
        let target = len
            .max(output.capacity().saturating_mul(2).min(MAX_TEXT))
            .clamp(1024, MAX_TEXT);
        output
            .try_reserve_exact(target - output.len())
            .map_err(|_| Error::Limit)?;
    }
    output.push_str(part);
    Ok(())
}
