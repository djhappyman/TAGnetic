//! Read-only library queries over an immutable, reusable display snapshot.
//!
//! The interface sends small query descriptions. File probing is confined to
//! snapshot creation; matching and suggestions perform no filesystem access.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::SystemTime;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::TrackDto;

static SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);
const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 8;
const FIELDS: &[&str] = &[
    "artist",
    "albumartist",
    "album_artist",
    "album",
    "title",
    "genre",
    "year",
    "date",
    "label",
    "publisher",
    "comment",
    "comments",
    "composer",
    "conductor",
    "remixer",
    "isrc",
    "bpm",
    "key",
    "initialkey",
    "track",
    "tracknumber",
    "tracktotal",
    "disc",
    "discnumber",
    "disctotal",
    "position",
    "file",
    "filename",
    "path",
    "folder",
    "format",
    "cover",
    "artwork",
    "duration",
    "length",
    "bitrate",
    "samplerate",
    "sample_rate",
    "channels",
    "size",
    "unreadable",
    "tagtypes",
    "compilation",
    "copyright",
    "lyrics",
    "grouping",
    "subtitle",
    "version",
    "catalog_number",
    "url",
    "catalognumber",
    "media",
    "encodedby",
    "encoder",
    "organization",
    "rating",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryModel {
    pub version: u8,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    pub root: QueryNode,
    #[serde(default = "search_mode")]
    pub mode: String,
    #[serde(default = "exact_kind")]
    pub duplicate_kind: String,
    #[serde(default)]
    pub sort: Option<QuerySort>,
    #[serde(default)]
    pub cover_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub custom_fields: Vec<CustomField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomField {
    pub key: String,
    pub mask: String,
}

fn search_mode() -> String {
    "search".into()
}
fn exact_kind() -> String {
    "exact".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QueryNode {
    Group {
        id: String,
        mode: String,
        children: Vec<QueryNode>,
    },
    Condition {
        id: String,
        field: String,
        operator: String,
        #[serde(default)]
        values: Vec<String>,
        #[serde(default)]
        negate: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySort {
    pub field: String,
    pub direction: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryRequest {
    pub request_id: u64,
    #[serde(default)]
    pub snapshot_id: Option<u64>,
    pub query: QueryModel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub request_id: u64,
    pub snapshot_id: u64,
    pub total: usize,
    pub matched: usize,
    pub indices: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    pub elapsed_ms: f64,
    pub cancelled: bool,
    pub duplicate_status: String,
    pub groups: Vec<DuplicateQueryGroup>,
    pub groups_revision: u64,
    pub groups_count: usize,
    pub skipped: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateQueryGroup {
    pub id: String,
    pub label: String,
    pub reason: String,
    pub confidence: String,
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct DuplicateResults {
    pub status: String,
    pub groups: Vec<DuplicateQueryGroup>,
    pub groups_revision: u64,
    pub skipped: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuggestRequest {
    pub request_id: u64,
    #[serde(default)]
    pub snapshot_id: Option<u64>,
    pub query: QueryModel,
    pub text: String,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default = "suggest_limit")]
    pub limit: usize,
    #[serde(default = "suggest_strategy")]
    pub strategy: String,
}
fn suggest_limit() -> usize {
    12
}
fn suggest_strategy() -> String {
    "values".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestResult {
    pub request_id: u64,
    pub snapshot_id: u64,
    pub suggestions: Vec<Suggestion>,
    pub truncated: bool,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub field: String,
    pub value: String,
    pub count: usize,
    pub operator: String,
}

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("{0}")]
    Invalid(String),
    #[error("Query superseded by a newer request or library revision.")]
    Cancelled,
    #[error(transparent)]
    Resource(#[from] tagrex_core::resources::AdmissionError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Value {
    raw: String,
    folded: String,
}
impl Value {
    fn new(raw: String) -> Self {
        Self {
            folded: raw.to_lowercase(),
            raw,
        }
    }
}

// Accounting identity is deliberately excluded from semantic row equality.
// Shared saved strings keep this owner alive when rows survive snapshot replacement.
#[derive(Debug, Clone, Default)]
struct RowOwnership {
    _lease: Option<Arc<tagrex_core::resources::Lease>>,
}
impl PartialEq for RowOwnership {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl Eq for RowOwnership {}

/// A single row's searchable saved metadata. Technical properties are optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRow {
    ownership: RowOwnership,
    pub path: String,
    values: Arc<BTreeMap<String, Value>>,
    technical: BTreeMap<String, Value>,
    computed: Arc<BTreeMap<String, Value>>,
    overlay_ownership: RowOwnership,
    search: Arc<Value>,
    format: Option<tagrex_core::model::AudioFormat>,
    props: Option<tagrex_core::model::AudioProps>,
    modified: Option<SystemTime>,
    source_fingerprint: Option<crate::read_cache::Fingerprint>,
}

impl QueryRow {
    /// Admit saved text, case-folded text and the combined search buffer before
    /// cloning them. The conservative reservation also covers map nodes and
    /// technical fields added before the row is published.
    pub fn try_from_track(
        track: &TrackDto,
    ) -> Result<Self, tagrex_core::resources::AdmissionError> {
        Self::try_from_track_in(track, &tagrex_core::resources::Budget::process())
    }

    pub fn try_from_track_in(
        track: &TrackDto,
        budget: &tagrex_core::resources::Budget,
    ) -> Result<Self, tagrex_core::resources::AdmissionError> {
        use tagrex_core::resources::{AdmissionError, Charge};
        // A native file has a small set of tag blocks. Bound externally built
        // DTOs too: even empty labels need join separators and a temporary
        // reference vector before any saved string can be measured.
        if track.tag_blocks.len() > 256 {
            return Err(AdmissionError::TooLarge);
        }
        let bytes = track
            .tags
            .iter()
            .try_fold(0usize, |sum, (key, value)| {
                sum.checked_add(key.len())?.checked_add(value.len())
            })
            .and_then(|sum| sum.checked_add(track.path.len().checked_mul(4)?))
            .and_then(|sum| sum.checked_add(track.format.len()))
            .and_then(|sum| {
                sum.checked_add(
                    track
                        .tag_blocks
                        .iter()
                        .try_fold(0usize, |n, b| n.checked_add(b.label.len()))?,
                )
            })
            .and_then(|sum| sum.checked_mul(12))
            .and_then(|sum| sum.checked_add(track.tag_blocks.len().checked_mul(128)?))
            .and_then(|sum| sum.checked_add(track.tags.len().checked_add(16)?.checked_mul(512)?))
            .and_then(|sum| sum.checked_add(4096))
            .ok_or(AdmissionError::TooLarge)?;
        let mut lease = budget.try_acquire(Charge {
            retained: bytes,
            ..Charge::default()
        })?;
        let mut row = Self::from_track(track);
        let measured = row
            .values
            .iter()
            .fold(4096usize + row.path.capacity(), |sum, (key, value)| {
                sum.saturating_add(512)
                    .saturating_add(key.capacity())
                    .saturating_add(value.raw.capacity())
                    .saturating_add(value.folded.capacity())
            })
            .saturating_add(row.search.raw.capacity())
            .saturating_add(row.search.folded.capacity());
        // The admission estimate must dominate the live allocation. Refuse an
        // unexpected shape rather than publishing an under-accounted row.
        if measured > bytes {
            return Err(AdmissionError::TooLarge);
        }
        lease.try_resize(Charge {
            retained: measured,
            ..Charge::default()
        })?;
        row.ownership = RowOwnership {
            _lease: Some(Arc::new(lease)),
        };
        Ok(row)
    }

    /// Unmetered low-level constructor for caller-managed datasets. Native
    /// session entry points use `try_from_track` and propagate admission refusal.
    pub fn from_track(track: &TrackDto) -> Self {
        let mut fields = track.tags.clone();
        let path = std::path::Path::new(&track.path);
        let file = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        fields.insert("file".into(), file.clone());
        fields.insert("path".into(), track.path.clone());
        fields.insert(
            "folder".into(),
            path.parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        fields.insert("format".into(), track.format.clone());
        fields.insert(
            "cover".into(),
            if track.unreadable {
                "unknown"
            } else if track.has_cover {
                "embedded"
            } else {
                "missing"
            }
            .into(),
        );
        fields.insert("unreadable".into(), track.unreadable.to_string());
        if let Some(duration) = track.duration_secs {
            fields.insert("duration".into(), duration.to_string());
        }
        let position = fields
            .get("track")
            .or_else(|| fields.get("tracknumber"))
            .cloned()
            .unwrap_or_default();
        let position = if fields.contains_key("media") {
            static SIDE: std::sync::LazyLock<tagrex_core::mask::Mask> =
                std::sync::LazyLock::new(|| {
                    tagrex_core::mask::Mask::parse("%side%").expect("static side mask")
                });
            let mut tags = tagrex_core::model::TagMap::new();
            for (key, field) in [
                ("media", tagrex_core::model::TagField::MediaType),
                ("disc", tagrex_core::model::TagField::DiscNumber),
            ] {
                if let Some(value) = fields.get(key) {
                    tags.insert(field, value.clone());
                }
            }
            format!("{}{position}", SIDE.render(&tags).unwrap_or_default())
        } else {
            position
        };
        fields.insert("position".into(), position);
        fields.insert(
            "tagtypes".into(),
            track
                .tag_blocks
                .iter()
                .filter(|block| block.read_from)
                .chain(track.tag_blocks.iter().filter(|block| !block.read_from))
                .map(|block| block.label.as_str())
                .collect::<Vec<_>>()
                .join(" + "),
        );
        let mut search = file;
        for value in track.tags.values() {
            search.push(' ');
            search.push_str(value);
        }
        Self {
            ownership: RowOwnership::default(),
            path: track.path.clone(),
            values: Arc::new(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, Value::new(v)))
                    .collect(),
            ),
            technical: BTreeMap::new(),
            computed: Arc::default(),
            overlay_ownership: RowOwnership::default(),
            search: Arc::new(Value::new(search)),
            format: None,
            props: None,
            modified: None,
            source_fingerprint: None,
        }
    }

    pub(crate) fn set_source_metadata(
        &mut self,
        metadata: Option<&std::fs::Metadata>,
        policy_revision: u64,
    ) {
        #[cfg(not(windows))]
        {
            self.source_fingerprint =
                metadata.and_then(crate::read_cache::Fingerprint::from_metadata);
        }
        #[cfg(windows)]
        {
            // Metadata alone has no authoritative Windows file ID/change time.
            self.source_fingerprint = metadata
                .and_then(|_| crate::read_cache::Fingerprint::of(std::path::Path::new(&self.path)));
        }
        if tagrex_core::model::read_policy_revision() != policy_revision {
            self.source_fingerprint = None;
        }
    }

    pub fn set_technical(
        &mut self,
        size: Option<u64>,
        bitrate: Option<u32>,
        sample_rate: Option<u32>,
        channels: Option<u8>,
    ) {
        for (key, value) in [
            ("size", size.map(|v| v.to_string())),
            ("bitrate", bitrate.map(|v| v.to_string())),
            ("samplerate", sample_rate.map(|v| v.to_string())),
            ("channels", channels.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                self.technical.insert(key.into(), Value::new(value));
            }
        }
    }

    pub fn set_file_context(
        &mut self,
        format: tagrex_core::model::AudioFormat,
        props: tagrex_core::model::AudioProps,
        modified: Option<SystemTime>,
    ) {
        self.format = Some(format);
        self.props = Some(props);
        self.modified = modified;
    }

    pub fn value(&self, field: &str) -> &str {
        self.computed
            .get(canonical_field(field))
            .or_else(|| self.technical.get(canonical_field(field)))
            .or_else(|| self.values.get(canonical_field(field)))
            .map(|v| v.raw.as_str())
            .unwrap_or("")
    }
    fn folded(&self, field: &str) -> &str {
        self.computed
            .get(canonical_field(field))
            .or_else(|| self.technical.get(canonical_field(field)))
            .or_else(|| self.values.get(canonical_field(field)))
            .map(|v| v.folded.as_str())
            .unwrap_or("")
    }
}

fn computed_render_error(error: &tagrex_core::mask::BoundedRenderError) -> QueryError {
    invalid(&format!("Computed field could not render: {error}"))
}

fn canonical_field(field: &str) -> &str {
    match field {
        "filename" => "file",
        "artwork" => "cover",
        "length" => "duration",
        "album_artist" => "albumartist",
        "tracknumber" => "track",
        "discnumber" => "disc",
        "comments" => "comment",
        "date" => "year",
        "label" => "publisher",
        "sample_rate" => "samplerate",
        "initialkey" => "key",
        "catalog_number" => "catalognumber",
        other => other,
    }
}

#[derive(Debug, Default)]
struct Facet {
    values: BTreeMap<String, (String, Vec<usize>)>,
    credits: HashMap<String, Vec<String>>,
    working_bytes: usize,
}

impl Facet {
    fn retained_bytes(&self) -> usize {
        let values = self
            .values
            .iter()
            .fold(128usize, |sum, (folded, (raw, rows))| {
                sum.saturating_add(128)
                    .saturating_add(folded.capacity())
                    .saturating_add(raw.capacity())
                    .saturating_add(rows.capacity().saturating_mul(std::mem::size_of::<usize>()))
            });
        self.credits.iter().fold(values, |sum, (key, names)| {
            sum.saturating_add(96)
                .saturating_add(key.capacity())
                .saturating_add(
                    names
                        .capacity()
                        .saturating_mul(std::mem::size_of::<String>()),
                )
                .saturating_add(names.iter().map(|s| s.capacity()).sum::<usize>())
        })
    }
}

const MAX_FACET_WORK_BYTES: usize = 64 * 1024 * 1024;
fn facet_budget(bytes: &mut usize, additional: usize) -> Result<(), QueryError> {
    *bytes = bytes.saturating_add(additional);
    if *bytes > MAX_FACET_WORK_BYTES {
        return Err(invalid("Filter facets exceed the 64 MiB working budget."));
    }
    Ok(())
}
fn push_facet_value(
    values: &mut BTreeMap<String, (String, Vec<usize>)>,
    bytes: &mut usize,
    folded: &str,
    raw: &str,
    index: usize,
) -> Result<(), QueryError> {
    if !values.contains_key(folded) {
        facet_budget(
            bytes,
            folded.len().saturating_add(raw.len()).saturating_add(128),
        )?;
        values.insert(folded.into(), (raw.into(), Vec::new()));
    }
    let rows = &mut values.get_mut(folded).expect("inserted facet value").1;
    if rows.len() == rows.capacity() {
        let capacity = rows.capacity().saturating_mul(2).max(4);
        facet_budget(
            bytes,
            (capacity - rows.capacity()).saturating_mul(std::mem::size_of::<usize>()),
        )?;
        rows.try_reserve_exact(capacity - rows.len())
            .map_err(|_| invalid("Insufficient memory for filter facets."))?;
    }
    rows.push(index);
    Ok(())
}

type RegexCache = Arc<Mutex<crate::bounded_cache::Cache<(String, bool), Arc<Regex>>>>;

/// Retained facet memory. One suggestion over a large library needs several
/// high-cardinality facets at once; at 32 MiB the cache could not hold two
/// ~17 MiB facets from a 100k-row library, so each request rebuilt the ones it
/// had just evicted. This cap holds the measured working set of the default
/// eight suggestion fields at 100k rows with headroom for larger libraries.
/// It remains a bounded, evictable derived cache, not a process-wide memory cap
/// or a value authority; only the 100k-row workload was measured.
const FACET_CACHE_BYTES: usize = 96 * 1024 * 1024;
const FACET_CACHE_ENTRIES: usize = 32;

/// How many exact stored field names one snapshot remembers. Scoped search text
/// is resolved against this memo instead of rescanning every row per candidate
/// prefix; a full memo stops admitting names and scopes still resolve by scan.
const MAX_FIELD_KEY_MEMO: usize = 256;
/// Only real field names are worth remembering; a longer prefix is search text.
const MAX_FIELD_KEY_LEN: usize = 128;

/// Immutable rows shared with query workers. Caches contain only derived data.
#[derive(Debug)]
pub struct QueryIndex {
    pub id: u64,
    pub rows: Vec<Arc<QueryRow>>,
    facets: Mutex<crate::bounded_cache::Cache<String, Arc<Facet>>>,
    duplicates: Arc<Mutex<HashMap<String, DuplicateResults>>>,
    valid: Arc<AtomicBool>,
    custom: Mutex<Option<(String, Arc<QueryIndex>)>>,
    custom_key: String,
    regex_cache: RegexCache,
    /// Bounded "some row stores this exact field key" evidence, positive and
    /// negative. Derived from immutable rows, never a value authority.
    field_keys: Mutex<HashMap<String, bool>>,
    read_policy_revision: u64,
    read_policy_owner: Option<tagrex_core::model::ModelPolicy>,
    _overlay_ownership: RowOwnership,
    _snapshot_ownership: RowOwnership,
}

impl QueryIndex {
    /// Build an admitted native snapshot; a refusal drops every partial row.
    pub fn try_from_tracks(
        tracks: &[TrackDto],
    ) -> Result<Self, tagrex_core::resources::AdmissionError> {
        let rows = tracks
            .iter()
            .map(QueryRow::try_from_track)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from_rows(rows))
    }
    pub fn from_tracks(tracks: &[TrackDto]) -> Self {
        Self::from_rows(tracks.iter().map(QueryRow::from_track).collect())
    }
    pub fn from_rows(rows: Vec<QueryRow>) -> Self {
        Self::from_shared_rows(rows.into_iter().map(Arc::new).collect())
    }
    fn from_shared_rows(rows: Vec<Arc<QueryRow>>) -> Self {
        Self {
            id: SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed),
            rows,
            facets: Mutex::new(crate::bounded_cache::Cache::new(
                FACET_CACHE_ENTRIES,
                FACET_CACHE_BYTES,
            )),
            duplicates: Arc::default(),
            valid: Arc::new(AtomicBool::new(true)),
            custom: Mutex::default(),
            custom_key: String::new(),
            regex_cache: Arc::new(Mutex::new(crate::bounded_cache::Cache::new(
                16,
                32 * 1024 * 1024,
            ))),
            field_keys: Mutex::default(),
            read_policy_revision: tagrex_core::model::read_policy_revision(),
            read_policy_owner: None,
            _overlay_ownership: RowOwnership::default(),
            _snapshot_ownership: RowOwnership::default(),
        }
    }
    /// Create independent query/cancellation/cache state over admitted immutable
    /// saved rows. No saved tag/search strings are cloned or reinterpreted.
    pub fn fork_snapshot_rows(&self, selected: Option<&[usize]>) -> Result<Self, QueryError> {
        if !self.is_current() {
            return Err(QueryError::Cancelled);
        }
        if let Some(selected) = selected {
            if selected.len() > self.rows.len()
                || selected.windows(2).any(|pair| pair[0] >= pair[1])
                || selected.last().is_some_and(|last| *last >= self.rows.len())
            {
                return Err(invalid(
                    "Snapshot row selection must be ordered, unique and in range.",
                ));
            }
        }
        let count = selected.map_or(self.rows.len(), <[usize]>::len);
        let retained = count
            .checked_mul(std::mem::size_of::<Arc<QueryRow>>())
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>() + 512))
            .ok_or(tagrex_core::resources::AdmissionError::TooLarge)?;
        let lease = tagrex_core::resources::Budget::process().try_acquire(
            tagrex_core::resources::Charge {
                retained,
                ..Default::default()
            },
        )?;
        let mut rows = Vec::with_capacity(count);
        for position in 0..count {
            if position.is_multiple_of(128) && !self.is_current() {
                return Err(QueryError::Cancelled);
            }
            let index = selected.map_or(position, |indices| indices[position]);
            rows.push(self.rows[index].clone());
        }
        if !self.is_current() {
            return Err(QueryError::Cancelled);
        }
        let mut index =
            Self::from_shared_rows(rows).with_policy_from(self);
        index._snapshot_ownership = RowOwnership {
            _lease: Some(Arc::new(lease)),
        };
        Ok(index)
    }

    pub(crate) fn with_model_policy(mut self, owner: tagrex_core::model::ModelPolicy, revision: u64) -> Self {
        self.read_policy_revision = revision;
        self.read_policy_owner = Some(owner);
        self
    }
    fn with_policy_from(mut self, source: &Self) -> Self {
        self.read_policy_revision = source.read_policy_revision;
        self.read_policy_owner = source.read_policy_owner.clone();
        self
    }
    pub fn with_updates(&self, rows: Vec<QueryRow>) -> Self {
        self.invalidate();
        let mut updates: HashMap<_, _> = rows
            .into_iter()
            .map(|r| (r.path.clone(), Arc::new(r)))
            .collect();
        let rows = self
            .rows
            .iter()
            .map(|row| updates.remove(&row.path).unwrap_or_else(|| row.clone()))
            .collect();
        Self::from_shared_rows(rows).with_policy_from(self)
    }
    /// Read-only refreshes must not invalidate facets, ongoing queries or
    /// duplicate evidence unless the rows or their source identity changed.
    pub fn with_updates_if_changed(&self, rows: Vec<QueryRow>) -> Option<Self> {
        let updates: HashMap<_, _> = rows.iter().map(|row| (row.path.as_str(), row)).collect();
        let changed = !self.is_current()
            || self.rows.iter().any(|row| {
                updates
                    .get(row.path.as_str())
                    .is_some_and(|new| new.source_fingerprint.is_none() || row.as_ref() != *new)
            });
        changed.then(|| self.with_updates(rows))
    }
    pub fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
        self.duplicates
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
    pub fn is_current(&self) -> bool {
        self.valid.load(Ordering::Acquire) && self.read_policy_is_current()
    }
    pub(crate) fn read_policy_is_current(&self) -> bool {
        self.read_policy_revision == self.read_policy_owner.as_ref().map_or_else(tagrex_core::model::read_policy_revision, tagrex_core::model::ModelPolicy::revision)
            && self.read_policy_revision != u64::MAX
    }
    pub fn known_field(&self, field: &str) -> bool {
        FIELDS.contains(&field)
            || (field.starts_with("custom:") && field.len() > 7 && field.len() <= 256)
            || self.stores_field_key(field)
    }

    /// Whether any row stores this exact field key. Remembered per snapshot:
    /// query and suggestion work compile the same model several times, and a
    /// colon-heavy search text can test many candidate prefixes.
    fn stores_field_key(&self, field: &str) -> bool {
        if let Some(known) = self
            .field_keys
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(field)
        {
            return *known;
        }
        let known = self
            .rows
            .iter()
            .any(|row| row.computed.contains_key(field) || row.values.contains_key(field));
        let mut memo = self.field_keys.lock().unwrap_or_else(|p| p.into_inner());
        if memo.len() < MAX_FIELD_KEY_MEMO && field.len() <= MAX_FIELD_KEY_LEN {
            memo.insert(field.into(), known);
        }
        known
    }

    fn text_scope(&self, text: &str) -> Option<(String, usize)> {
        text.match_indices(':')
            .rev()
            .find_map(|(offset, _)| {
                let field = &text[..offset];
                // An exactly spelled standard name needs no row evidence.
                if FIELDS.contains(&field) {
                    return Some((field.to_string(), offset + 1));
                }
                // A stored key still outranks a case-folded standard alias, so a
                // computed field named "DATE" keeps resolving to itself. Absent
                // names are remembered, so this is one pass per name per
                // snapshot rather than one pass per candidate per request.
                if self.stores_field_key(field) {
                    return Some((field.to_string(), offset + 1));
                }
                let folded = field.to_lowercase();
                if FIELDS.contains(&folded.as_str()) {
                    return Some((folded, offset + 1));
                }
                None
            })
            .or_else(|| {
                let rest = text.strip_prefix("custom:")?;
                let offset = rest.find(':')? + 7;
                Some((text[..offset].to_string(), offset + 1))
            })
    }

    fn custom_index(
        &self,
        definitions: &[CustomField],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Arc<QueryIndex>>, QueryError> {
        use tagrex_core::resources::{AdmissionError, Budget, Charge};
        const RETAINED: usize = 64 * 1024 * 1024;
        const WORK: usize = 128 * 1024 * 1024;
        const RENDER_WORK: usize = 64 * 1024 * 1024;
        const INPUT: usize = 8 * 1024 * 1024;
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        if definitions.is_empty() {
            return Ok(None);
        }
        if definitions.len() > 32 {
            return Err(invalid("Use at most 32 computed filter fields."));
        }
        // Validate borrowed strings before serializing a cache key or cloning
        // definitions. Duplicate checks need no allocations for 32 entries.
        for (i, definition) in definitions.iter().enumerate() {
            if definition.key.is_empty()
                || definition.key.len() > 128
                || definition.mask.len() > 2048
                || FIELDS.contains(&definition.key.as_str())
                || definitions[..i]
                    .iter()
                    .any(|previous| previous.key == definition.key)
            {
                return Err(invalid(
                    "Computed fields need unique, non-reserved keys and bounded masks.",
                ));
            }
        }
        let budget = Budget::process();
        // 32 bounded definitions, including JSON's worst-case six-byte escapes
        // and Vec growth, fit this small warm-lookup allowance.
        let _key_work = budget.try_acquire(Charge {
            transient: 1024 * 1024,
            ..Charge::default()
        })?;
        let key =
            serde_json::to_string(definitions).map_err(|error| invalid(&error.to_string()))?;
        if key == self.custom_key {
            return Ok(None);
        }
        let cached = self
            .custom
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .filter(|(cached_key, _)| cached_key == &key)
            .map(|(_, index)| index.clone());
        if let Some(index) = cached {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            return Ok(Some(index));
        }
        // Reserve the bounded shared AST before parsing, then grow retained
        // and per-row work admission before each clone/render. Tiny overlays do
        // not occupy a whole 128 MiB work slot merely because that is the cap.
        let compile_work = definitions.iter().fold(64 * 1024usize, |sum, definition| {
            sum.saturating_add(definition.mask.len().saturating_mul(512))
                .saturating_add(8192)
        });
        let mut derived_bytes = key.capacity().saturating_mul(2).saturating_add(8192);
        let mut lease = budget.try_acquire(Charge {
            retained: derived_bytes,
            transient: compile_work,
            jobs: 1,
        })?;
        for definition in definitions {
            for (row, source) in self.rows.iter().enumerate() {
                if row.is_multiple_of(128) && cancelled() {
                    return Err(QueryError::Cancelled);
                }
                if source.values.contains_key(&definition.key) {
                    return Err(invalid(
                        "Computed fields need unique, non-reserved keys and bounded masks.",
                    ));
                }
            }
        }
        let mut masks = Vec::with_capacity(definitions.len());
        for definition in definitions {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            masks.push(
                tagrex_core::mask::Mask::parse_for_render(&definition.mask)
                    .map_err(|error| invalid(&format!("Invalid computed filter mask: {error}")))?,
            );
        }
        let render_fields = masks
            .iter()
            .flat_map(|mask| mask.render_fields())
            .collect::<std::collections::BTreeSet<_>>();
        if render_fields.len() > 512 {
            return Err(AdmissionError::TooLarge.into());
        }
        let mut rows = Vec::new();
        for source in &self.rows {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            let mut selected = Vec::with_capacity(render_fields.len());
            let mut input_bytes = 8192usize;
            for field in &render_fields {
                if cancelled() {
                    return Err(QueryError::Cancelled);
                }
                let key = field.to_storage_key();
                let mut value = source.values.get(&key);
                if value.is_none() {
                    if let tagrex_core::model::TagField::Custom(name) = field {
                        for (number, (stored, candidate)) in source.values.iter().enumerate() {
                            if number.is_multiple_of(128) && cancelled() {
                                return Err(QueryError::Cancelled);
                            }
                            // Compare borrowed names first. Do not allocate a
                            // TagField for every unrelated potentially huge key.
                            if stored
                                .strip_prefix("custom:")
                                .unwrap_or(stored)
                                .eq_ignore_ascii_case(name)
                                && tagrex_core::model::TagField::from_storage_key(stored)
                                    .same_name(field)
                            {
                                value = Some(candidate);
                                break;
                            }
                        }
                    }
                }
                if let Some(value) = value {
                    if value.raw.len() > 1024 * 1024 {
                        return Err(AdmissionError::TooLarge.into());
                    }
                    input_bytes = input_bytes
                        .saturating_add(key.len())
                        .saturating_add(value.raw.len())
                        .saturating_add(512);
                    if input_bytes > INPUT {
                        return Err(AdmissionError::TooLarge.into());
                    }
                    selected.push((field, value));
                }
            }
            // Only clone after the aggregate has been checked. The source map
            // itself and each individual field may be much larger than this job.
            let input_work = compile_work.saturating_add(input_bytes);
            lease.try_resize(Charge {
                retained: derived_bytes,
                transient: input_work,
                jobs: 1,
            })?;
            let mut tags = tagrex_core::export::lenient_tags(&tagrex_core::model::TagMap::new());
            for (field, value) in selected {
                tags.insert(field.clone(), value.raw.clone());
            }
            let context = tagrex_core::mask::FileContext {
                path: Some(std::path::Path::new(&source.path)),
                format: source.format,
                props: source.props,
                size_bytes: source.value("size").parse().ok(),
                modified: source.modified,
            };
            let mut render_work = 0usize;
            for mask in &masks {
                if cancelled() {
                    return Err(QueryError::Cancelled);
                }
                let bytes = mask
                    .render_work_bytes(&tags, &context)
                    .ok_or(AdmissionError::TooLarge)?;
                if bytes > RENDER_WORK {
                    return Err(AdmissionError::TooLarge.into());
                }
                render_work = render_work.max(bytes);
            }
            // Core's AST allowance includes rendering scratch/result growth;
            // the additional 8 MiB dominates a <=1 MiB result's case-folded
            // copy and allocation-growth overlap before it becomes retention.
            let row_work = input_work
                .saturating_add(render_work)
                .saturating_add(8 * 1024 * 1024);
            if row_work > WORK {
                return Err(AdmissionError::TooLarge.into());
            }
            let base = std::mem::size_of::<QueryRow>()
                .saturating_mul(2)
                .saturating_add(256)
                .saturating_add(source.path.len());
            derived_bytes = derived_bytes.saturating_add(base);
            // Inherited overlays are uncommon but cannot be copied outside the
            // new owner's allowance when a derived index is queried directly.
            for (key, value) in source.computed.iter().chain(source.technical.iter()) {
                derived_bytes = derived_bytes
                    .saturating_add(512)
                    .saturating_add(key.capacity())
                    .saturating_add(value.raw.capacity())
                    .saturating_add(value.folded.capacity());
            }
            if derived_bytes > RETAINED {
                return Err(AdmissionError::TooLarge.into());
            }
            lease.try_resize(Charge {
                retained: derived_bytes,
                transient: row_work,
                jobs: 1,
            })?;
            let mut row = (**source).clone();
            let mut computed = (*source.computed).clone();
            for (definition, mask) in definitions.iter().zip(&masks) {
                if cancelled() {
                    return Err(QueryError::Cancelled);
                }
                let value = match mask.render_with_budget(&tags, &context, RENDER_WORK) {
                    Ok(value) => value,
                    Err(tagrex_core::mask::BoundedRenderError::TooLarge) => {
                        return Err(AdmissionError::TooLarge.into())
                    }
                    Err(tagrex_core::mask::BoundedRenderError::Mask(
                        tagrex_core::mask::MaskError::MissingTag(_),
                    )) => String::new(),
                    Err(error) => return Err(computed_render_error(&error)),
                };
                let value = Value::new(value);
                derived_bytes = derived_bytes
                    .saturating_add(512)
                    .saturating_add(definition.key.len())
                    .saturating_add(value.raw.capacity())
                    .saturating_add(value.folded.capacity());
                if derived_bytes > RETAINED {
                    return Err(AdmissionError::TooLarge.into());
                }
                lease.try_resize(Charge {
                    retained: derived_bytes,
                    transient: row_work,
                    jobs: 1,
                })?;
                computed.insert(definition.key.clone(), value);
            }
            row.computed = Arc::new(computed);
            rows.push(Arc::new(row));
            drop(tags);
            lease.try_resize(Charge {
                retained: derived_bytes,
                transient: compile_work,
                jobs: 1,
            })?;
        }
        drop(masks);
        drop(render_fields);
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        lease.try_resize(Charge {
            retained: derived_bytes,
            ..Charge::default()
        })?;
        let owner = RowOwnership {
            _lease: Some(Arc::new(lease)),
        };
        for row in &mut rows {
            // Every row was allocated above and has not escaped yet.
            Arc::get_mut(row)
                .expect("unpublished computed row is uniquely owned")
                .overlay_ownership = owner.clone();
        }
        let index = Arc::new(Self {
            id: self.id,
            rows,
            facets: Mutex::new(crate::bounded_cache::Cache::new(
                FACET_CACHE_ENTRIES,
                FACET_CACHE_BYTES,
            )),
            duplicates: self.duplicates.clone(),
            valid: self.valid.clone(),
            custom: Mutex::default(),
            custom_key: key.clone(),
            regex_cache: self.regex_cache.clone(),
            field_keys: Mutex::default(),
            read_policy_revision: self.read_policy_revision,
            read_policy_owner: self.read_policy_owner.clone(),
            _overlay_ownership: owner,
            _snapshot_ownership: RowOwnership::default(),
        });
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        // Preserve the old hot overlay on every refusal/error/cancellation.
        let previous = {
            self.custom
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .replace((key, index.clone()))
        };
        // A large old overlay can be the final owner of many buffers. Release
        // it after unlocking so another warm lookup never waits on that drop.
        drop(previous);
        Ok(Some(index))
    }
    pub fn duplicate_results(&self, kind: &str) -> DuplicateResults {
        self.duplicates
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(kind)
            .cloned()
            .unwrap_or_else(|| DuplicateResults {
                status: "not_scanned".into(),
                ..Default::default()
            })
    }

    fn regex(&self, pattern: &str, case: bool) -> Result<Arc<Regex>, QueryError> {
        let key = (pattern.to_string(), case);
        if let Some(regex) = self
            .regex_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(regex);
        }
        let regex = Arc::new(
            RegexBuilder::new(pattern)
                .case_insensitive(!case)
                .size_limit(2 * 1024 * 1024)
                .dfa_size_limit(2 * 1024 * 1024)
                .build()
                .map_err(|e| {
                    QueryError::Invalid(format!("Invalid or unsupported regular expression: {e}"))
                })?,
        );
        let mut cache = self.regex_cache.lock().unwrap_or_else(|p| p.into_inner());
        // Both compiled and lazy DFA limits contribute; this is conservative
        // admission accounting, not an allocator/RSS measurement.
        let bytes = 4 * 1024 * 1024 + key.0.capacity() + 128;
        cache.insert(key, regex.clone(), bytes);
        Ok(regex)
    }
    pub fn set_duplicate_results(&self, kind: &str, result: DuplicateResults) {
        let mut results = self.duplicates.lock().unwrap_or_else(|p| p.into_inner());
        if self.is_current() {
            results.insert(kind.into(), result);
        }
    }
    fn facet(&self, field: &str, cancelled: &impl Fn() -> bool) -> Result<Arc<Facet>, QueryError> {
        let field = canonical_field(field);
        if let Some(value) = self
            .facets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(field)
            .cloned()
        {
            return Ok(value);
        }
        let mut facet = Facet::default();
        for (i, row) in self.rows.iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return Err(QueryError::Cancelled);
            }
            let raw = row.value(field);
            if raw.trim().is_empty() {
                continue;
            }
            push_facet_value(
                &mut facet.values,
                &mut facet.working_bytes,
                row.folded(field),
                raw,
                i,
            )?;
        }
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let facet = Arc::new(facet);
        // Bound cached fields, rather than retaining an index for every custom tag.
        let mut cache = self.facets.lock().unwrap_or_else(|p| p.into_inner());
        let bytes = facet.retained_bytes().saturating_add(field.len() * 2 + 256);
        cache.insert(field.into(), facet.clone(), bytes);
        Ok(facet)
    }

    fn member_facet(
        &self,
        field: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Arc<Facet>, QueryError> {
        let field = canonical_field(field);
        let key = format!("@members:{field}");
        if let Some(facet) = self
            .facets
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(facet);
        }
        // Admission precedes the member parser's normalization scratch and
        // known-credit set. Count unique borrowed inputs before making strings.
        if field != "genre" {
            let mut known_inputs = std::collections::HashSet::new();
            let mut known_bytes = 0usize;
            for (i, row) in self.rows.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return Err(QueryError::Cancelled);
                }
                for raw in [row.value("artist"), row.value("albumartist")] {
                    if !raw.is_empty() && !known_inputs.contains(raw) {
                        facet_budget(
                            &mut known_bytes,
                            raw.len().saturating_mul(16).saturating_add(256),
                        )?;
                        known_inputs.insert(raw);
                    }
                }
            }
            drop(known_inputs);
        }
        let known = if field == "genre" {
            std::collections::HashSet::new()
        } else {
            crate::query_members::known_artists_with_cancel(
                self.rows
                    .iter()
                    .flat_map(|row| [row.value("artist"), row.value("albumartist")]),
                cancelled,
            )
            .ok_or(QueryError::Cancelled)?
        };
        let mut facet = Facet::default();
        for (i, row) in self.rows.iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return Err(QueryError::Cancelled);
            }
            let raw = row.value(field);
            if raw.trim().is_empty() {
                continue;
            }
            if !facet.credits.contains_key(raw) {
                // Includes normalization token scratch before parsing any row.
                let mut scratch = facet.working_bytes;
                facet_budget(
                    &mut scratch,
                    raw.len().saturating_mul(16).saturating_add(4096),
                )?;
                let names = crate::query_members::members(raw, field, &known);
                let bytes = raw
                    .len()
                    .saturating_add(128)
                    .saturating_add(
                        names
                            .capacity()
                            .saturating_mul(std::mem::size_of::<String>()),
                    )
                    .saturating_add(names.iter().map(|n| n.capacity()).sum::<usize>());
                facet_budget(&mut facet.working_bytes, bytes)?;
                facet.credits.insert(raw.into(), names);
            }
            for name in &facet.credits[raw] {
                // The folded temporary is bounded by the admitted credit.
                push_facet_value(
                    &mut facet.values,
                    &mut facet.working_bytes,
                    &name.to_lowercase(),
                    name,
                    i,
                )?;
            }
        }
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let facet = Arc::new(facet);
        let mut cache = self.facets.lock().unwrap_or_else(|p| p.into_inner());
        let bytes = facet
            .retained_bytes()
            .saturating_add(key.capacity() * 2 + 256);
        cache.insert(key, facet.clone(), bytes);
        Ok(facet)
    }

    pub fn query(
        &self,
        request: &QueryRequest,
        cancelled: impl Fn() -> bool,
    ) -> Result<QueryResult, QueryError> {
        let cancelled = || cancelled() || !self.is_current();
        let start = Instant::now();
        let custom = self.custom_index(&request.query.custom_fields, &cancelled)?;
        let mut result = custom
            .as_deref()
            .unwrap_or(self)
            .query_prepared(request, &cancelled)?;
        result.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(result)
    }

    fn query_prepared(
        &self,
        request: &QueryRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<QueryResult, QueryError> {
        let start = Instant::now();
        let compiled = CompiledQuery::new(&request.query, self, cancelled)?;
        let mut indices = compiled.matches(self, &cancelled)?;
        let mut duplicate_status = "not_scanned".to_string();
        let mut groups = Vec::new();
        let mut groups_revision = 0;
        let mut groups_count = 0;
        let mut skipped = 0;
        let mut errors = 0;
        if request.query.mode == "duplicates" {
            let result = self.duplicate_results(&request.query.duplicate_kind);
            groups_revision = result.groups_revision;
            duplicate_status = result.status;
            groups_count = if ["running", "complete"].contains(&duplicate_status.as_str()) {
                result.groups.len()
            } else {
                0
            };
            skipped = result.skipped;
            errors = result.errors;
            let matched: std::collections::HashSet<_> = indices.into_iter().collect();
            let mut companions = std::collections::BTreeSet::new();
            for group in result
                .groups
                .into_iter()
                .filter(|_| ["running", "complete"].contains(&duplicate_status.as_str()))
            {
                if group.indices.iter().any(|i| matched.contains(i)) {
                    companions.extend(group.indices.iter().copied());
                    groups.push(group);
                }
            }
            indices = companions.into_iter().collect();
        }
        if let Some(sort) = &request.query.sort {
            let mut keyed: Vec<_> = indices
                .into_iter()
                .map(|i| {
                    let value = self.rows[i].value(&sort.field);
                    (
                        i,
                        value.parse::<f64>().ok().filter(|n| n.is_finite()),
                        self.rows[i].folded(&sort.field),
                    )
                })
                .collect();
            keyed.sort_by(|a, b| {
                let order = match (a.1, b.1) {
                    (Some(a), Some(b)) => a.total_cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.2.cmp(b.2),
                };
                if sort.direction == "desc" {
                    order.reverse()
                } else {
                    order
                }
            });
            indices = keyed.into_iter().map(|(i, _, _)| i).collect();
        }
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        Ok(QueryResult {
            request_id: request.request_id,
            snapshot_id: self.id,
            total: self.rows.len(),
            matched: indices.len(),
            indices,
            paths: (request.snapshot_id != Some(self.id))
                .then(|| self.rows.iter().map(|r| r.path.clone()).collect()),
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            cancelled: false,
            duplicate_status,
            groups,
            groups_revision,
            groups_count,
            skipped,
            errors,
        })
    }

    pub fn suggest(
        &self,
        request: &SuggestRequest,
        cancelled: impl Fn() -> bool,
    ) -> Result<SuggestResult, QueryError> {
        let cancelled = || cancelled() || !self.is_current();
        let start = Instant::now();
        let custom = self.custom_index(&request.query.custom_fields, &cancelled)?;
        let mut result = custom
            .as_deref()
            .unwrap_or(self)
            .suggest_prepared(request, &cancelled)?;
        result.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        Ok(result)
    }

    fn suggest_prepared(
        &self,
        request: &SuggestRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<SuggestResult, QueryError> {
        let start = Instant::now();
        if request.text.len() > 512 || request.limit == 0 || request.limit > 50 {
            return Err(invalid("Suggestion text/limit is out of range."));
        }
        if !["values", "fields"].contains(&request.strategy.as_str()) {
            return Err(invalid("Unsupported suggestion strategy."));
        }
        let field_strategy = request.strategy == "fields";
        let scope = field_strategy
            .then(|| self.text_scope(request.text.trim()))
            .flatten();
        let requested_field = request
            .field
            .as_deref()
            .or_else(|| scope.as_ref().map(|(field, _)| field.as_str()));
        let text = if field_strategy {
            let text = request.text.trim();
            scope
                .as_ref()
                .filter(|(field, _)| {
                    requested_field
                        .is_some_and(|own| canonical_field(own) == canonical_field(field))
                })
                .map(|(_, offset)| text[*offset..].trim())
                .unwrap_or(text)
        } else {
            &request.text
        };
        let fields: Vec<&str> = if let Some(field) = requested_field {
            if !self.known_field(field) {
                return Err(invalid("Unsupported suggestion field."));
            }
            vec![field]
        } else {
            vec![
                "genre",
                "artist",
                "albumartist",
                "album",
                "title",
                "year",
                "label",
                "format",
            ]
        };
        let needle = text.to_lowercase();
        let duplicate_members = if request.query.mode == "duplicates" {
            let result = self.duplicate_results(&request.query.duplicate_kind);
            let mut members = vec![false; self.rows.len()];
            if ["running", "complete"].contains(&result.status.as_str()) {
                for group in result.groups {
                    for i in group.indices {
                        if i < members.len() {
                            members[i] = true;
                        }
                    }
                }
            }
            Some(members)
        } else {
            None
        };
        let mut ranked: std::collections::BinaryHeap<(
            u8,
            std::cmp::Reverse<usize>,
            String,
            String,
        )> = std::collections::BinaryHeap::new();
        let mut candidates = 0usize;
        let mut field_ranked = Vec::new();
        let mut contexts: HashMap<String, Arc<Vec<bool>>> = HashMap::new();
        for field in fields {
            // Disjunctive faceting: adding another value to a simple top-level
            // field chip must offer alternatives outside its current value.
            // Nested groups and negations retain their explicit semantics.
            let mut context = request.query.clone();
            if let QueryNode::Group { mode, children, .. } = &mut context.root {
                if mode == "all" {
                    children.retain(|node| !matches!(node,QueryNode::Condition{field:own,operator,negate:false,..} if canonical_field(own)==canonical_field(field)&&(["equals","member"].contains(&operator.as_str())||(field_strategy&&operator=="contains"))));
                }
            }
            let key =
                serde_json::to_string(&context).map_err(|e| QueryError::Invalid(e.to_string()))?;
            let membership = if let Some(membership) = contexts.get(&key) {
                membership.clone()
            } else {
                let matches =
                    CompiledQuery::new(&context, self, cancelled)?.matches(self, &cancelled)?;
                let mut membership = vec![false; self.rows.len()];
                for i in matches {
                    membership[i] = duplicate_members.as_ref().is_none_or(|members| members[i]);
                }
                let membership = Arc::new(membership);
                contexts.insert(key, membership.clone());
                membership
            };
            if field_strategy {
                if !needle.is_empty() {
                    if let Some((suggestion, exact, starts)) = self.field_suggestion(
                        field,
                        text,
                        &needle,
                        &membership,
                        &request.query,
                        cancelled,
                    )? {
                        field_ranked.push((
                            !exact,
                            std::cmp::Reverse(suggestion.count),
                            !starts,
                            suggestion,
                        ));
                    }
                }
                continue;
            }
            let facet = if crate::query_members::is_member_field(field) {
                self.member_facet(field, cancelled)?
            } else {
                self.facet(field, cancelled)?
            };
            for (folded, (value, rows)) in &facet.values {
                if cancelled() {
                    return Err(QueryError::Cancelled);
                }
                if !folded.contains(&needle) {
                    continue;
                }
                let count = rows.iter().filter(|i| membership[**i]).count();
                if count > 0 {
                    candidates += 1;
                    let score = if folded == &needle {
                        0
                    } else if folded.starts_with(&needle) {
                        1
                    } else {
                        2
                    };
                    let wanted = (score, std::cmp::Reverse(count), field, value.as_str());
                    let retain = ranked.len() < request.limit
                        || ranked.peek().is_some_and(|worst| {
                            wanted < (worst.0, worst.1, worst.2.as_str(), worst.3.as_str())
                        });
                    if retain {
                        ranked.push((score, std::cmp::Reverse(count), field.into(), value.clone()));
                        if ranked.len() > request.limit {
                            ranked.pop();
                        }
                    }
                }
            }
        }
        if field_strategy {
            field_ranked.sort_by(|a, b| {
                (&a.0, &a.1, &a.2, &a.3.field).cmp(&(&b.0, &b.1, &b.2, &b.3.field))
            });
            let truncated = field_ranked.len() > request.limit;
            return Ok(SuggestResult {
                request_id: request.request_id,
                snapshot_id: self.id,
                suggestions: field_ranked
                    .into_iter()
                    .take(request.limit)
                    .map(|(_, _, _, value)| value)
                    .collect(),
                truncated,
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            });
        }
        let suggestions = ranked
            .into_sorted_vec()
            .into_iter()
            .map(|(_, std::cmp::Reverse(count), field, value)| Suggestion {
                operator: if crate::query_members::is_member_field(&field) {
                    "member"
                } else {
                    "equals"
                }
                .into(),
                field,
                value,
                count,
            })
            .collect();
        let truncated = candidates > request.limit;
        Ok(SuggestResult {
            request_id: request.request_id,
            snapshot_id: self.id,
            suggestions,
            truncated,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }
    fn field_suggestion(
        &self,
        field: &str,
        text: &str,
        needle: &str,
        membership: &[bool],
        model: &QueryModel,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<(Suggestion, bool, bool)>, QueryError> {
        let canonical = canonical_field(field);
        let category = ["genre", "year", "format", "cover"].contains(&canonical);
        let facet = if canonical == "genre" {
            self.member_facet(field, cancelled)?
        } else {
            self.facet(field, cancelled)?
        };
        let cover_value = (canonical == "cover")
            .then(|| {
                model
                    .cover_labels
                    .iter()
                    .find(|(_, label)| label.to_lowercase() == needle)
            })
            .flatten()
            .map(|(value, _)| value.as_str());
        let exact_value = category
            .then(|| facet.values.get(cover_value.unwrap_or(needle)))
            .flatten();
        let mut count = 0;
        let mut starts = false;
        if let Some((_, rows)) = exact_value {
            count = count_members(rows, membership, cancelled)?;
            starts = true;
        } else {
            // Raw field postings are disjoint: a collaboration containing the
            // term twice still contributes one file to a contains suggestion.
            let raw = if canonical == "genre" {
                self.facet(field, cancelled)?
            } else {
                facet.clone()
            };
            for (folded, (_, rows)) in &raw.values {
                if cancelled() {
                    return Err(QueryError::Cancelled);
                }
                if folded.contains(needle) {
                    let matching = count_members(rows, membership, cancelled)?;
                    count += matching;
                    starts |= matching > 0 && folded.starts_with(needle);
                }
            }
        }
        if count == 0 {
            return Ok(None);
        }
        let value = exact_value
            .map(|(value, _)| value.clone())
            .unwrap_or_else(|| text.into());
        let operator = if exact_value.is_some() {
            if canonical == "genre" {
                "member"
            } else {
                "equals"
            }
        } else {
            "contains"
        };
        if model.case_sensitive {
            // Folded postings discover identities independently of case. Once
            // the emitted rule is known, count it with the actual matcher so
            // its displayed scope agrees with committing that rule.
            let node = QueryNode::Condition {
                id: "suggestion".into(),
                field: field.into(),
                operator: operator.into(),
                values: vec![value.clone()],
                negate: false,
            };
            let condition = CompiledNode::new(
                &node,
                self,
                true,
                0,
                &mut 0,
                &mut std::collections::HashSet::new(),
                cancelled,
            )?;
            count = 0;
            for (i, row) in self.rows.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return Err(QueryError::Cancelled);
                }
                count += usize::from(membership[i] && condition.matches(row));
            }
            if count == 0 {
                return Ok(None);
            }
        }
        let artist_identity = if ["artist", "albumartist"].contains(&canonical) {
            self.member_facet(field, cancelled)?
                .values
                .contains_key(needle)
        } else {
            false
        };
        Ok(Some((
            Suggestion {
                field: field.into(),
                value,
                count,
                operator: operator.into(),
            },
            exact_value.is_some() || artist_identity,
            starts,
        )))
    }
}

fn count_members(
    rows: &[usize],
    membership: &[bool],
    cancelled: &impl Fn() -> bool,
) -> Result<usize, QueryError> {
    let mut count = 0;
    for (i, row) in rows.iter().enumerate() {
        if i % 256 == 0 && cancelled() {
            return Err(QueryError::Cancelled);
        }
        count += usize::from(membership[*row]);
    }
    Ok(count)
}

fn invalid(message: &str) -> QueryError {
    QueryError::Invalid(message.into())
}

struct CompiledQuery {
    root: CompiledNode,
    text: Option<TextMatch>,
    scope: Option<String>,
    case: bool,
    cover_labels: BTreeMap<String, String>,
}
enum TextMatch {
    Literal(String),
    Regex(Arc<Regex>),
}
enum CompiledNode {
    Group {
        all: bool,
        children: Vec<CompiledNode>,
    },
    Condition {
        field: String,
        operator: String,
        values: Vec<String>,
        numbers: Vec<f64>,
        negate: bool,
        case: bool,
        members: Option<Arc<Facet>>,
    },
}

impl CompiledQuery {
    fn new(
        model: &QueryModel,
        index: &QueryIndex,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, QueryError> {
        if model.version != 2 || model.text.len() > 2048 {
            return Err(invalid(
                "Unsupported query version or overlong search text.",
            ));
        }
        if model.cover_labels.len() > 3
            || model.cover_labels.iter().any(|(k, v)| {
                !["embedded", "missing", "unknown"].contains(&k.as_str()) || v.len() > 512
            })
        {
            return Err(invalid("Invalid artwork status labels."));
        }
        if !["search", "duplicates"].contains(&model.mode.as_str())
            || !["exact", "same_audio", "likely"].contains(&model.duplicate_kind.as_str())
        {
            return Err(invalid("Unsupported search mode or duplicate criterion."));
        }
        if let Some(sort) = &model.sort {
            if !index.known_field(&sort.field)
                || !["asc", "desc"].contains(&sort.direction.as_str())
            {
                return Err(invalid("Unsupported sort field/direction."));
            }
        }
        let mut count = 0;
        let mut ids = std::collections::HashSet::new();
        let root = CompiledNode::new(
            &model.root,
            index,
            model.case_sensitive,
            0,
            &mut count,
            &mut ids,
            cancelled,
        )?;
        let scope = index.text_scope(&model.text);
        let (scope, raw) = scope
            .map(|(field, start)| {
                (
                    Some(canonical_field(&field).to_string()),
                    &model.text[start..],
                )
            })
            .unwrap_or((None, model.text.as_str()));
        let text = if raw.is_empty() {
            None
        } else if model.regex {
            Some(TextMatch::Regex(index.regex(raw, model.case_sensitive)?))
        } else {
            Some(TextMatch::Literal(if model.case_sensitive {
                raw.into()
            } else {
                raw.to_lowercase()
            }))
        };
        Ok(Self {
            root,
            text,
            scope,
            case: model.case_sensitive,
            cover_labels: model.cover_labels.clone(),
        })
    }
    fn matches(
        &self,
        index: &QueryIndex,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<usize>, QueryError> {
        let unrestricted =
            matches!(&self.root,CompiledNode::Group{all:true,children} if children.is_empty());
        let root = if unrestricted {
            None
        } else {
            self.root.candidates(index, cancelled)?
        };
        let root_complete = unrestricted || root.is_some();
        let scoped = if !self.case && self.scope.is_some() && self.text.is_some() {
            let field = self.scope.as_deref().expect("checked scope");
            let facet = index.facet(field, cancelled)?;
            Some(facet_matches(
                &facet,
                |folded, raw| self.text_matches(raw, folded),
                cancelled,
            )?)
        } else {
            None
        };
        let text_complete = self.text.is_none() || scoped.is_some();
        let candidates = match (root, scoped) {
            (Some(a), Some(b)) => Some(intersection(&a, &b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        if root_complete && text_complete {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            return Ok(candidates.unwrap_or_else(|| (0..index.rows.len()).collect()));
        }
        let matches = |i: usize| {
            let row = &index.rows[i];
            if !root_complete && !self.root.matches(row) {
                return false;
            }
            if text_complete {
                return true;
            }
            let (raw, folded) = if let Some(field) = &self.scope {
                (row.value(field), row.folded(field))
            } else {
                (row.search.raw.as_str(), row.search.folded.as_str())
            };
            self.text_matches(raw, folded)
        };
        let mut found = Vec::new();
        if let Some(candidates) = candidates {
            for (n, i) in candidates.into_iter().enumerate() {
                if n % 256 == 0 && cancelled() {
                    return Err(QueryError::Cancelled);
                }
                if matches(i) {
                    found.push(i);
                }
            }
        } else {
            for i in 0..index.rows.len() {
                if i % 256 == 0 && cancelled() {
                    return Err(QueryError::Cancelled);
                }
                if matches(i) {
                    found.push(i);
                }
            }
        }
        Ok(found)
    }

    fn text_matches(&self, raw: &str, folded: &str) -> bool {
        match &self.text {
            None => true,
            Some(TextMatch::Literal(needle)) => {
                let value = if self.case { raw } else { folded };
                if self.scope.as_deref() == Some("cover") {
                    if ["embedded", "missing", "unknown"].contains(&needle.as_str()) {
                        value == needle
                    } else if needle == "no cover" {
                        value == "missing"
                    } else {
                        let label = self
                            .cover_labels
                            .get(raw)
                            .map(String::as_str)
                            .unwrap_or(value);
                        if self.case {
                            label.contains(needle)
                        } else {
                            label.to_lowercase().contains(needle)
                        }
                    }
                } else {
                    value.contains(needle)
                }
            }
            Some(TextMatch::Regex(regex)) => {
                regex.is_match(raw)
                    || (self.scope.as_deref() == Some("cover")
                        && self
                            .cover_labels
                            .get(raw)
                            .is_some_and(|label| regex.is_match(label)))
            }
        }
    }
}

impl CompiledNode {
    fn new(
        node: &QueryNode,
        index: &QueryIndex,
        case: bool,
        depth: usize,
        count: &mut usize,
        ids: &mut std::collections::HashSet<String>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, QueryError> {
        *count += 1;
        if *count > MAX_NODES || depth > MAX_DEPTH {
            return Err(invalid(
                "Use at most 128 conditions and eight group levels.",
            ));
        }
        let id = match node {
            QueryNode::Group { id, .. } | QueryNode::Condition { id, .. } => id,
        };
        if id.is_empty() || id.len() > 100 || !ids.insert(id.clone()) {
            return Err(invalid("Query nodes need unique nonempty IDs."));
        }
        match node {
            QueryNode::Group { mode, children, .. } => {
                if !["all", "any"].contains(&mode.as_str()) {
                    return Err(invalid("Groups must match all or any conditions."));
                }
                Ok(Self::Group {
                    all: mode == "all",
                    children: children
                        .iter()
                        .map(|n| Self::new(n, index, case, depth + 1, count, ids, cancelled))
                        .collect::<Result<_, _>>()?,
                })
            }
            QueryNode::Condition {
                field,
                operator,
                values,
                negate,
                ..
            } => {
                if !index.known_field(field) || field.len() > 128 {
                    return Err(QueryError::Invalid(format!(
                        "Unsupported filter field: {field}"
                    )));
                }
                if ![
                    "contains", "equals", "starts", "empty", "present", "gt", "gte", "lt", "lte",
                    "between", "member",
                ]
                .contains(&operator.as_str())
                {
                    return Err(invalid("Unsupported filter operator."));
                }
                let no_value = ["empty", "present"].contains(&operator.as_str());
                if operator == "member" && !crate::query_members::is_member_field(field) {
                    return Err(invalid(
                        "Member matching is supported for artist, album artist and genre.",
                    ));
                }
                if values.len() > 64
                    || (!no_value && values.is_empty())
                    || values
                        .iter()
                        .any(|v| v.len() > 512 || (!no_value && v.trim().is_empty()))
                {
                    return Err(invalid(
                        "Complete each filter value (at most 64 alternatives).",
                    ));
                }
                let numeric = ["gt", "gte", "lt", "lte", "between"].contains(&operator.as_str());
                let numbers = if numeric {
                    values
                        .iter()
                        .map(|v| {
                            v.parse::<f64>()
                                .ok()
                                .filter(|n| n.is_finite())
                                .ok_or_else(|| invalid("Numeric filters need finite numbers."))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };
                if numeric
                    && ((operator == "between" && (numbers.len() != 2 || numbers[0] > numbers[1]))
                        || (operator != "between" && numbers.len() != 1))
                {
                    return Err(invalid("Supply one number or an ordered two-number range."));
                }
                Ok(Self::Condition {
                    field: canonical_field(field).into(),
                    operator: operator.clone(),
                    values: values
                        .iter()
                        .map(|v| {
                            let value = if operator == "member" {
                                crate::query_members::normalize_space(v)
                            } else {
                                v.clone()
                            };
                            if case {
                                value
                            } else {
                                value.to_lowercase()
                            }
                        })
                        .collect(),
                    numbers,
                    negate: *negate,
                    case,
                    members: if operator == "member" {
                        Some(index.member_facet(field, cancelled)?)
                    } else {
                        None
                    },
                })
            }
        }
    }
    fn matches(&self, row: &QueryRow) -> bool {
        match self {
            Self::Group { all, children } => {
                if *all {
                    children.iter().all(|n| n.matches(row))
                } else {
                    children.iter().any(|n| n.matches(row))
                }
            }
            Self::Condition {
                field,
                operator,
                values,
                numbers,
                negate,
                case,
                members,
            } => {
                let value = if *case {
                    row.value(field)
                } else {
                    row.folded(field)
                };
                let matches = match operator.as_str() {
                    "empty" => value.trim().is_empty(),
                    "present" => !value.trim().is_empty(),
                    "equals" => values.iter().any(|v| value == v),
                    "starts" => values.iter().any(|v| value.starts_with(v)),
                    "contains" => values.iter().any(|v| value.contains(v)),
                    "member" => members
                        .as_ref()
                        .and_then(|facet| facet.credits.get(row.value(field)))
                        .is_some_and(|names| {
                            names.iter().any(|name| {
                                values.iter().any(|needle| {
                                    if *case {
                                        name == needle
                                    } else {
                                        name.to_lowercase() == *needle
                                    }
                                })
                            })
                        }),
                    _ => value
                        .split('/')
                        .next()
                        .and_then(|v| v.parse::<f64>().ok())
                        .is_some_and(|v| match operator.as_str() {
                            "gt" => v > numbers[0],
                            "gte" => v >= numbers[0],
                            "lt" => v < numbers[0],
                            "lte" => v <= numbers[0],
                            "between" => v >= numbers[0] && v <= numbers[1],
                            _ => false,
                        }),
                };
                if *negate {
                    !matches
                } else {
                    matches
                }
            }
        }
    }
    fn candidates(
        &self,
        index: &QueryIndex,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Vec<usize>>, QueryError> {
        match self {
            Self::Condition {
                field,
                operator,
                values,
                numbers,
                negate,
                case: false,
                members,
            } => {
                let facet = if operator == "member" {
                    members.clone().expect("compiled member index")
                } else {
                    index.facet(field, cancelled)?
                };
                let mut selected = facet_matches(
                    &facet,
                    |value, _| match operator.as_str() {
                        "empty" | "present" => true,
                        "equals" | "member" => values.iter().any(|v| value == v),
                        "starts" => values.iter().any(|v| value.starts_with(v)),
                        "contains" => values.iter().any(|v| value.contains(v)),
                        _ => value
                            .split('/')
                            .next()
                            .and_then(|v| v.parse::<f64>().ok())
                            .is_some_and(|value| match operator.as_str() {
                                "gt" => value > numbers[0],
                                "gte" => value >= numbers[0],
                                "lt" => value < numbers[0],
                                "lte" => value <= numbers[0],
                                "between" => value >= numbers[0] && value <= numbers[1],
                                _ => false,
                            }),
                    },
                    cancelled,
                )?;
                if (operator == "empty") != *negate {
                    selected = complement(&selected, index.rows.len());
                }
                Ok(Some(selected))
            }
            Self::Group { all, children } => {
                let mut result: Option<Vec<usize>> = None;
                for child in children {
                    let Some(selected) = child.candidates(index, cancelled)? else {
                        return Ok(None);
                    };
                    result = Some(if let Some(previous) = result {
                        if *all {
                            intersection(&previous, &selected)
                        } else {
                            union(&previous, &selected)
                        }
                    } else {
                        selected
                    });
                }
                Ok(Some(result.unwrap_or_else(|| {
                    if *all {
                        (0..index.rows.len()).collect()
                    } else {
                        Vec::new()
                    }
                })))
            }
            _ => Ok(None),
        }
    }
}

fn facet_matches(
    facet: &Facet,
    mut predicate: impl FnMut(&str, &str) -> bool,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<usize>, QueryError> {
    let mut selected = Vec::new();
    for (n, (folded, (raw, rows))) in facet.values.iter().enumerate() {
        if n % 256 == 0 && cancelled() {
            return Err(QueryError::Cancelled);
        }
        if predicate(folded, raw) {
            selected.extend_from_slice(rows);
        }
    }
    selected.sort_unstable();
    selected.dedup();
    Ok(selected)
}
fn intersection(a: &[usize], b: &[usize]) -> Vec<usize> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}
fn union(a: &[usize], b: &[usize]) -> Vec<usize> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len() + b.len());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}
fn complement(selected: &[usize], total: usize) -> Vec<usize> {
    let mut n = 0;
    let mut out = Vec::with_capacity(total.saturating_sub(selected.len()));
    for i in 0..total {
        if selected.get(n) == Some(&i) {
            n += 1;
        } else {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks() -> Vec<TrackDto> {
        (0..4).map(|i|serde_json::from_value(serde_json::json!({"path":format!("/music/{i}.flac"),"format":"FLAC","tags":{"artist":"Example","genre":if i<2{"Hardcore"}else{"Jungle"},"year":(1994+i).to_string()},"has_cover":i%2==0})).expect("fixture")).collect()
    }
    fn request(root: serde_json::Value) -> QueryRequest {
        serde_json::from_value(serde_json::json!({"requestId":1,"query":{"version":2,"root":root}}))
            .expect("fixture")
    }
    fn condition(id: &str, field: &str, operator: &str, values: Vec<&str>) -> serde_json::Value {
        serde_json::json!({"kind":"condition","id":id,"field":field,"operator":operator,"values":values})
    }
    #[test]
    fn computed_overlay_shares_large_saved_values_and_search_text() {
        let mut tracks = tracks();
        tracks[0]
            .tags
            .insert("custom:LYRICS".into(), "x".repeat(2 * 1024 * 1024));
        let index = QueryIndex::from_tracks(&tracks);
        let definitions = [CustomField {
            key: "custom:display".into(),
            mask: "%artist% · %genre%".into(),
        }];
        let computed = index
            .custom_index(&definitions, &|| false)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&index.rows[0].values, &computed.rows[0].values));
        assert!(Arc::ptr_eq(&index.rows[0].search, &computed.rows[0].search));
        assert_eq!(
            computed.rows[0].value("custom:display"),
            "Example · Hardcore"
        );
        assert_eq!(index.rows[0].value("custom:display"), "");
        assert_eq!(
            computed.rows[0].value("custom:LYRICS").len(),
            2 * 1024 * 1024
        );
    }
    #[test]
    fn facet_lru_keeps_hot_fields_and_budget_refuses_before_cloning_a_value() {
        let index = QueryIndex::from_tracks(&tracks());
        *index.facets.lock().unwrap() = crate::bounded_cache::Cache::new(2, 100_000);
        let hot = index.facet("artist", &|| false).unwrap();
        let cold = index.facet("genre", &|| false).unwrap();
        assert!(Arc::ptr_eq(
            &hot,
            &index.facet("artist", &|| false).unwrap()
        ));
        index.facet("year", &|| false).unwrap();
        assert!(Arc::ptr_eq(
            &hot,
            &index.facet("artist", &|| false).unwrap()
        ));
        assert!(!Arc::ptr_eq(
            &cold,
            &index.facet("genre", &|| false).unwrap()
        ));
        let mut values = BTreeMap::new();
        let mut bytes = MAX_FACET_WORK_BYTES - 5;
        assert!(push_facet_value(&mut values, &mut bytes, "new", "new", 0).is_err());
        assert!(values.is_empty());
        assert!(index.facet("title", &|| true).is_err());
    }
    #[test]
    fn nested_groups_multivalues_and_ranges() {
        let index = QueryIndex::from_tracks(&tracks());
        let query = request(
            serde_json::json!({"kind":"group","id":"root","mode":"all","children":[condition("genres","genre","equals",vec!["Hardcore","Jungle"]),{"kind":"group","id":"any","mode":"any","children":[condition("years","year","between",vec!["1995","1996"]),condition("art","cover","equals",vec!["missing"])]}]}),
        );
        assert_eq!(
            index.query(&query, || false).expect("valid").indices,
            vec![1, 2, 3]
        );
    }
    #[test]
    fn unknown_fields_and_bad_regex_do_not_broaden() {
        let index = QueryIndex::from_tracks(&tracks());
        assert!(index
            .query(
                &request(condition("bad", "not_a_tag", "equals", vec!["x"])),
                || false
            )
            .is_err());
        let mut q =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}));
        q.query.regex = true;
        q.query.text = "(?<=a)b".into();
        assert!(index.query(&q, || false).is_err());
    }
    #[test]
    fn manifests_are_once_per_snapshot_and_cancellation_is_checked() {
        let index = QueryIndex::from_tracks(&tracks());
        let mut q =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}));
        assert_eq!(
            index
                .query(&q, || false)
                .expect("valid")
                .paths
                .expect("manifest")
                .len(),
            4
        );
        q.snapshot_id = Some(index.id);
        assert!(index.query(&q, || false).expect("valid").paths.is_none());
        assert!(matches!(
            index.query(&q, || true),
            Err(QueryError::Cancelled)
        ));
    }
    #[test]
    fn facets_respect_existing_filters() {
        let index = QueryIndex::from_tracks(&tracks());
        let q = request(condition("genre", "genre", "equals", vec!["Hardcore"]));
        let suggestions = index
            .suggest(
                &SuggestRequest {
                    request_id: 1,
                    snapshot_id: None,
                    query: q.query,
                    text: "199".into(),
                    field: Some("year".into()),
                    limit: 12,
                    strategy: "values".into(),
                },
                || false,
            )
            .expect("valid");
        assert_eq!(
            suggestions
                .suggestions
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            vec!["1994", "1995"]
        );
    }

    fn field_request(text: &str, field: Option<&str>) -> SuggestRequest {
        serde_json::from_value(serde_json::json!({
            "requestId":1,"text":text,"field":field,"strategy":"fields",
            "query":{"version":2,"root":{"kind":"group","id":"root","mode":"all","children":[]}}
        }))
        .unwrap()
    }

    fn field_tracks() -> Vec<TrackDto> {
        [
            ("Hixxy & Sharkey", "Hixxy / Dougal", "Hardcore; Jungle"),
            ("Hixxy / Hixxy", "Hixxy", "Hardcore"),
            ("Hixxy2", "Other", "Happy Hardcore"),
            ("Other", "Other", "Jungle"),
        ]
        .iter()
        .enumerate()
        .map(|(i, (artist, albumartist, genre))| {
            serde_json::from_value(serde_json::json!({
                "path":format!("/music/{i}.flac"),"format":"FLAC","has_cover":i%2==0,
                "tags":{"artist":artist,"albumartist":albumartist,"genre":genre,
                    "title":"Hixxy recording","album":"Hixxy album","publisher":"Hixxy label","year":"1996"}
            })).unwrap()
        })
        .collect()
    }

    #[test]
    fn field_suggestions_combine_credits_once_and_preserve_typed_contains_term() {
        let index = QueryIndex::from_tracks(&field_tracks());
        let request = field_request("hIxXy", None);
        let result = index.suggest(&request, || false).unwrap();
        assert_eq!(result.suggestions.len(), 5);
        assert_eq!(result.suggestions[0].field, "artist");
        let mut fields = std::collections::HashSet::new();
        for suggestion in &result.suggestions {
            assert!(fields.insert(&suggestion.field));
            assert_eq!(suggestion.operator, "contains");
            assert_eq!(suggestion.value, "hIxXy");
            let expected = match suggestion.field.as_str() {
                "artist" => 3, // Hixxy / Hixxy is one row; contains also finds Hixxy2.
                "albumartist" => 2,
                _ => 4,
            };
            assert_eq!(suggestion.count, expected);
        }
        let mut limited = request;
        limited.limit = 1;
        let result = index.suggest(&limited, || false).unwrap();
        assert_eq!(result.suggestions[0].field, "artist");
        assert!(result.truncated);
    }

    #[test]
    fn field_suggestions_count_the_emitted_rule_under_case_sensitive_matching() {
        let tracks = [
            ("Hixxy & Sharkey", "Hardcore; Jungle"),
            ("hixxy / Dougal", "hardcore"),
            ("HIXXY", "HARDCORE"),
            ("Hixxy", "Hardcore"),
        ]
        .iter()
        .enumerate()
        .map(|(i, (artist, genre))| {
            serde_json::from_value(serde_json::json!({
                "path":format!("/music/{i}.flac"),"format":"FLAC",
                "tags":{"artist":artist,"genre":genre}
            }))
            .unwrap()
        })
        .collect::<Vec<TrackDto>>();
        let index = QueryIndex::from_tracks(&tracks);
        for case_sensitive in [false, true] {
            for (field, term, operator) in [
                ("artist", "Hixxy", "contains"),
                ("genre", "hardcore", "member"),
            ] {
                let mut request = field_request(term, Some(field));
                request.query.case_sensitive = case_sensitive;
                let result = index.suggest(&request, || false).unwrap();
                assert_eq!(result.suggestions.len(), 1);
                let suggestion = &result.suggestions[0];
                assert_eq!(suggestion.operator, operator);
                let mut emitted = QueryRequest {
                    request_id: 2,
                    snapshot_id: Some(index.id),
                    query: request.query,
                };
                emitted.query.root = serde_json::from_value(condition(
                    "emitted",
                    &suggestion.field,
                    &suggestion.operator,
                    vec![&suggestion.value],
                ))
                .unwrap();
                assert_eq!(suggestion.count, if case_sensitive { 2 } else { 4 });
                assert_eq!(
                    suggestion.count,
                    index.query(&emitted, || false).unwrap().matched,
                    "{field}/{case_sensitive}"
                );
            }
        }
    }

    #[test]
    fn field_suggestions_use_exact_category_rules_and_known_prefixes() {
        let mut tracks = field_tracks();
        tracks[0]
            .tags
            .insert("custom:MOOD:DETAIL".into(), "Dark".into());
        let index = QueryIndex::from_tracks(&tracks);
        for (text, field, value, operator, count) in [
            ("GENRE:hardCORE", "genre", "Hardcore", "member", 2),
            ("genre:hard", "genre", "hard", "contains", 3),
            ("format:flac", "format", "FLAC", "equals", 4),
            ("year:199", "year", "199", "contains", 4),
            ("cover:missing", "cover", "missing", "equals", 2),
            (
                "custom:MOOD:DETAIL:dark",
                "custom:MOOD:DETAIL",
                "dark",
                "contains",
                1,
            ),
        ] {
            let result = index.suggest(&field_request(text, None), || false).unwrap();
            assert_eq!(result.suggestions.len(), 1, "{text}");
            let suggestion = &result.suggestions[0];
            assert_eq!(
                (
                    &*suggestion.field,
                    &*suggestion.value,
                    &*suggestion.operator,
                    suggestion.count
                ),
                (field, value, operator, count)
            );
        }
        let mut explicit = field_request("hixxy", Some("title"));
        let result = index.suggest(&explicit, || false).unwrap();
        assert_eq!(result.suggestions[0].field, "title");
        explicit.text = "artist:hixxy".into();
        assert!(index
            .suggest(&explicit, || false)
            .unwrap()
            .suggestions
            .is_empty());
        let result = index
            .suggest(&field_request("Hardcore", None), || false)
            .unwrap();
        assert_eq!(result.suggestions[0].field, "genre");
        assert_eq!(result.suggestions[0].operator, "member");
    }

    #[test]
    fn field_suggestions_disjoin_only_positive_top_level_same_field_contains() {
        let index = QueryIndex::from_tracks(&field_tracks());
        let mut request = field_request("hixxy", Some("artist"));
        request.query.root = serde_json::from_value(serde_json::json!({
            "kind":"group","id":"root","mode":"all","children":[
                condition("genre","genre","member",vec!["Hardcore"]),
                condition("artist","artist","contains",vec!["Other"])
            ]
        }))
        .unwrap();
        let result = index.suggest(&request, || false).unwrap();
        assert_eq!(result.suggestions[0].count, 2);
        let mut legacy = request.clone();
        legacy.strategy = "values".into();
        assert!(index
            .suggest(&legacy, || false)
            .unwrap()
            .suggestions
            .is_empty());
        request.query.root = serde_json::from_value(serde_json::json!({
            "kind":"group","id":"root","mode":"all","children":[{
                "kind":"group","id":"nested","mode":"all","children":[
                    condition("artist","artist","contains",vec!["Other"])
                ]
            }]
        }))
        .unwrap();
        assert!(index
            .suggest(&request, || false)
            .unwrap()
            .suggestions
            .is_empty());
        request.query.root = serde_json::from_value(serde_json::json!({
            "kind":"group","id":"root","mode":"any","children":[
                condition("artist","artist","contains",vec!["Other"])
            ]
        }))
        .unwrap();
        assert!(index
            .suggest(&request, || false)
            .unwrap()
            .suggestions
            .is_empty());
    }

    #[test]
    fn field_suggestions_respect_duplicate_members_and_cancellation() {
        let index = QueryIndex::from_tracks(&field_tracks());
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "complete".into(),
                groups: vec![DuplicateQueryGroup {
                    id: "g".into(),
                    label: "Copies".into(),
                    reason: "bytes".into(),
                    confidence: "exact".into(),
                    indices: vec![0, 2],
                }],
                ..Default::default()
            },
        );
        let mut request = field_request("hixxy", Some("artist"));
        request.query.mode = "duplicates".into();
        let result = index.suggest(&request, || false).unwrap();
        assert_eq!(result.suggestions[0].count, 2);
        assert!(matches!(
            index.suggest(&request, || true),
            Err(QueryError::Cancelled)
        ));
        request.strategy = "unknown".into();
        assert!(index.suggest(&request, || false).is_err());
    }

    #[test]
    fn same_field_facets_offer_alternatives_without_rewriting_nested_groups() {
        let index = QueryIndex::from_tracks(&tracks());
        let query = request(
            serde_json::json!({"kind":"group","id":"root","mode":"all","children":[condition("genre","genre","equals",vec!["Hardcore"])]}),
        );
        let result = index
            .suggest(
                &SuggestRequest {
                    request_id: 1,
                    snapshot_id: None,
                    query: query.query,
                    text: String::new(),
                    field: Some("genre".into()),
                    limit: 12,
                    strategy: "values".into(),
                },
                || false,
            )
            .unwrap();
        assert_eq!(result.suggestions.len(), 2);
        let query = request(
            serde_json::json!({"kind":"group","id":"root","mode":"any","children":[condition("genre","genre","equals",vec!["Hardcore"])]}),
        );
        let result = index
            .suggest(
                &SuggestRequest {
                    request_id: 2,
                    snapshot_id: None,
                    query: query.query,
                    text: String::new(),
                    field: Some("genre".into()),
                    limit: 12,
                    strategy: "values".into(),
                },
                || false,
            )
            .unwrap();
        assert_eq!(result.suggestions.len(), 1);
    }

    #[test]
    fn localized_artwork_search_and_regex_preserve_status_aliases() {
        let index = QueryIndex::from_tracks(&tracks());
        let mut query =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}));
        query
            .query
            .cover_labels
            .insert("missing".into(), "No embedded artwork".into());
        query.query.text = "cover:No embedded artwork".into();
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![1, 3]);
        query.query.text = "cover:embedded".into();
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![0, 2]);
        query.query.text = "cover:^No embedded".into();
        query.query.regex = true;
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![1, 3]);
    }

    #[test]
    fn custom_tags_and_mask_columns_use_lossless_keys_and_cached_values() {
        let mut tracks = tracks();
        tracks[0].tags.insert("custom:MOOD".into(), "Dark".into());
        let index = QueryIndex::from_tracks(&tracks);
        let mut query =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}));
        query.query.text = "custom:MOOD:dark".into();
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![0]);
        query.query.text.clear();
        query.query.root =
            serde_json::from_value(condition("absent", "custom:ABSENT", "empty", vec![])).unwrap();
        assert_eq!(index.query(&query, || false).unwrap().matched, 4);
        query.query.root = serde_json::from_value(condition(
            "computed",
            "custom:column-id",
            "equals",
            vec!["Example · Hardcore"],
        ))
        .unwrap();
        query.query.custom_fields = vec![CustomField {
            key: "custom:column-id".into(),
            mask: "%artist% · %genre%".into(),
        }];
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![0, 1]);
        let first = index
            .custom_index(&query.query.custom_fields, &|| false)
            .unwrap()
            .unwrap();
        let second = index
            .custom_index(&query.query.custom_fields, &|| false)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        index.invalidate();
        assert!(!first.is_current());
    }

    #[test]
    fn indexed_conditions_match_row_evaluation_for_all_operators() {
        let mut tracks = tracks();
        tracks[0].tags.remove("genre");
        tracks[1].tags.insert("genre".into(), " ".into());
        let index = QueryIndex::from_tracks(&tracks);
        for (field, operator, values) in [
            ("genre", "empty", vec![]),
            ("genre", "present", vec![]),
            ("genre", "contains", vec!["ung"]),
            ("genre", "starts", vec!["J"]),
            ("genre", "equals", vec!["Jungle"]),
            ("year", "between", vec!["1995", "1996"]),
            ("year", "gt", vec!["1995"]),
        ] {
            for negate in [false, true] {
                let mut node = condition("condition", field, operator, values.clone());
                node["negate"] = serde_json::json!(negate);
                let query = request(node);
                let compiled = CompiledQuery::new(&query.query, &index, &|| false).unwrap();
                let expected = index
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| compiled.root.matches(row))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                assert_eq!(
                    index.query(&query, || false).unwrap().indices,
                    expected,
                    "{field}/{operator}/{negate}"
                );
            }
        }
    }

    #[test]
    fn duplicate_facets_are_restricted_to_analyzed_members() {
        let index = QueryIndex::from_tracks(&tracks());
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "complete".into(),
                groups: vec![DuplicateQueryGroup {
                    id: "g".into(),
                    label: "Copies".into(),
                    reason: "bytes".into(),
                    confidence: "exact".into(),
                    indices: vec![0, 1],
                }],
                ..Default::default()
            },
        );
        let mut query =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}))
                .query;
        query.mode = "duplicates".into();
        let result = index
            .suggest(
                &SuggestRequest {
                    request_id: 1,
                    snapshot_id: None,
                    query,
                    text: String::new(),
                    field: Some("genre".into()),
                    limit: 12,
                    strategy: "values".into(),
                },
                || false,
            )
            .unwrap();
        assert_eq!(result.suggestions.len(), 1);
        assert_eq!(result.suggestions[0].value, "Hardcore");
        assert_eq!(result.suggestions[0].count, 2);
    }

    #[test]
    fn duplicate_suggestions_never_fall_back_to_unscanned_library_values() {
        let index = QueryIndex::from_tracks(&tracks());
        let mut request = field_request("Hardcore", Some("genre"));
        request.query.mode = "duplicates".into();
        for status in ["not_scanned", "running", "cancelled", "failed"] {
            index.set_duplicate_results(
                "exact",
                DuplicateResults {
                    status: status.into(),
                    // Empty running scans and stale terminal groups cannot
                    // broaden suggestion scope to the opened library.
                    groups: if status == "running" {
                        vec![]
                    } else {
                        vec![DuplicateQueryGroup {
                            id: "partial".into(),
                            label: "Copies".into(),
                            reason: "bytes".into(),
                            confidence: "exact".into(),
                            indices: vec![0, 1],
                        }]
                    },
                    ..Default::default()
                },
            );
            for strategy in ["fields", "values"] {
                request.strategy = strategy.into();
                assert!(
                    index
                        .suggest(&request, || false)
                        .unwrap()
                        .suggestions
                        .is_empty(),
                    "{status}/{strategy}"
                );
            }
        }
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "complete".into(),
                groups: vec![DuplicateQueryGroup {
                    id: "complete".into(),
                    label: "Copies".into(),
                    reason: "bytes".into(),
                    confidence: "exact".into(),
                    indices: vec![2, 3],
                }],
                ..Default::default()
            },
        );
        for strategy in ["fields", "values"] {
            request.strategy = strategy.into();
            request.text = "Hardcore".into();
            assert!(index
                .suggest(&request, || false)
                .unwrap()
                .suggestions
                .is_empty());
            request.text = "Jungle".into();
            let result = index.suggest(&request, || false).unwrap();
            assert_eq!(result.suggestions.len(), 1);
            assert_eq!(result.suggestions[0].value, "Jungle");
            assert_eq!(result.suggestions[0].count, 2);
        }
    }

    #[test]
    fn running_duplicate_groups_supply_partial_queries_facets_and_total_group_count() {
        let index = QueryIndex::from_tracks(&tracks());
        let derived = index
            .custom_index(
                &[CustomField {
                    key: "custom:progress".into(),
                    mask: "%artist%".into(),
                }],
                &|| false,
            )
            .unwrap()
            .unwrap();
        assert!(derived.duplicate_results("exact").groups.is_empty());
        let groups = vec![
            DuplicateQueryGroup {
                id: "one".into(),
                label: "First".into(),
                reason: "bytes".into(),
                confidence: "exact".into(),
                indices: vec![0, 1],
            },
            DuplicateQueryGroup {
                id: "two".into(),
                label: "Second".into(),
                reason: "bytes".into(),
                confidence: "exact".into(),
                indices: vec![2, 3],
            },
        ];
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "running".into(),
                groups,
                groups_revision: 3,
                ..Default::default()
            },
        );
        assert_eq!(derived.duplicate_results("exact").groups_revision, 3);
        assert_eq!(derived.duplicate_results("exact").groups.len(), 2);
        let mut query = request(condition("genre", "genre", "equals", vec!["Hardcore"]));
        query.query.mode = "duplicates".into();
        let result = index.query(&query, || false).unwrap();
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups_count, 2);
        assert_eq!(result.groups_revision, 3);
        assert_eq!(result.indices, vec![0, 1]);
        for strategy in ["fields", "values"] {
            let mut suggestion = field_request("Hardcore", Some("genre"));
            suggestion.strategy = strategy.into();
            suggestion.query.mode = "duplicates".into();
            let result = index.suggest(&suggestion, || false).unwrap();
            assert_eq!(result.suggestions.len(), 1);
            assert_eq!(result.suggestions[0].count, 2);
        }
        index.invalidate();
        assert!(index.duplicate_results("exact").groups.is_empty());
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "running".into(),
                groups_revision: 99,
                ..Default::default()
            },
        );
        assert_eq!(index.duplicate_results("exact").status, "not_scanned");
        assert!(matches!(
            index.query(&query, || false),
            Err(QueryError::Cancelled)
        ));
    }

    #[test]
    fn virtual_fields_match_saved_table_labels() {
        let mut track = tracks().remove(0);
        track.tags.insert("media".into(), "Vinyl".into());
        track.tags.insert("disc".into(), "2".into());
        track.tags.insert("track".into(), "3".into());
        track.tag_blocks = vec![
            crate::TagBlockDto {
                label: "ID3v1".into(),
                kind: "id3v1".into(),
                read_from: false,
            },
            crate::TagBlockDto {
                label: "ID3v2.4".into(),
                kind: "id3v2".into(),
                read_from: true,
            },
        ];
        let row = QueryRow::from_track(&track);
        assert_eq!(row.value("position"), "B3");
        assert_eq!(row.value("tagtypes"), "ID3v2.4 + ID3v1");
    }

    #[test]
    fn artist_and_genre_members_are_exact_deduplicated_and_suggested() {
        let credits = [
            "Hixxy",
            "Hixxy & Sharkey",
            "Hixxy / Dougal",
            "Hixxy, Seduction",
            "Hixxy2 & Sharkey",
            "Hixxy&Sharkey",
            "Earth, Wind & Fire",
            "AC/DC",
            "Hixxy; hixxy",
        ];
        let tracks=credits.iter().enumerate().map(|(i,artist)|serde_json::from_value(serde_json::json!({"path":format!("/music/{i}.flac"),"format":"FLAC","tags":{"artist":artist,"albumartist":artist,"genre":"Drum & Bass; Jungle"}})).unwrap()).collect::<Vec<TrackDto>>();
        let index = QueryIndex::from_tracks(&tracks);
        let query = request(condition("artist", "artist", "member", vec!["  hixxy  "]));
        assert_eq!(
            index.query(&query, || false).unwrap().indices,
            vec![0, 1, 2, 3, 5, 8]
        );
        let query = request(condition(
            "artists",
            "artist",
            "member",
            vec!["Hixxy", "Sharkey"],
        ));
        assert_eq!(
            index.query(&query, || false).unwrap().indices,
            vec![0, 1, 2, 3, 4, 5, 8]
        );
        let query = request(condition("whole", "artist", "equals", vec!["Hixxy"]));
        assert_eq!(index.query(&query, || false).unwrap().indices, vec![0]);
        let query = request(condition("genre", "genre", "member", vec!["Drum & Bass"]));
        assert_eq!(
            index.query(&query, || false).unwrap().matched,
            credits.len()
        );
        let query = request(condition("genre", "genre", "member", vec!["Bass"]));
        assert_eq!(index.query(&query, || false).unwrap().matched, 0);
        let query =
            request(serde_json::json!({"kind":"group","id":"root","mode":"all","children":[]}));
        let suggestions = index
            .suggest(
                &SuggestRequest {
                    request_id: 1,
                    snapshot_id: None,
                    query: query.query,
                    text: "Hixxy".into(),
                    field: Some("artist".into()),
                    limit: 12,
                    strategy: "values".into(),
                },
                || false,
            )
            .unwrap();
        let hixxy = suggestions
            .suggestions
            .iter()
            .find(|s| s.value == "Hixxy")
            .unwrap();
        assert_eq!(hixxy.count, 6);
        assert_eq!(hixxy.operator, "member");
    }
    #[test]
    fn duplicate_filter_retains_companion_files() {
        let index = QueryIndex::from_tracks(&tracks());
        index.set_duplicate_results(
            "exact",
            DuplicateResults {
                status: "complete".into(),
                groups: vec![DuplicateQueryGroup {
                    id: "a".into(),
                    label: "Copies".into(),
                    reason: "Identical bytes".into(),
                    confidence: "exact".into(),
                    indices: vec![0, 2],
                }],
                ..Default::default()
            },
        );
        let mut q = request(condition("genre", "genre", "equals", vec!["Hardcore"]));
        q.query.mode = "duplicates".into();
        assert_eq!(
            index.query(&q, || false).expect("valid").indices,
            vec![0, 2]
        );
    }
}

