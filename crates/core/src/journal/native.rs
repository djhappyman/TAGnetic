//! Durable storage for Executor-owned protected FLAC operations. Not a write API.
use super::*;
use crate::{
    media_mutation::{self as media, Record, Store},
    resources::{Budget, Charge, Lease},
};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{fs::File, sync::Arc};

// Only admission is retried, before allocation or SQL effects. SQL errors and
// uncertain acknowledgements are never retried here.
fn admit(charge: Charge) -> Result<Lease, crate::resources::AdmissionError> {
    let start = std::time::Instant::now();
    loop {
        match Budget::process().try_acquire(charge) {
            Err(crate::resources::AdmissionError::Busy)
                if start.elapsed() < std::time::Duration::from_millis(500) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result,
        }
    }
}
pub(crate) const LIMIT: usize = 32 * 1024 * 1024;
const ROWS: usize = 8192;
const PATH_MAGIC: &[u8] = b"TRX-NATIVE-PATH\x01";
pub struct Intent {
    pub(crate) identity: [u8; 32],
    pub(crate) operation_id: String,
    pub(crate) digest: [u8; 32],
    pub(crate) roots_digest: [u8; 32],
    pub(crate) bytes: Vec<u8>,
    pub(crate) _owner: Arc<Lease>,
}
pub struct Entry {
    pub(crate) ordinal: usize,
    pub(crate) intent: Intent,
    pub(crate) operation: u8,
    pub(crate) record: Option<Record>,
    pub(crate) receipts: Option<Vec<u8>>,
    pub(crate) recovery: Option<Vec<u8>>,
    pub(crate) recovery_role: u8,
    pub(crate) done: bool,
}
pub struct Entries {
    pub(crate) rows: Vec<Entry>,
    _owner: Arc<Lease>,
}
#[doc(hidden)]
pub trait NativeJournal {
    fn store(&mut self) -> Result<Store, JournalError>;
    fn record_native(
        &mut self,
        batch: &AppliedBatch,
        intents: &[Intent],
    ) -> Result<BatchId, JournalError>;
    fn entries(&self, batch: &AppliedBatch) -> Result<Entries, JournalError>;
    fn begin(
        &mut self,
        batch: &AppliedBatch,
        ordinal: usize,
        operation: u8,
    ) -> Result<String, JournalError>;
    fn checkpoint(
        &mut self,
        batch: &AppliedBatch,
        position: (usize, u8),
        record: &Record,
        expected: &Intent,
        receipts: Option<&[u8]>,
        done: bool,
    ) -> Result<(), JournalError>;
    fn confirm(
        &self,
        batch: &AppliedBatch,
        position: (usize, u8),
        record: &Record,
        expected: &Intent,
        receipts: &[u8],
    ) -> Result<(), JournalError>;
    fn finish_native(&mut self, batch: &AppliedBatch) -> Result<(), JournalError>;
    fn retain_native(&mut self, batch: &AppliedBatch) -> Result<(), JournalError>;
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError>;
}
pub(crate) struct Profile {
    parent: File,
    path: PathBuf,
    logical_parent: PathBuf,
    store: Option<Store>,
}
impl Profile {
    pub fn capture(path: &Path) -> Result<Option<Self>, JournalError> {
        if path == Path::new(":memory:") {
            return Ok(None);
        }
        let path = std::path::absolute(path).map_err(storage)?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid("journal profile parent"))?;
        let file = match rustix::fs::open(
            parent,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(_) => return Ok(None),
        };
        if file.metadata().and_then(|m| m.created()).is_err() {
            return Ok(None);
        }
        // One store per database, not per batch. No persisted path selects it.
        let name = path
            .file_name()
            .ok_or_else(|| invalid("journal profile name"))?;
        let mut store_name = name.to_os_string();
        store_name.push(".native-media");
        Ok(Some(Self {
            parent: file,
            path: std::fs::canonicalize(parent)
                .map_err(storage)?
                .join(store_name),
            logical_parent: parent.to_path_buf(),
            store: None,
        }))
    }
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        let now = File::from(
            rustix::fs::open(
                &self.logical_parent,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(std::io::Error::from)
            .map_err(storage)?,
        );
        if same_file::Handle::from_file(now).map_err(storage)?
            != same_file::Handle::from_file(self.parent.try_clone().map_err(storage)?)
                .map_err(storage)?
        {
            return Err(invalid("journal profile parent changed"));
        }
        Ok(())
    }
    fn store(&mut self) -> Result<Store, JournalError> {
        self.validate()?;
        if let Some(store) = &self.store {
            store.records().map_err(storage)?;
            return Ok(store.clone());
        }
        let store = Store::open_in(
            self.parent.try_clone().map_err(storage)?,
            &self.path,
            media::Limits::default(),
            Budget::process(),
        )
        .map_err(storage)?;
        self.validate()?;
        self.store = Some(store.clone());
        Ok(store)
    }
}
pub(crate) fn storage(error: impl std::fmt::Display) -> JournalError {
    JournalError::Storage(error.to_string())
}
pub(crate) fn invalid(message: &str) -> JournalError {
    JournalError::NotRollbackable(message.into())
}
pub(crate) fn schema(conn: &rusqlite::Connection) -> Result<(), JournalError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS native_tag_batches(batch_id INTEGER PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,expected_count INTEGER NOT NULL,plan_digest BLOB NOT NULL,intent_digest BLOB NOT NULL); CREATE TABLE IF NOT EXISTS native_tag_schema(id INTEGER PRIMARY KEY CHECK(id=1),signature TEXT NOT NULL); CREATE TABLE IF NOT EXISTS native_tag_files(file_change_id INTEGER PRIMARY KEY REFERENCES file_changes(id) ON DELETE CASCADE,batch_id INTEGER NOT NULL REFERENCES batches(id) ON DELETE CASCADE,ordinal INTEGER NOT NULL,identity BLOB NOT NULL,operation_id TEXT NOT NULL,roots_digest BLOB NOT NULL,digest BLOB NOT NULL,intents BLOB NOT NULL,operation INTEGER NOT NULL DEFAULT 0,record BLOB,receipts BLOB,recovery BLOB,recovery_role INTEGER NOT NULL DEFAULT 0,done INTEGER NOT NULL DEFAULT 0,UNIQUE(batch_id,ordinal));")?;
    if !super::column_exists(conn, "native_tag_files", "recovery_role")? {
        conn.execute_batch(
            "ALTER TABLE native_tag_files ADD COLUMN recovery_role INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    Ok(())
}
pub(crate) fn marked_path(path: &Path) -> Vec<u8> {
    let mut bytes = PATH_MAGIC.to_vec();
    bytes.extend(path.to_string_lossy().as_bytes());
    bytes
}
pub(crate) fn root_digest(roots: &[PathBuf]) -> [u8; 32] {
    let mut paths: Vec<_> = roots
        .iter()
        .map(|p| p.as_os_str().as_encoded_bytes())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    let mut h = Sha256::new();
    for path in paths {
        h.update((path.len() as u64).to_le_bytes());
        h.update(path);
    }
    h.finalize().into()
}
pub(crate) fn file_digest(change: &FileChange) -> [u8; 32] {
    let mut h = Sha256::new();
    let mut text = |s: &[u8]| {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s);
    };
    text(change.path.as_os_str().as_encoded_bytes());
    for field in &change.tag_changes {
        text(field.field.to_storage_key().as_bytes());
        for value in [&field.old, &field.new] {
            match value {
                Some(value) => {
                    text(b"some");
                    text(value.as_bytes())
                }
                None => text(b"none"),
            }
        }
    }
    h.finalize().into()
}
pub(crate) fn pack(parts: &[Vec<u8>]) -> Result<Vec<u8>, JournalError> {
    let len = parts
        .iter()
        .try_fold(4usize, |n, p| n.checked_add(4 + p.len()))
        .filter(|n| *n <= LIMIT)
        .ok_or_else(|| invalid("native payload limit"))?;
    if parts.is_empty() || parts.len() > 64 {
        return Err(invalid("native field count"));
    }
    let mut out = Vec::with_capacity(len);
    out.extend((parts.len() as u32).to_le_bytes());
    for p in parts {
        out.extend((p.len() as u32).to_le_bytes());
        out.extend(p);
    }
    Ok(out)
}
pub(crate) fn unpack(mut bytes: &[u8]) -> Result<Vec<&[u8]>, JournalError> {
    fn number(bytes: &mut &[u8]) -> Result<usize, JournalError> {
        if bytes.len() < 4 {
            return Err(invalid("native payload truncated"));
        }
        let n = u32::from_le_bytes(bytes[..4].try_into().map_err(storage)?) as usize;
        *bytes = &bytes[4..];
        Ok(n)
    }
    if bytes.len() > LIMIT {
        return Err(invalid("native payload limit"));
    }
    let count = number(&mut bytes)?;
    if count == 0 || count > 64 {
        return Err(invalid("native field count"));
    }
    let mut parts = Vec::with_capacity(count);
    for _ in 0..count {
        let len = number(&mut bytes)?;
        let part = bytes
            .get(..len)
            .ok_or_else(|| invalid("native payload extent"))?;
        parts.push(part);
        bytes = &bytes[len..];
    }
    if !bytes.is_empty() {
        return Err(invalid("native payload suffix"));
    }
    Ok(parts)
}
pub(crate) fn plan_digest(batch: &AppliedBatch) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"tagrex-native-plan-v2");
    hash.update((batch.plan.changes.len() as u64).to_le_bytes());
    hash.update(root_digest(&batch.allowed_roots));
    for (ordinal, change) in batch.plan.changes.iter().enumerate() {
        hash.update((ordinal as u64).to_le_bytes());
        hash.update(file_digest(change));
    }
    hash.finalize().into()
}
pub(crate) fn intent_digest<'a>(intents: impl Iterator<Item = &'a Intent>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"tagrex-native-intents-v2");
    for (ordinal, intent) in intents.enumerate() {
        hash.update((ordinal as u64).to_le_bytes());
        hash.update(intent.identity);
        hash.update((intent.bytes.len() as u64).to_le_bytes());
        hash.update(&intent.bytes);
    }
    hash.finalize().into()
}
pub(crate) fn check_batch_header(
    conn: &rusqlite::Connection,
    batch: &AppliedBatch,
) -> Result<(), JournalError> {
    let digest: Vec<u8> = conn.query_row("SELECT CASE WHEN length(plan_digest)=32 THEN plan_digest END FROM native_tag_batches WHERE batch_id=?1", [batch.id.0], |r|r.get(0))?;
    if super::native_batch_count(conn, batch.id.0)? != Some(batch.plan.changes.len())
        || digest.as_slice() != plan_digest(batch)
    {
        return Err(invalid("native immutable batch collection changed"));
    }
    Ok(())
}