/// Scoped search resolution and derived-cache retention under a high-cardinality
/// library. Both are read-only caches: results must not change, only the work
/// done to reach them.
#[cfg(test)]
mod scoped_text_and_facet_cache_tests {
    use super::*;

    fn row(path: &str, tags: serde_json::Value) -> QueryRow {
        let track: TrackDto = serde_json::from_value(serde_json::json!({
            "path": path, "format": "FLAC", "tags": tags
        }))
        .expect("fixture");
        QueryRow::from_track(&track)
    }
    fn index() -> QueryIndex {
        QueryIndex::from_rows(vec![
            row(
                "/music/a.flac",
                serde_json::json!({
                    "artist": "Hixxy", "genre": "Jungle",
                    "comment": "played 01:23:45", "custom:MOOD": "Dark"
                }),
            ),
            row(
                "/music/b.flac",
                serde_json::json!({
                    "artist": "Sharkey", "genre": "Hardcore", "media": "Vinyl"
                }),
            ),
        ])
    }
    fn model(text: &str) -> QueryModel {
        serde_json::from_value(serde_json::json!({
            "version":2,"text":text,
            "root":{"kind":"group","id":"root","mode":"all","children":[]}
        }))
        .expect("fixture")
    }
    fn query(index: &QueryIndex, text: &str) -> Vec<usize> {
        index
            .query(
                &QueryRequest {
                    request_id: 1,
                    snapshot_id: Some(index.id),
                    query: model(text),
                },
                || false,
            )
            .expect("valid query")
            .indices
    }

    #[test]
    fn scoped_search_resolves_known_fields_without_row_evidence() {
        let index = index();
        // A standard field resolves from the built-in list, the way an ordinary
        // "genre:jungle" search does, and a stored key is preserved exactly.
        assert_eq!(index.text_scope("genre:jungle"), Some(("genre".into(), 6)));
        assert_eq!(index.text_scope("GENRE:jungle"), Some(("genre".into(), 6)));
        assert_eq!(
            index.text_scope("custom:MOOD:dark"),
            Some(("custom:MOOD".into(), 12))
        );
        assert_eq!(index.text_scope("media:vinyl"), Some(("media".into(), 6)));
        // A time-like text is not a field scope and never becomes one.
        assert_eq!(index.text_scope("01:23:45"), None);
        assert_eq!(index.text_scope("played 01:23:45"), None);
        // Results are the same on the first and later resolutions.
        assert_eq!(query(&index, "genre:jungle"), vec![0]);
        assert_eq!(query(&index, "genre:jungle"), vec![0]);
        assert_eq!(query(&index, "played 01:23:45"), vec![0]);
        assert_eq!(query(&index, "01:23:45"), vec![0]);
        assert!(index.known_field("media"));
        assert!(!index.known_field("not_a_tag"));
    }

    #[test]
    fn absent_scope_candidates_are_remembered_and_the_memo_stays_bounded() {
        let index = index();
        // Many distinct non-field prefixes, the shape of a colon-heavy search
        // text that used to rescan every row per candidate.
        for i in 0..(MAX_FIELD_KEY_MEMO * 3) {
            assert_eq!(index.text_scope(&format!("n{i}:1:2:3")), None, "{i}");
        }
        let memo = index.field_keys.lock().unwrap();
        assert!(memo.len() <= MAX_FIELD_KEY_MEMO);
        assert!(memo
            .iter()
            .all(|(name, known)| !*known || index.known_field(name)));
        drop(memo);
        // Correctness is unaffected once the memo is full: the fallback scan
        // still resolves a stored key and still rejects an absent one.
        assert_eq!(
            index.text_scope("custom:MOOD:dark"),
            Some(("custom:MOOD".into(), 12))
        );
        assert_eq!(index.text_scope("n999999:1"), None);
        assert!(index.known_field("custom:MOOD"));
        assert!(!index.known_field("absent_field"));
    }