pub(crate) fn load_entries(
    conn: &rusqlite::Connection,
    batch: &AppliedBatch,
) -> Result<Entries, JournalError> {
    check_batch_header(conn, batch)?;
    let(count,bytes):(i64,i64)=conn.query_row("SELECT count(*),coalesce(sum(length(intents)+coalesce(length(record),0)+coalesce(length(receipts),0)+coalesce(length(recovery),0)+512),0) FROM native_tag_files WHERE batch_id=?1",[batch.id.0],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count as usize != batch.plan.changes.len()
        || count < 1
        || count as usize > ROWS
        || bytes < 0
        || bytes as usize > 64 * 1024 * 1024
    {
        return Err(invalid("native collection bounds"));
    }
    let owner = Arc::new(
        admit(Charge {
            retained: bytes as usize + count as usize * 512,
            ..Default::default()
        })
        .map_err(storage)?,
    );
    let _work = admit(Charge {
        transient: bytes as usize + 8192,
        jobs: 1,
        ..Default::default()
    })
    .map_err(storage)?;
    let mut stmt=conn.prepare("SELECT n.ordinal,CASE WHEN length(n.identity)=32 THEN n.identity END,CASE WHEN length(n.digest)=32 THEN n.digest END,n.intents,n.operation,n.record,n.receipts,n.recovery,n.done,CASE WHEN length(CAST(n.operation_id AS BLOB))=34 THEN n.operation_id END,CASE WHEN length(CAST(f.path AS BLOB))<=8192 THEN f.path END,CASE WHEN length(n.roots_digest)=32 THEN n.roots_digest END,n.recovery_role FROM native_tag_files n JOIN file_changes f ON f.id=n.file_change_id AND f.batch_id=n.batch_id WHERE n.batch_id=?1 ORDER BY n.ordinal")?;
    let mut rows = stmt.query([batch.id.0])?;
    let mut entries = Vec::with_capacity(count as usize);
    while let Some(row) = rows.next()? {
        let ordinal = usize::try_from(row.get::<_, i64>(0)?).map_err(storage)?;
        if ordinal != entries.len() {
            return Err(invalid("native ordinal collection"));
        }
        let roots_digest: Vec<u8> = row.get(11)?;
        if roots_digest.as_slice() != root_digest(&batch.allowed_roots) {
            return Err(invalid("native root authorization binding"));
        }
        let identity: Vec<u8> = row.get(1)?;
        let digest: Vec<u8> = row.get(2)?;
        let bytes: Vec<u8> = row.get(3)?;
        let operation: u8 = row.get(4)?;
        let operation_id: String = row.get(9)?;
        let (path, marked) = super::stored_path(row.get_ref(10)?)?;
        if !marked || path != batch.plan.changes[ordinal].path {
            return Err(invalid("native row owner"));
        }
        let record: Option<Vec<u8>> = row.get(5)?;
        let receipts: Option<Vec<u8>> = row.get(6)?;
        let recovery: Option<Vec<u8>> = row.get(7)?;
        let done: bool = row.get(8)?;
        let recovery_role: u8 = row.get(12)?;
        let record = record
            .as_deref()
            .map(Record::from_journal)
            .transpose()
            .map_err(storage)?;
        let identity: [u8; 32] = identity
            .try_into()
            .map_err(|_| invalid("native source identity"))?;
        let digest: [u8; 32] = digest
            .try_into()
            .map_err(|_| invalid("native plan binding"))?;
        if operation > 2
            || (operation == 0 && recovery.is_some())
            || (recovery.is_none() != (recovery_role == 0))
            || recovery_role > 2
            || (operation == 1 && recovery_role > 1)
            || (operation == 2 && recovery.is_none())
            || digest != file_digest(&batch.plan.changes[ordinal])
            || record.as_ref().is_some_and(|r| {
                r.source_identity() != identity
                    || r.original_path() != batch.plan.changes[ordinal].path
                    || r.id() != operation_id
            })
        {
            return Err(invalid("native receipt binding"));
        }
        if record
            .as_ref()
            .is_some_and(|r| r.phase() >= media::Phase::Published && done != (operation != 1))
        {
            return Err(invalid("native outcome state"));
        }
        if operation_id.len() != 34
            || !operation_id.starts_with("m-")
            || !operation_id.as_bytes()[2..]
                .iter()
                .all(u8::is_ascii_hexdigit)
        {
            return Err(invalid("native operation identity"));
        }
        unpack(&bytes)?;
        if let Some(p) = &receipts {
            unpack(p)?;
        }
        if let Some(p) = &recovery {
            unpack(p)?;
        }
        entries.push(Entry {
            ordinal,
            intent: Intent {
                identity,
                operation_id,
                roots_digest: root_digest(&batch.allowed_roots),
                digest,
                bytes,
                _owner: owner.clone(),
            },
            operation,
            record,
            receipts,
            recovery,
            recovery_role,
            done,
        });
    }
    if entries.len() != count as usize {
        return Err(invalid("native row collection owner"));
    }
    let digest: Vec<u8> = conn.query_row("SELECT CASE WHEN length(intent_digest)=32 THEN intent_digest END FROM native_tag_batches WHERE batch_id=?1", [batch.id.0], |r| r.get(0))?;
    if digest.as_slice() != intent_digest(entries.iter().map(|e| &e.intent)) {
        return Err(invalid("native immutable intent collection changed"));
    }
    Ok(Entries {
        rows: entries,
        _owner: owner,
    })
}

/// A decoded, semantically bound row. This proof performs no source or Store IO.
/// Keeping this phase separate lets recovery reason about never-published rows
/// without treating a missing/foreign source as authority to skip journal data.
pub(crate) struct Proof {
    pub entry: Entry,
    pub intents: Vec<crate::model::native_patch::codec::StoredIntent>,
    pub receipts: Vec<crate::model::native_patch::Receipt>,
}
pub(crate) fn desired_state(
    field: &TagField,
    value: &Option<String>,
    policy: &crate::model::exact::ExactReadPolicy,
) -> Result<crate::model::exact::ExactTextState, JournalError> {
    use crate::{model::exact::ExactTextState, values::FieldValues};
    match value {
        None => Ok(ExactTextState::absent()),
        Some(value) => ExactTextState::present(
            if field.is_multi_value() {
                FieldValues::from_legacy_list(value, &policy.multi_value_separator)
            } else {
                FieldValues::new(vec![value.clone()])
            }
            .map_err(storage)?,
        )
        .map_err(storage),
    }
}
fn decode<T>(
    mut action: impl FnMut() -> Result<T, crate::model::native_patch::Error>,
) -> Result<T, JournalError> {
    let start = std::time::Instant::now();
    loop {
        match action() {
            Err(crate::model::native_patch::Error::Resource(
                crate::resources::AdmissionError::Busy,
            )) if start.elapsed() < std::time::Duration::from_millis(500) => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            result => return result.map_err(storage),
        }
    }
}
pub(crate) fn prove(
    batch: &AppliedBatch,
    entries: Entries,
    complete: bool,
) -> Result<Vec<Proof>, JournalError> {
    use crate::model::{
        exact::NativeTextTarget,
        native_patch::{codec, Receipt},
        AudioFormat,
    };
    use std::collections::HashSet;
    let mut out = Vec::with_capacity(entries.rows.len());
    let mut paths = HashSet::new();
    let mut identities = HashSet::new();
    for entry in entries.rows {
        let change = &batch.plan.changes[entry.ordinal];
        if !paths.insert(change.path.clone()) || !identities.insert(entry.intent.identity) {
            return Err(invalid("duplicate native source path or identity"));
        }
        if change.copy
            || change.rename_to.is_some()
            || !change.sidecar_renames.is_empty()
            || change.cover_change.is_some()
            || !change.block_changes.is_empty()
            || batch.plan.prune_empty_dirs
            || change.tag_changes.is_empty()
        {
            return Err(invalid("native history profile changed"));
        }
        let intent_bytes = unpack(&entry.intent.bytes)?;
        if intent_bytes.len() != change.tag_changes.len() {
            return Err(invalid("native intent count"));
        }
        let receipt_bytes = entry
            .receipts
            .as_deref()
            .map(unpack)
            .transpose()?
            .unwrap_or_default();
        if (entry
            .record
            .as_ref()
            .is_some_and(|r| r.phase() >= media::Phase::Prepared)
            || entry.done)
            && receipt_bytes.is_empty()
        {
            return Err(invalid("native selective receipt missing"));
        }
        if entry.operation == 0 && entry.record.is_none() && !receipt_bytes.is_empty() {
            return Err(invalid("native Apply receipts without preparation"));
        }
        if entry.operation > 0
            && entry
                .record
                .as_ref()
                .is_some_and(|r| r.phase() >= media::Phase::Prepared)
            && (entry.recovery.is_none() || entry.recovery_role != entry.operation)
        {
            return Err(invalid("native recovery receipt generation"));
        }
        if !receipt_bytes.is_empty() && receipt_bytes.len() != intent_bytes.len() {
            return Err(invalid("native receipt count"));
        }
        if complete
            && (entry.operation != 0
                || !entry.done
                || entry
                    .record
                    .as_ref()
                    .is_none_or(|r| r.phase() != media::Phase::Retired)
                || entry.recovery.is_some())
        {
            return Err(invalid(
                "native completion requires all durable Undo proofs",
            ));
        }
        let mut targets = HashSet::new();
        let mut intents = Vec::with_capacity(intent_bytes.len());
        let mut receipts = Vec::with_capacity(receipt_bytes.len());
        for (index, bytes) in intent_bytes.iter().enumerate() {
            let intent = decode(|| codec::decode_intent(bytes))?;
            let NativeTextTarget::Vorbis { name } = &intent.target else {
                return Err(invalid("native target profile"));
            };
            if intent
                .target
                .logical_field()
                .as_ref()
                .is_none_or(|field| !field.same_name(&change.tag_changes[index].field))
            {
                return Err(invalid("native target field binding"));
            }
            if !targets.insert(name.to_ascii_uppercase()) {
                return Err(invalid("duplicate stored native target"));
            }
            if desired_state(
                &change.tag_changes[index].field,
                &change.tag_changes[index].new,
                &intent.context.read_policy,
            )? != intent.new
            {
                return Err(invalid("native desired-state binding"));
            }
            if !receipt_bytes.is_empty() {
                let receipt = decode(|| Receipt::decode_for_journal(receipt_bytes[index]))?;
                if receipt.target() != &intent.target
                    || receipt.old() != &intent.old
                    || receipt.new_state() != &intent.new
                    || receipt.context().read_policy != intent.context.read_policy
                    || receipt.context().container != AudioFormat::Flac
                {
                    return Err(invalid("native receipt intent mismatch"));
                }
                receipts.push(receipt.rebind_current_policy().map_err(storage)?);
            }
            intents.push(intent);
        }
        if let Some(bytes) = &entry.recovery {
            let mut seen = HashSet::new();
            for bytes in unpack(bytes)? {
                let receipt = decode(|| Receipt::decode_for_journal(bytes))?
                    .rebind_current_policy()
                    .map_err(storage)?;
                let historical = receipts
                    .iter()
                    .find(|r| r.target() == receipt.target())
                    .ok_or_else(|| invalid("unapproved native recovery target"))?;
                let NativeTextTarget::Vorbis { name } = receipt.target() else {
                    return Err(invalid("native recovery profile"));
                };
                if !seen.insert(name.to_ascii_uppercase())
                    || receipt.context().container != AudioFormat::Flac
                    || (entry.recovery_role == 1 && receipt.new_state() != historical.old())
                    || (entry.recovery_role == 2 && receipt.old() != historical.old())
                {
                    return Err(invalid("native recovery receipt binding"));
                }
            }
        }
        out.push(Proof {
            entry,
            intents,
            receipts,
        });
    }
    Ok(out)
}

impl NativeJournal for SqliteJournal {
    fn store(&mut self) -> Result<Store, JournalError> {
        self.native_profile
            .as_mut()
            .ok_or_else(|| invalid("journal has no protected profile"))?
            .store()
    }
    fn record_native(
        &mut self,
        batch: &AppliedBatch,
        intents: &[Intent],
    ) -> Result<BatchId, JournalError> {
        self.record_with_native(batch, Some(intents))
    }
    fn entries(&self, batch: &AppliedBatch) -> Result<Entries, JournalError> {
        load_entries(&self.conn, batch)
    }
    fn begin(
        &mut self,
        batch: &AppliedBatch,
        ordinal: usize,
        operation: u8,
    ) -> Result<String, JournalError> {
        if operation == 0 || operation > 2 || ordinal >= batch.plan.changes.len() {
            return Err(invalid("native operation"));
        }
        let id = media::operation_id().map_err(storage)?;
        let tx = self.conn.transaction()?;
        let count=tx.execute("UPDATE native_tag_files SET operation=?3,operation_id=?4,record=NULL,recovery=CASE WHEN ?3=1 THEN NULL ELSE recovery END,recovery_role=CASE WHEN ?3=1 THEN 0 ELSE recovery_role END WHERE batch_id=?1 AND ordinal=?2 AND digest=?5 AND (?3=1 OR (operation IN(1,2) AND recovery IS NOT NULL AND recovery_role IN(1,2)))",params![batch.id.0,ordinal as i64,operation,id,file_digest(&batch.plan.changes[ordinal]).as_slice()])?;
        if count != 1 {
            return Err(invalid("native operation plan binding"));
        }
        tx.commit()?;
        Ok(id)
    }
    fn checkpoint(
        &mut self,
        batch: &AppliedBatch,
        position: (usize, u8),
        record: &Record,
        expected: &Intent,
        receipts: Option<&[u8]>,
        done: bool,
    ) -> Result<(), JournalError> {
        let (ordinal, operation) = position;
        if operation > 2 || ordinal >= batch.plan.changes.len() {
            return Err(invalid("native checkpoint ordinal"));
        }
        self.native_profile
            .as_ref()
            .ok_or_else(|| invalid("journal profile missing"))?
            .validate()?;
        let bytes = record.journal_bytes().map_err(storage)?;
        let _work = admit(Charge {
            transient: expected.bytes.len().saturating_mul(2).saturating_add(65536),
            jobs: 1,
            ..Default::default()
        })
        .map_err(storage)?;
        if let Some(receipts) = receipts {
            unpack(receipts)?;
        }
        let tx = self.conn.transaction()?;
        // Immutable intent/path/root binding is checked in the same transaction
        // as every acknowledgement, including the pre-publication barrier.
        let bound: i64 = tx.query_row("SELECT count(*) FROM native_tag_files n JOIN file_changes f ON f.id=n.file_change_id AND f.batch_id=n.batch_id WHERE n.batch_id=?1 AND n.ordinal=?2 AND n.identity=?3 AND n.digest=?4 AND n.roots_digest=?5 AND n.intents=?6 AND f.path=?7", params![batch.id.0, ordinal as i64, expected.identity.as_slice(), file_digest(&batch.plan.changes[ordinal]).as_slice(), root_digest(&batch.allowed_roots).as_slice(), expected.bytes, marked_path(&batch.plan.changes[ordinal].path)], |r| r.get(0))?;
        if bound != 1 || record.source_identity() != expected.identity {
            return Err(invalid("native immutable intent changed"));
        }
        let(identity,digest,old_operation,old_record,operation_id):(Vec<u8>,Vec<u8>,u8,Option<Vec<u8>>,String)=tx.query_row("SELECT CASE WHEN length(identity)=32 THEN identity END,CASE WHEN length(digest)=32 THEN digest END,operation,CASE WHEN length(record)<=8192 THEN record WHEN record IS NULL THEN NULL ELSE zeroblob(0) END,CASE WHEN length(CAST(operation_id AS BLOB))=34 THEN operation_id END FROM native_tag_files WHERE batch_id=?1 AND ordinal=?2",params![batch.id.0,ordinal as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        if record.id() != operation_id
            || identity.as_slice() != record.source_identity()
            || digest.as_slice() != file_digest(&batch.plan.changes[ordinal])
            || record.original_path() != batch.plan.changes[ordinal].path
        {
            return Err(invalid("native checkpoint binding"));
        }
        if old_operation == operation {
            if let Some(old) = old_record {
                let old = Record::from_journal(&old).map_err(storage)?;
                if old.id() != record.id() || old.phase() > record.phase() {
                    return Err(invalid("native checkpoint regression"));
                }
            }
        } else {
            return Err(invalid("native operation regression"));
        }
        if record.phase() >= media::Phase::Prepared && receipts.is_none() {
            return Err(invalid("native checkpoint needs selective receipt"));
        }
        tx.execute("UPDATE native_tag_files SET operation=?3,record=?4,receipts=CASE WHEN ?3=0 THEN ?5 ELSE receipts END,recovery=CASE WHEN ?3<>0 THEN ?5 ELSE recovery END,recovery_role=CASE WHEN ?3<>0 THEN ?3 ELSE 0 END,done=?6 WHERE batch_id=?1 AND ordinal=?2",params![batch.id.0,ordinal as i64,operation,bytes,receipts,done])?;
        tx.commit()?;
        Ok(())
    }
    fn confirm(
        &self,
        batch: &AppliedBatch,
        position: (usize, u8),
        record: &Record,
        expected: &Intent,
        receipts: &[u8],
    ) -> Result<(), JournalError> {
        let (ordinal, operation) = position;
        if ordinal >= batch.plan.changes.len() || record.phase() != media::Phase::Published {
            return Err(invalid("native durable confirmation"));
        }
        self.native_profile
            .as_ref()
            .ok_or_else(|| invalid("journal profile missing"))?
            .validate()?;
        let _work = admit(Charge {
            transient: receipts.len().saturating_add(65536),
            jobs: 1,
            ..Default::default()
        })
        .map_err(storage)?;
        let bytes = record.journal_bytes().map_err(storage)?;
        let count:i64=self.conn.query_row("SELECT count(*) FROM native_tag_files n JOIN file_changes f ON f.id=n.file_change_id AND f.batch_id=n.batch_id WHERE n.batch_id=?1 AND n.ordinal=?2 AND n.operation=?3 AND n.identity=?4 AND n.digest=?5 AND n.roots_digest=?6 AND n.operation_id=?7 AND n.record=?8 AND n.done=?9 AND (CASE WHEN ?3=0 THEN n.receipts ELSE n.recovery END)=?10 AND f.path=?11 AND n.intents=?12 AND n.recovery_role=CASE WHEN ?3=0 THEN 0 ELSE ?3 END",params![batch.id.0,ordinal as i64,operation,expected.identity.as_slice(),file_digest(&batch.plan.changes[ordinal]).as_slice(),root_digest(&batch.allowed_roots).as_slice(),record.id(),bytes,operation!=1,receipts,marked_path(&batch.plan.changes[ordinal].path),expected.bytes],|r|r.get(0))?;
        if count != 1 {
            return Err(invalid("native durable outcome was not confirmed"));
        }
        Ok(())
    }
    fn finish_native(&mut self, batch: &AppliedBatch) -> Result<(), JournalError> {
        self.finish_with_validation(batch.id, batch, |journal| {
            let persisted = UndoJournal::batch(journal, batch.id)?
                .ok_or(JournalError::UnknownBatch(batch.id))?;
            prove(&persisted, load_entries(&journal.conn, &persisted)?, true)?;
            Ok(())
        })
    }
    fn retain_native(&mut self, batch: &AppliedBatch) -> Result<(), JournalError> {
        UndoJournal::retain_interrupted(self, batch.id, batch)
    }
    fn remove_native(&mut self, id: BatchId) -> Result<(), JournalError> {
        UndoJournal::rollback(self, id)
    }
}

#[cfg(test)]
#[path = "../native_flac_storage_tests.rs"]
mod storage_tests;