    #[test]
    fn a_computed_key_that_case_folds_to_a_standard_name_keeps_its_identity() {
        let index = index();
        let derived = index
            .custom_index(
                &[CustomField {
                    key: "DATE".into(),
                    mask: "%genre%".into(),
                }],
                &|| false,
            )
            .unwrap()
            .unwrap();
        assert_eq!(derived.text_scope("DATE:jungle"), Some(("DATE".into(), 5)));
        let mut request = QueryRequest {
            request_id: 1,
            snapshot_id: Some(index.id),
            query: model("DATE:jungle"),
        };
        request.query.custom_fields = vec![CustomField {
            key: "DATE".into(),
            mask: "%genre%".into(),
        }];
        assert_eq!(index.query(&request, || false).unwrap().indices, vec![0]);
        // Without the computed field the same text is a plain year search.
        request.query.custom_fields.clear();
        assert!(index.query(&request, || false).unwrap().indices.is_empty());
    }

    #[test]
    fn long_scope_candidates_are_resolved_without_being_remembered() {
        let index = index();
        let long = "z".repeat(MAX_FIELD_KEY_LEN + 1);
        assert_eq!(index.text_scope(&format!("{long}:value")), None);
        assert!(index.field_keys.lock().unwrap().len() <= MAX_FIELD_KEY_MEMO);
        assert!(index
            .field_keys
            .lock()
            .unwrap()
            .keys()
            .all(|name| name.len() <= MAX_FIELD_KEY_LEN));
    }

    const ARTISTS: [&str; 3] = ["Hixxy", "Sharkey", "Dougal"];
    const GENRES: [&str; 2] = ["Jungle", "Hardcore"];
    fn high_cardinality(rows: usize) -> QueryIndex {
        QueryIndex::from_rows(
            (0..rows)
                .map(|i| {
                    row(
                        &format!("/music/{i:05}.flac"),
                        serde_json::json!({
                            "artist": ARTISTS[i % ARTISTS.len()],
                            "album": format!("Album {i}"),
                            "title": format!("Track {i:05}"),
                            "genre": GENRES[i % GENRES.len()],
                        }),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn hot_high_cardinality_facets_stay_resident_across_suggestions() {
        // A suggestion reads eight fields. When the budget can hold that whole
        // working set, the two unique-per-row fields must be reused rather than
        // rebuilt on the next keystroke; the eviction cycle only appeared once
        // the budget was smaller than the set (measured at 100k rows).
        let index = high_cardinality(2_000);
        let working = [
            "genre",
            "artist",
            "albumartist",
            "album",
            "title",
            "year",
            "label",
            "format",
        ]
        .iter()
        .map(|field| index.facet(field, &|| false).unwrap().retained_bytes())
        .sum::<usize>()
        .saturating_add(4_096);
        *index.facets.lock().unwrap() =
            crate::bounded_cache::Cache::new(FACET_CACHE_ENTRIES, working);
        let request: SuggestRequest = serde_json::from_value(serde_json::json!({
            "requestId":1,"text":"","strategy":"values",
            "query":{"version":2,"root":{"kind":"group","id":"root","mode":"all","children":[]}}
        }))
        .unwrap();
        let first = index.suggest(&request, || false).unwrap();
        assert!(!first.suggestions.is_empty());
        let album = index.facet("album", &|| false).unwrap();
        let title = index.facet("title", &|| false).unwrap();
        let second = index.suggest(&request, || false).unwrap();
        assert_eq!(
            first
                .suggestions
                .iter()
                .map(|s| (&s.field, &s.value, s.count))
                .collect::<Vec<_>>(),
            second
                .suggestions
                .iter()
                .map(|s| (&s.field, &s.value, s.count))
                .collect::<Vec<_>>()
        );
        assert!(
            Arc::ptr_eq(&album, &index.facet("album", &|| false).unwrap()),
            "album facet was rebuilt between suggestions"
        );
        assert!(
            Arc::ptr_eq(&title, &index.facet("title", &|| false).unwrap()),
            "title facet was rebuilt between suggestions"
        );
        let limits = index.facets.lock().unwrap().limits();
        assert_eq!(limits.1, working);
    }

    #[test]
    fn the_default_facet_budget_holds_two_large_facets() {
        // 17.3 and 17.5 MiB were measured for the 100k-distinct album and title
        // facets of a 100k-row library. A budget under their sum forces every
        // suggestion to rebuild both, so this guards the retention decision.
        const { assert!(FACET_CACHE_BYTES >= 2 * 17 * 1024 * 1024) };
        const { assert!(FACET_CACHE_ENTRIES >= 2) };
        let index = index();
        assert_eq!(
            index.facets.lock().unwrap().limits(),
            (FACET_CACHE_ENTRIES, FACET_CACHE_BYTES)
        );
        let derived = index
            .custom_index(
                &[CustomField {
                    key: "custom:display".into(),
                    mask: "%artist%".into(),
                }],
                &|| false,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            derived.facets.lock().unwrap().limits(),
            (FACET_CACHE_ENTRIES, FACET_CACHE_BYTES),
            "a computed overlay must not inherit the old thrashing budget"
        );
    }
}

#[cfg(test)]
mod overlay_owner_tests {
    use super::*;
    #[test]
    fn immutable_overlay_rows_keep_the_batch_and_base_charged_until_last_clone() {
        let name = "query::overlay_owner_tests::immutable_overlay_rows_keep_the_batch_and_base_charged_until_last_clone";
        if std::env::var("TAGREX_OVERLAY_OWNER_CHILD").as_deref() != Ok("1") {
            let output =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args(["--exact", name])
                    .env("TAGREX_OVERLAY_OWNER_CHILD", "1")
                    .output()
                    .expect("child process");
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let tracks: Vec<TrackDto> = (0..4).map(|i| serde_json::from_value(serde_json::json!({
            "path":format!("/fixture/{i}.flac"),"format":"FLAC","tags":{"comment":"Long value ".repeat(4000)},"has_cover":false
        })).expect("fixture")).collect();
        let budget = tagrex_core::resources::Budget::process();
        let index = QueryIndex::try_from_tracks(&tracks).expect("base admission");
        let derived = index
            .custom_index(
                &[CustomField {
                    key: "custom:display".into(),
                    mask: "%comment%".into(),
                }],
                &|| false,
            )
            .expect("overlay admission")
            .expect("overlay");
        let row = derived.rows[0].clone();
        let clone = (*row).clone();
        assert!(Arc::ptr_eq(&row.computed, &clone.computed));
        drop(index);
        drop(derived);
        drop(row);
        assert!(budget.usage().0.retained > 0);
        assert!(!clone.value("custom:display").is_empty());
        drop(clone);
        assert_eq!(budget.usage().0, tagrex_core::resources::Charge::default());
    }
}
