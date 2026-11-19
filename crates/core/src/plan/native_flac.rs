//! The first protected production profile: whole plans of in-place FLAC text.
use super::*;
use crate::{
    journal::native::{self, Entries, Entry, Intent, NativeJournal},
    media_mutation::{self as media, BoundMedia, Current, Phase, Record, StateSink, Store},
    model::{
        exact::{ExactTextState, NativeTextTarget, PreparedTextPatch},
        native_patch::{self, codec, Receipt},
        AudioFormat,
    },
    resources::{Budget, Charge, Lease},
};
use std::{fs::File, io, sync::Arc};

// Limit only the private synchronous IO phase. The Store holds one job while
// its native editor/ack needs another; admitting every outer job first would
// exhaust all worker slots before their children can run. Component leases and
// byte/job ceilings remain authoritative and unchanged.
static IO_PHASES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static IO_COMPLETED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IO_LONGEST_MICROS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
thread_local! { static IO_PHASE_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) fn private_phase_active() -> bool {
    IO_PHASE_HELD.with(|held| held.get())
}
struct IoPhase(
    std::time::Instant,
    std::marker::PhantomData<std::rc::Rc<()>>,
);
fn io_capacity(limits: crate::resources::Limits) -> usize {
    let bytes = native_patch::WORKING_BYTES + media::WORKING_BYTES;
    (limits.jobs / 2)
        .min(limits.transient / bytes)
        .min(limits.total / bytes)
}
impl IoPhase {
    fn enter(cancel: &AtomicBool) -> Result<Self, PlanError> {
        if IO_PHASE_HELD.with(|held| held.get()) {
            // A private sink can reenter the executor. Refuse immediately rather
            // than waiting on an outer phase this thread cannot finish yet.
            return Err(error(format!(
                "native I/O admission reentrancy: {}",
                crate::resources::AdmissionError::Busy
            )));
        }
        let capacity = io_capacity(Budget::process().limits());
        if capacity == 0 {
            return Err(error(crate::resources::AdmissionError::TooLarge));
        }
        let start = std::time::Instant::now();
        let completed_before = IO_COMPLETED.load(Ordering::Acquire);
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(PlanError::Cancelled);
            }
            let used = IO_PHASES.load(Ordering::Acquire);
            if used < capacity
                && IO_PHASES
                    .compare_exchange(used, used + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                IO_PHASE_HELD.with(|held| held.set(true));
                return Ok(Self(std::time::Instant::now(), std::marker::PhantomData));
            }
            if !wait_busy(start, cancel) {
                return Err(if cancel.load(Ordering::SeqCst) {
                    PlanError::Cancelled
                } else {
                    error(format!(
                        "native I/O admission deadline: {}; completed while waiting: {}; active: {}/{}; longest completed phase: {}us",
                        crate::resources::AdmissionError::Busy,
                        IO_COMPLETED.load(Ordering::Acquire).wrapping_sub(completed_before),
                        IO_PHASES.load(Ordering::Acquire),
                        capacity,
                        IO_LONGEST_MICROS.load(Ordering::Acquire),
                    ))
                });
            }
        }
    }
}
impl Drop for IoPhase {
    fn drop(&mut self) {
        IO_PHASE_HELD.with(|held| held.set(false));
        IO_LONGEST_MICROS.fetch_max(
            self.0.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            Ordering::AcqRel,
        );
        IO_PHASES.fetch_sub(1, Ordering::AcqRel);
        IO_COMPLETED.fetch_add(1, Ordering::AcqRel);
    }
}

struct Cancel<'a> {
    flag: &'a AtomicBool,
    publishing: bool,
    mutating: bool,
}
impl native_patch::Control for Cancel<'_> {
    fn event(&mut self, event: native_patch::Event) -> io::Result<()> {
        if event == native_patch::Event::BeforeMutation {
            self.mutating = true;
        }
        Ok(())
    }
    fn cancelled(&mut self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}
impl media::Observer for Cancel<'_> {
    fn event(&mut self, event: media::Event) -> io::Result<()> {
        if event == media::Event::StateDurable(Phase::Publishing) {
            self.publishing = true;
        }
        if !self.publishing && self.flag.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "native preparation cancelled",
            ));
        }
        Ok(())
    }
}
static NEVER: AtomicBool = AtomicBool::new(false);
fn wait_busy(start: std::time::Instant, cancel: &AtomicBool) -> bool {
    if cancel.load(Ordering::SeqCst) || start.elapsed() >= std::time::Duration::from_millis(500) {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(2));
    true
}
fn reserve(charge: Charge, cancel: &AtomicBool) -> Result<Lease, crate::resources::AdmissionError> {
    let start = std::time::Instant::now();
    loop {
        match Budget::process().try_acquire(charge) {
            Err(crate::resources::AdmissionError::Busy) if wait_busy(start, cancel) => continue,
            result => return result,
        }
    }
}
fn read_exact(
    file: File,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<crate::model::exact::ExactTrackRead, crate::model::exact::ExactReadError> {
    let start = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(crate::model::exact::ExactReadError::Cancelled);
        }
        let result = TagEngine::read_exact_file_with_cancel(file.try_clone()?, path, || {
            cancel.load(Ordering::SeqCst)
        });
        match result {
            Err(crate::model::exact::ExactReadError::Resource(
                crate::resources::AdmissionError::Busy,
            )) if wait_busy(start, cancel) => continue,
            Err(crate::model::exact::ExactReadError::Resource(
                crate::resources::AdmissionError::Busy,
            )) if cancel.load(Ordering::SeqCst) => {
                return Err(crate::model::exact::ExactReadError::Cancelled)
            }
            other => return other,
        }
    }
}
fn retry_native<T>(
    cancel: &AtomicBool,
    mut action: impl FnMut(&mut Cancel<'_>) -> Result<T, native_patch::Error>,
) -> Result<T, native_patch::Error> {
    let start = std::time::Instant::now();
    loop {
        let mut control = Cancel {
            flag: cancel,
            publishing: false,
            mutating: false,
        };
        let result = action(&mut control);
        match result {
            Err(native_patch::Error::Resource(crate::resources::AdmissionError::Busy))
                if !control.mutating && wait_busy(start, cancel) =>
            {
                continue
            }
            other => return other,
        }
    }
}
pub(super) struct Captured {
    patches: Vec<PreparedTextPatch>,
    intent: Intent,
}
fn error(error: impl std::fmt::Display) -> PlanError {
    native::storage(error).into()
}
fn media_error(stage: &str, failure: media::Error) -> PlanError {
    match failure {
        media::Error::Resource(failure) => {
            error(format!("native media {stage} admission: {failure}"))
        }
        failure => error(failure),
    }
}
fn exact_error(error: crate::model::exact::ExactReadError) -> PlanError {
    match error {
        crate::model::exact::ExactReadError::Cancelled => PlanError::Cancelled,
        crate::model::exact::ExactReadError::Resource(crate::resources::AdmissionError::Busy) => {
            self::error(format!(
                "native metadata read admission: {}",
                crate::resources::AdmissionError::Busy
            ))
        }
        error => self::error(error),
    }
}
fn recovery(path: &Path, error: impl std::fmt::Display) -> PlanError {
    PlanError::RecoveryRequired {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}
fn io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
fn target_name(field: &TagField) -> Option<String> {
    Some(
        match field {
            TagField::Title => "TITLE",
            TagField::Artist => "ARTIST",
            TagField::Album => "ALBUM",
            TagField::AlbumArtist => "ALBUMARTIST",
            TagField::Genre => "GENRE",
            TagField::Composer => "COMPOSER",
            TagField::Publisher => "PUBLISHER",
            TagField::Isrc => "ISRC",
            TagField::InitialKey => "INITIALKEY",
            TagField::CatalogNumber => "CATALOGNUMBER",
            TagField::Url => "WWW",
            TagField::MediaType => "MEDIA",
            TagField::Comment => "COMMENT",
            TagField::Custom(name) => return Some(name.clone()),
            _ => return None,
        }
        .into(),
    )
}
use native::desired_state as new_state;
pub(super) fn capture(
    plan: &ChangePlan,
    roots: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<Option<Vec<Captured>>, PlanError> {
    if plan.changes.is_empty()
        || plan.prune_empty_dirs
        || plan.changes.len() > 8192
        || plan.changes.iter().any(|c| {
            c.tag_changes.is_empty()
                || c.tag_changes.len() > 64
                || c.rename_to.is_some()
                || c.copy
                || !c.sidecar_renames.is_empty()
                || c.cover_change.is_some()
                || !c.block_changes.is_empty()
                || c.tag_changes
                    .iter()
                    .any(|f| target_name(&f.field).is_none())
        })
    {
        return Ok(None);
    }
    let mut captured = Vec::with_capacity(plan.changes.len());
    let mut paths = HashSet::new();
    let mut identities = HashSet::new();
    for change in &plan.changes {
        if cancel.load(Ordering::SeqCst) {
            return Err(PlanError::Cancelled);
        }
        ensure_within_roots(&change.path, roots)?;
        if !paths.insert(change.path.clone()) {
            return Err(native::invalid("duplicate native file intent").into());
        }
        let _step = pinned_paths::Step::enter([change.path.clone()]).map_err(PlanError::Io)?;
        let file = open_bound_file(&change.path, true, false).map_err(PlanError::Io)?;
        let source_len = file.metadata().map_err(PlanError::Io)?.len();
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut probe = file.try_clone().map_err(PlanError::Io)?;
            probe.seek(SeekFrom::Start(0)).map_err(PlanError::Io)?;
            let mut header = [0; 4];
            if probe.read_exact(&mut header).is_err() || &header != b"fLaC" {
                return Ok(None);
            }
            if source_len > media::Limits::default().media_bytes {
                return Ok(None);
            }
        }
        let identity = match media::file_identity(&file) {
            Ok(id) => id,
            Err(media::Error::Unsupported) => return Ok(None),
            Err(e) => return Err(error(e)),
        };
        if !identities.insert(identity) {
            return Err(native::invalid("duplicate native source identity").into());
        }
        let exact = read_exact(file, &change.path, cancel).map_err(exact_error)?;
        if exact.legacy().file.format != AudioFormat::Flac
            || exact.text().context().selected_block != Some(TagBlockKind::VorbisComments)
        {
            return Ok(None);
        }
        if exact.text().unavailable().is_some() {
            return Err(native::invalid("unavailable native FLAC inventory").into());
        }
        let mut patches = Vec::new();
        let mut targets = HashSet::new();
        let mut parts = Vec::new();
        let mut size = 4usize;
        let _work = reserve(
            Charge {
                transient: 64 * 1024 * 1024,
                jobs: 1,
                ..Default::default()
            },
            cancel,
        )
        .map_err(|error| self::error(format!("native capture working admission: {error}")))?;
        for field in &change.tag_changes {
            if exact.legacy().file.tags.get(&field.field) != field.old.as_ref() {
                return Err(PlanError::Stale(change.path.clone()));
            }
            let target = match exact
                .text()
                .resolve_field(&field.field)
                .map_err(|_| native::invalid("ambiguous native field"))?
            {
                Some(entry) => entry.target.clone(),
                None => NativeTextTarget::Vorbis {
                    name: target_name(&field.field)
                        .ok_or_else(|| native::invalid("unsupported native field"))?,
                },
            };
            let name = match &target {
                NativeTextTarget::Vorbis { name } => name.to_ascii_uppercase(),
                _ => return Err(native::invalid("not a native Vorbis field").into()),
            };
            if !targets.insert(name) {
                return Err(native::invalid("duplicate native target").into());
            }
            let patch = exact
                .text()
                .prepare_patch(
                    &target,
                    new_state(
                        &field.field,
                        &field.new,
                        &exact.text().context().read_policy,
                    )?,
                )
                .map_err(error)?;
            let encoded = retry_native(cancel, |_| codec::encode_intent(&patch)).map_err(error)?;
            size = size
                .checked_add(4 + encoded.bytes().len())
                .filter(|n| *n <= native::LIMIT)
                .ok_or_else(|| native::invalid("native intent aggregate limit"))?;
            parts.push(encoded.bytes().to_vec());
            patches.push(patch);
        }
        let maximum_growth = growth(patches.iter().map(|p| (p.target(), p.new_state())))?;
        if source_len
            .checked_add(maximum_growth)
            .is_none_or(|n| n > media::Limits::default().media_bytes)
        {
            return Err(native::invalid("native candidate capacity").into());
        }
        let owner = reserve(
            Charge {
                retained: size + 256,
                ..Default::default()
            },
            cancel,
        )
        .map_err(|error| self::error(format!("native intent retention admission: {error}")))?;
        let bytes = native::pack(&parts)?;
        captured.push(Captured {
            patches,
            intent: Intent {
                identity,
                roots_digest: native::root_digest(roots),
                operation_id: media::operation_id().map_err(error)?,
                digest: native::file_digest(change),
                bytes,
                _owner: Arc::new(owner),
            },
        });
    }
    Ok(Some(captured))
}
pub(super) fn revalidate(
    plan: &ChangePlan,
    captured: &[Captured],
    cancel: &AtomicBool,
) -> Result<(), PlanError> {
    for (change, captured) in plan.changes.iter().zip(captured) {
        let _step = pinned_paths::Step::enter([change.path.clone()]).map_err(PlanError::Io)?;
        let original = bound(change, &captured.intent.identity)?;
        let fresh = read_exact(
            original.held_file().map_err(PlanError::Io)?,
            &change.path,
            cancel,
        )
        .map_err(exact_error)?;
        if captured.patches.iter().any(|p| {
            fresh.text().context() != p.context()
                || fresh.text().state(p.target()).as_ref().ok() != Some(p.old())
        }) {
            return Err(PlanError::Stale(change.path.clone()));
        }
    }
    Ok(())
}
fn bound(change: &FileChange, identity: &[u8; 32]) -> Result<BoundMedia, PlanError> {
    let file = open_bound_file(&change.path, true, false).map_err(PlanError::Io)?;
    if &media::file_identity(&file).map_err(error)? != identity {
        return Err(PlanError::Stale(change.path.clone()));
    }
    BoundMedia::from_file(file, &change.path).map_err(error)
}
struct Driver<'a> {
    journal: &'a mut dyn NativeJournal,
    batch: &'a AppliedBatch,
    ordinal: usize,
    operation: u8,
    receipts: Option<Vec<u8>>,
    expected: &'a Intent,
    _receipt_owner: Lease,
    policy_revision: u64,
}
impl StateSink for Driver<'_> {
    fn persist(&mut self, record: &Record) -> io::Result<()> {
        ensure_operation_policy().map_err(io_error)?;
        if record.phase() == Phase::Publishing
            && crate::model::read_policy_revision() != self.policy_revision
        {
            return Err(io::Error::other("native publication policy changed"));
        }
        self.journal
            .checkpoint(
                self.batch,
                (self.ordinal, self.operation),
                record,
                self.expected,
                self.receipts.as_deref(),
                record.phase() >= Phase::Published && self.operation != 1,
            )
            .map_err(io_error)?;
        ensure_operation_policy().map_err(io_error)
    }
}
fn push_bundle(parts: &mut Vec<Vec<u8>>, bytes: &[u8], owner: &mut Lease) -> io::Result<()> {
    let total = parts
        .iter()
        .try_fold(4usize, |n, p| n.checked_add(4 + p.len()))
        .and_then(|n| n.checked_add(4 + bytes.len()))
        .filter(|n| *n <= native::LIMIT)
        .ok_or_else(|| io::Error::other("native receipt aggregate limit"))?;
    owner
        .try_resize(Charge {
            retained: total
                .checked_mul(2)
                .and_then(|n| n.checked_add(4096))
                .ok_or_else(|| io::Error::other("native receipt capacity"))?,
            ..Default::default()
        })
        .map_err(io_error)?;
    parts.push(bytes.to_vec());
    Ok(())
}
fn verify_finish(driver: &mut Driver<'_>, record: &Record) -> io::Result<()> {
    driver
        .journal
        .confirm(
            driver.batch,
            (driver.ordinal, driver.operation),
            record,
            driver.expected,
            driver
                .receipts
                .as_deref()
                .ok_or_else(|| io::Error::other("native receipt missing"))?,
        )
        .map_err(io_error)?;
    ensure_operation_policy().map_err(io_error)
}
fn growth<'a>(
    values: impl Iterator<Item = (&'a NativeTextTarget, &'a ExactTextState)>,
) -> Result<u64, PlanError> {
    let mut bytes = 64u64;
    for (target, state) in values {
        let NativeTextTarget::Vorbis { name } = target else {
            return Err(native::invalid("native growth target").into());
        };
        if let Some(values) = state.values() {
            for value in values.as_slice() {
                bytes = bytes
                    .checked_add((name.len() + value.len() + 5) as u64)
                    .filter(|n| *n <= 16 * 1024 * 1024)
                    .ok_or_else(|| native::invalid("native metadata growth limit"))?;
            }
        }
    }
    Ok(bytes)
}
fn apply_file(
    store: &Store,
    journal: &mut dyn NativeJournal,
    batch: &AppliedBatch,
    ordinal: usize,
    captured: &Captured,
    cancel: &AtomicBool,
) -> Result<(), PlanError> {
    let change = &batch.plan.changes[ordinal];
    let _phase = IoPhase::enter(cancel)?;
    let original = bound(change, &captured.intent.identity)?;
    let mut driver = Driver {
        journal,
        batch,
        ordinal,
        operation: 0,
        expected: &captured.intent,
        policy_revision: captured.patches[0].context().policy_revision,
        receipts: None,
        _receipt_owner: Budget::process()
            .try_acquire(Charge::default())
            .map_err(error)?,
    };
    let mut observed = Cancel {
        flag: cancel,
        publishing: false,
        mutating: false,
    };
    let prepared = store
        .prepare_file_id(
            (&captured.intent.operation_id, "flac", cancel),
            original,
            growth(captured.patches.iter().map(|p| (p.target(), p.new_state())))?,
            &mut driver,
            &mut observed,
            |candidate, driver| {
                let mut encoded = Vec::new();
                for (at, patch) in captured.patches.iter().enumerate() {
                    if cancel.load(Ordering::SeqCst) {
                        return Err(io::Error::other("native editing cancelled"));
                    }
                    let fresh = read_exact(candidate.try_clone()?, &change.path, cancel)
                        .map_err(io_error)?;
                    if at == 0 && fresh.text().context() != patch.context() {
                        return Err(io::Error::other("native initial context changed"));
                    }
                    if fresh.text().state(patch.target()).map_err(io_error)? != *patch.old() {
                        return Err(io::Error::other("native old state changed"));
                    }
                    let current = fresh
                        .text()
                        .prepare_patch(patch.target(), patch.new_state().clone())
                        .map_err(io_error)?;
                    let receipt = retry_native(cancel, |control| {
                        native_patch::apply_prepared(candidate, &current, control)
                    })
                    .map_err(io_error)?;
                    let bytes =
                        retry_native(cancel, |_| receipt.encode_for_journal()).map_err(io_error)?;
                    push_bundle(&mut encoded, bytes.bytes(), &mut driver._receipt_owner)?;
                }
                let final_read =
                    read_exact(candidate.try_clone()?, &change.path, cancel).map_err(io_error)?;
                for patch in &captured.patches {
                    if final_read.text().state(patch.target()).map_err(io_error)?
                        != *patch.new_state()
                    {
                        return Err(io::Error::other("native composed candidate state"));
                    }
                }
                driver.receipts = Some(native::pack(&encoded).map_err(io_error)?);
                Ok(())
            },
        )
        .map_err(|e| {
            if cancel.load(Ordering::SeqCst) && !observed.publishing {
                PlanError::Cancelled
            } else {
                media_error("preparation", e)
            }
        })?;
    let published = prepared.publish(&mut driver, &mut observed).map_err(|e| {
        if cancel.load(Ordering::SeqCst) && !observed.publishing {
            PlanError::Cancelled
        } else {
            recovery(&change.path, e)
        }
    })?;
    // The durable per-file publication and rollback material are already stored.
    // Check their actual persisted representation before full-image retirement.
    let record = published.record().clone();
    verify_finish(&mut driver, &record).map_err(|e| recovery(&change.path, e))?;
    published
        .retire(|_| Ok(()), &mut driver, &mut observed)
        .map_err(|e| recovery(&change.path, e))
}
pub(super) fn apply(
    mut batch: AppliedBatch,
    captured: Vec<Captured>,
    journal: &mut dyn NativeJournal,
    progress: &mut dyn FnMut(&ApplyProgress),
    cancel: &AtomicBool,
) -> Result<AppliedBatch, PlanError> {
    let rollback_policy = crate::model::RecoveryPolicy::capture()
        .ok_or_else(|| error("native operation policy changed before admission"))?;
    batch.native_tags = true;
    // NativeJournal takes a contiguous slice without cloning charged payloads.
    // Move intent owners out while the independently charged prepared patches stay.
    let (patches, intents): (Vec<_>, Vec<_>) =
        captured.into_iter().map(|c| (c.patches, c.intent)).unzip();
    batch.id = journal.record_native(&batch, &intents)?;
    let store = match ensure_operation_policy().and_then(|_| {
        let store = journal.store()?;
        ensure_operation_policy()?;
        Ok(store)
    }) {
        Ok(store) => store,
        Err(error) => {
            let _rollback_policy = rollback_policy.enter();
            let _ = journal.remove_native(batch.id);
            return Err(error);
        }
    };
    let mut steps = Steps::new(batch.plan.changes.len() * 2, progress, cancel);
    steps.done = batch.plan.changes.len();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), PlanError> {
            for (ordinal, (patches, intent)) in patches.into_iter().zip(intents).enumerate() {
                let change = &batch.plan.changes[ordinal];
                let _step =
                    pinned_paths::Step::enter([change.path.clone()]).map_err(PlanError::Io)?;
                steps.begin(&change.path)?;
                ensure_within_roots(&change.path, &batch.allowed_roots)?;
                apply_file(
                    &store,
                    journal,
                    &batch,
                    ordinal,
                    &Captured { patches, intent },
                    cancel,
                )?;
                batch
                    .applied_values
                    .extend(applied_values_at(change, &change.path));
                steps.complete(&change.path, ApplyStep::Tags);
                ensure_operation_policy()?;
            }
            ensure_operation_policy()?;
            batch.interrupted = false;
            journal.finish_native(&batch)?;
            ensure_operation_policy()?;
            Ok(())
        }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(cause)) => {
            let _rollback_policy = rollback_policy.enter();
            batch.interrupted = true;
            let _ = journal.retain_native(&batch);
            return match undo(&batch, journal, UndoMode::KeepChanged, &mut |_| {}, true) {
                Ok(_) => Err(cause),
                Err(PlanError::RollbackIncomplete { count, first, .. }) => {
                    Err(PlanError::RollbackIncomplete {
                        cause: Box::new(cause),
                        count,
                        first,
                    })
                }
                Err(_) => Err(recovery(&batch.plan.changes[0].path, cause)),
            };
        }
        Err(payload) => {
            let _rollback_policy = rollback_policy.enter();
            batch.interrupted = true;
            let _ = journal.retain_native(&batch);
            let _ = undo(&batch, journal, UndoMode::KeepChanged, &mut |_| {}, true);
            std::panic::resume_unwind(payload)
        }
    }
    Ok(batch)
}

struct Validated {
    entry: Entry,
    receipts: Vec<Receipt>,
    original: BoundMedia,
    selected: Vec<bool>,
    overwrite: bool,
    pending: Option<Record>,
    preimage: bool,
}
fn full_fingerprint(file: &mut File) -> Result<media::Fingerprint, PlanError> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    let before = file.metadata().map_err(PlanError::Io)?;
    if before.len() > 8 << 30 {
        return Err(native::invalid("native recovery media length").into());
    }
    file.seek(SeekFrom::Start(0)).map_err(PlanError::Io)?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    let mut len = 0;
    loop {
        let n = file.read(&mut bytes).map_err(PlanError::Io)?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > 8 << 30 {
            return Err(native::invalid("native recovery media length").into());
        }
        hash.update(&bytes[..n]);
    }
    let after = file.metadata().map_err(PlanError::Io)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(native::invalid("native recovery source changed").into());
    }
    Ok(media::Fingerprint {
        len,
        sha256: hash.finalize().into(),
    })
}
fn validated(
    batch: &AppliedBatch,
    entries: Entries,
    mode: UndoMode,
    rollback: bool,
) -> Result<(Vec<Validated>, Vec<Entry>, UndoReport), PlanError> {
    // Prove the entire durable collection before attempting source-dependent
    // decisions. A source failure never excuses malformed later journal data.
    let proofs = native::prove(batch, entries, false)?;
    let mut out = Vec::new();
    let mut unreached = Vec::new();
    let mut report = UndoReport::default();
    for native::Proof {
        entry,
        intents,
        receipts,
    } in proofs
    {
        let change = &batch.plan.changes[entry.ordinal];
        let pristine = entry.operation == 0
            && entry.record.is_none()
            && entry.receipts.is_none()
            && entry.recovery.is_none()
            && !entry.done;
        let source = (|| -> Result<_, PlanError> {
            let original = bound(change, &entry.intent.identity)?;
            let exact = read_exact(
                original.held_file().map_err(PlanError::Io)?,
                &change.path,
                &NEVER,
            )
            .map_err(error)?;
            Ok((original, exact))
        })();
        let (original, exact) = match source {
            Ok(source) => source,
            Err(_) if rollback && pristine => {
                // There was no durable permission for this operation to publish.
                // Preserve a deleted, foreign or unreadable source, while keeping
                // its operation ID for the whole Store check before any restore.
                report
                    .kept
                    .extend(change.tag_changes.iter().map(|field| KeptField {
                        path: change.path.clone(),
                        field: field.field.clone(),
                    }));
                unreached.push(entry);
                continue;
            }
            Err(cause) => return Err(cause),
        };
        let mut selected = Vec::with_capacity(intents.len());
        for (index, intent) in intents.iter().enumerate() {
            let current = exact.text().state(&intent.target).map_err(error)?;
            let choose = if receipts.is_empty() {
                if current != intent.old {
                    if !matches!(mode, UndoMode::KeepChanged) {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()));
                    }
                    report.kept.push(KeptField {
                        path: change.path.clone(),
                        field: change.tag_changes[index].field.clone(),
                    });
                }
                false
            } else if current == *receipts[index].old() {
                false
            } else if current == *receipts[index].new_state() {
                true
            } else {
                match mode {
                    UndoMode::Strict => {
                        return Err(PlanError::ChangedSinceApplied(change.path.clone()))
                    }
                    UndoMode::KeepChanged => {
                        report.kept.push(KeptField {
                            path: change.path.clone(),
                            field: change.tag_changes[index].field.clone(),
                        });
                        false
                    }
                    UndoMode::Overwrite => true,
                }
            };
            selected.push(choose);
        }
        out.push(Validated {
            entry,
            receipts,
            original,
            selected,
            pending: None,
            preimage: false,
            overwrite: matches!(mode, UndoMode::Overwrite),
        });
    }
    Ok((out, unreached, report))
}
fn attach_records(
    batch: &AppliedBatch,
    validated: &mut [Validated],
    unreached: &[Entry],
    store: &Store,
) -> Result<(), PlanError> {
    let records = store.records().map_err(error)?;
    for entry in unreached {
        if records.iter().any(|r| r.id() == entry.intent.operation_id) {
            return Err(recovery(
                &batch.plan.changes[entry.ordinal].path,
                "unbound protected preparation requires retained-evidence recovery",
            ));
        }
    }
    for valid in validated {
        let change = &batch.plan.changes[valid.entry.ordinal];
        valid.pending = valid
            .entry
            .record
            .as_ref()
            .and_then(|record| records.iter().find(|r| r.id() == record.id()))
            .cloned();
        if valid.entry.record.is_none()
            && records
                .iter()
                .any(|r| r.id() == valid.entry.intent.operation_id)
        {
            return Err(recovery(
                &change.path,
                "unbound protected preparation requires retained-evidence recovery",
            ));
        }
        valid.preimage = false;
        if let Some(record) = &valid.pending {
            if record.source_identity() != valid.entry.intent.identity
                || record.original_path() != change.path
            {
                return Err(native::invalid("native store/intent binding").into());
            }
            if record.phase() == Phase::Preparing {
                return Err(recovery(&change.path, "incomplete protected preparation"));
            }
            // Retired records may have partially removed images; finishing their
            // terminal cleanup uses its stronger tombstone protocol below.
            if record.phase() != Phase::Retired {
                match store
                    .inspect(record.id(), &mut valid.original)
                    .map_err(error)?
                {
                    Current::Preimage => valid.preimage = true,
                    Current::Candidate if record.phase() == Phase::Published => {}
                    _ => return Err(recovery(&change.path, "ambiguous protected publication")),
                }
            }
        } else if let Some(record) = &valid.entry.record {
            if record.phase() < Phase::Retired {
                if full_fingerprint(&mut valid.original.held_file().map_err(PlanError::Io)?)?
                    != record.before()
                {
                    return Err(recovery(&change.path, "missing protected evidence"));
                }
                valid.preimage = true;
            }
        }
        if valid.preimage && valid.entry.operation == 0 {
            valid.selected.fill(false);
        }
    }
    Ok(())
}
fn retire_pending(
    store: &Store,
    journal: &mut dyn NativeJournal,
    batch: &AppliedBatch,
    mut valid: Validated,
) -> Result<Validated, PlanError> {
    if let Some(record) = valid.pending.take() {
        let _phase = IoPhase::enter(&NEVER)?;
        if valid.preimage {
            store
                .discard_preimage(record.id(), valid.original)
                .map_err(error)?;
            valid.original = bound(
                &batch.plan.changes[valid.entry.ordinal],
                &valid.entry.intent.identity,
            )?;
        } else {
            let mut driver = Driver {
                journal,
                batch,
                ordinal: valid.entry.ordinal,
                operation: valid.entry.operation,
                expected: &valid.entry.intent,
                policy_revision: crate::model::read_policy_revision(),
                receipts: match valid.entry.operation {
                    0 => valid.entry.receipts.clone(),
                    _ => valid.entry.recovery.clone(),
                },
                _receipt_owner: Budget::process()
                    .try_acquire(Charge {
                        retained: valid.entry.receipts.as_ref().map_or(0, Vec::len)
                            + valid.entry.recovery.as_ref().map_or(0, Vec::len)
                            + 4096,
                        ..Default::default()
                    })
                    .map_err(error)?,
            };
            if record.phase() == Phase::Retired {
                store
                    .finish_retirement(record.id(), &mut valid.original, |_| Ok(()), &mut ())
                    .map_err(error)?;
            } else {
                // A local Published manifest plus a matching admitted source
                // witnesses the completed image even if its SQL ack was lost.
                driver.persist(&record).map_err(error)?;
                let published = store
                    .resume_published(record.id(), valid.original)
                    .map_err(error)?;
                let durable = published.record().clone();
                verify_finish(&mut driver, &durable).map_err(error)?;
                published
                    .retire(|_| Ok(()), &mut driver, &mut ())
                    .map_err(|e| media_error("retirement", e))?;
                valid.original = bound(
                    &batch.plan.changes[valid.entry.ordinal],
                    &valid.entry.intent.identity,
                )?;
            }
        }
    }
    Ok(valid)
}
fn undo_file(
    store: &Store,
    journal: &mut dyn NativeJournal,
    batch: &AppliedBatch,
    valid: Validated,
    operation: u8,
) -> Result<(), PlanError> {
    let change = &batch.plan.changes[valid.entry.ordinal];
    let _phase = IoPhase::enter(&NEVER)?;
    let mut driver = Driver {
        journal,
        batch,
        ordinal: valid.entry.ordinal,
        operation,
        expected: &valid.entry.intent,
        policy_revision: valid.receipts[0].context().policy_revision,
        receipts: None,
        _receipt_owner: Budget::process()
            .try_acquire(Charge::default())
            .map_err(error)?,
    };
    let receipts = valid.receipts;
    let selected = valid.selected;
    let overwrite = valid.overwrite;
    let id = driver
        .journal
        .begin(batch, valid.entry.ordinal, operation)?;
    let prepared = store
        .prepare_file_id(
            (&id, "flac", &NEVER),
            valid.original,
            growth(
                receipts
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| selected[*i])
                    .map(|(_, r)| (r.target(), r.old())),
            )?,
            &mut driver,
            &mut (),
            |candidate, driver| {
                let mut reverse = Vec::new();
                // Reverse order preserves the raw positional anchors from a composed
                // Apply; each restore leaves unrelated current records intact.
                for (index, receipt) in receipts.iter().enumerate().rev() {
                    if !selected[index] {
                        continue;
                    }
                    let fresh = read_exact(candidate.try_clone()?, &change.path, &NEVER)
                        .map_err(io_error)?;
                    let current = fresh.text().state(receipt.target()).map_err(io_error)?;
                    let reversed = retry_native(&NEVER, |control| {
                        native_patch::restore_flac_values(
                            candidate,
                            receipt,
                            fresh.text().context(),
                            &current,
                            overwrite,
                            control,
                        )
                    })
                    .map_err(io_error)?;
                    let bytes = retry_native(&NEVER, |_| reversed.encode_for_journal())
                        .map_err(io_error)?;
                    push_bundle(&mut reverse, bytes.bytes(), &mut driver._receipt_owner)?;
                }
                let final_read =
                    read_exact(candidate.try_clone()?, &change.path, &NEVER).map_err(io_error)?;
                for (index, receipt) in receipts.iter().enumerate() {
                    if selected[index]
                        && final_read
                            .text()
                            .state(receipt.target())
                            .map_err(io_error)?
                            != *receipt.old()
                    {
                        return Err(io::Error::other("native restored candidate state"));
                    }
                }
                driver.receipts = Some(native::pack(&reverse).map_err(io_error)?);
                Ok(())
            },
        )
        .map_err(|e| media_error("Undo preparation", e))?;
    let published = prepared
        .publish(&mut driver, &mut ())
        .map_err(|e| recovery(&change.path, e))?;
    let record = published.record().clone();
    verify_finish(&mut driver, &record).map_err(error)?;
    published
        .retire(|_| Ok(()), &mut driver, &mut ())
        .map_err(|e| media_error("retirement", e))
}
fn rollback_undo(
    batch: &AppliedBatch,
    journal: &mut dyn NativeJournal,
    store: &Store,
    completed: &[usize],
) -> Result<(), PlanError> {
    let mut entries = journal.entries(batch)?.rows;
    let records = store.records().map_err(error)?;
    let mut work = Vec::new();
    // Validate all recovery receipts before any earlier source can be changed.
    for ordinal in completed.iter().rev() {
        let at = entries
            .iter()
            .position(|e| e.ordinal == *ordinal)
            .ok_or_else(|| native::invalid("native rollback ordinal"))?;
        let entry = entries.remove(at);
        if entry.record.is_none() {
            continue;
        } // No Prepared SQL ack means publication could not begin.
        let bytes = entry
            .recovery
            .as_deref()
            .ok_or_else(|| native::invalid("native Undo rollback material missing"))?;
        let receipts = native::unpack(bytes)?
            .into_iter()
            .map(|b| {
                retry_native(&NEVER, |_| Receipt::decode_for_journal(b))
                    .and_then(Receipt::rebind_current_policy)
                    .map_err(error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        work.push((entry, receipts));
    }
    let mut failure = None;
    for (entry, receipts) in work {
        let change = &batch.plan.changes[entry.ordinal];
        let result = (|| -> Result<(), PlanError> {
            let mut original = bound(change, &entry.intent.identity)?;
            let record = entry
                .record
                .as_ref()
                .ok_or_else(|| native::invalid("native rollback record"))?;
            let pending = records.iter().find(|r| r.id() == record.id()).cloned();
            let preimage = full_fingerprint(&mut original.held_file().map_err(PlanError::Io)?)?
                == record.before();
            if preimage {
                if let Some(pending) = pending {
                    store
                        .discard_preimage(pending.id(), original)
                        .map_err(error)?;
                }
                return Ok(());
            }
            if let Some(pending) = &pending {
                if pending.phase() != Phase::Published && pending.phase() != Phase::Retired {
                    return Err(recovery(&change.path, "Undo publication remains ambiguous"));
                }
                if pending.phase() != Phase::Retired
                    && store.inspect(pending.id(), &mut original).map_err(error)?
                        != Current::Candidate
                {
                    return Err(recovery(&change.path, "Undo rollback image changed"));
                }
            }
            let selected = vec![true; receipts.len()];
            let valid = Validated {
                entry,
                receipts,
                original,
                selected,
                pending,
                preimage: false,
                overwrite: false,
            };
            let valid = retire_pending(store, journal, batch, valid)?;
            undo_file(store, journal, batch, valid, 2)
        })();
        if let Err(error) = result {
            if failure.is_none() {
                failure = Some(error);
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
fn recheck_strict_selection(batch: &AppliedBatch, valid: &Validated) -> Result<(), PlanError> {
    let change = &batch.plan.changes[valid.entry.ordinal];
    // The preflight capability already holds the exclusive source lock. Reopen
    // only for identity/readback; taking another BoundMedia would self-contend.
    let current = open_bound_file(&change.path, true, false).map_err(PlanError::Io)?;
    if media::file_identity(&current).map_err(error)? != valid.entry.intent.identity {
        return Err(PlanError::Stale(change.path.clone()));
    }
    let fresh = read_exact(current, &change.path, &NEVER).map_err(error)?;
    if valid.receipts.is_empty() {
        for bytes in native::unpack(&valid.entry.intent.bytes)? {
            let intent = retry_native(&NEVER, |_| codec::decode_intent(bytes)).map_err(error)?;
            if fresh.text().state(&intent.target).map_err(error)? != intent.old {
                return Err(PlanError::ChangedSinceApplied(change.path.clone()));
            }
        }
    } else {
        for (receipt, selected) in valid.receipts.iter().zip(&valid.selected) {
            let expected = if *selected {
                receipt.new_state()
            } else {
                receipt.old()
            };
            if fresh.text().state(receipt.target()).map_err(error)? != *expected {
                return Err(PlanError::ChangedSinceApplied(change.path.clone()));
            }
        }
    }
    Ok(())
}
pub(super) fn undo(
    batch: &AppliedBatch,
    journal: &mut dyn NativeJournal,
    mode: UndoMode,
    progress: &mut dyn FnMut(&ApplyProgress),
    rollback: bool,
) -> Result<UndoReport, PlanError> {
    let rollback_policy = crate::model::RecoveryPolicy::capture()
        .ok_or_else(|| error("native Undo policy changed before admission"))?;
    let entries = journal.entries(batch)?;
    ensure_operation_policy()?;
    let (mut validated, unreached, report) = validated(batch, entries, mode, rollback)?;
    let store = journal.store()?;
    ensure_operation_policy()?;
    attach_records(batch, &mut validated, &unreached, &store)?;
    let mut interrupted = batch.clone();
    interrupted.interrupted = true;
    journal.retain_native(&interrupted)?;
    let never = AtomicBool::new(false);
    let mut steps = Steps::new(batch.plan.changes.len(), progress, &never);
    let mut completed = Vec::new();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), PlanError> {
            for valid in validated {
                let ordinal = valid.entry.ordinal;
                let change = &batch.plan.changes[ordinal];
                let _step =
                    pinned_paths::Step::enter([change.path.clone()]).map_err(PlanError::Io)?;
                steps.begin(&change.path)?;
                if matches!(mode, UndoMode::Strict) {
                    recheck_strict_selection(batch, &valid)?;
                }
                let valid = retire_pending(&store, journal, batch, valid)?;
                if valid.selected.iter().any(|selected| *selected) {
                    // Include the attempted file: publication may have reached
                    // media before a later acknowledgement/retirement failed.
                    completed.push(ordinal);
                    undo_file(&store, journal, batch, valid, 1)?;
                }
                ensure_operation_policy()?;
                steps.done();
            }
            ensure_operation_policy()?;
            Ok(())
        }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(cause)) => {
            let _rollback_policy = rollback_policy.enter();
            return match rollback_undo(batch, journal, &store, &completed) {
                Ok(()) => Err(cause),
                Err(other) => Err(recovery(
                    &batch.plan.changes[0].path,
                    format!("{cause}; Undo rollback also failed: {other}"),
                )),
            };
        }
        Err(payload) => {
            let _rollback_policy = rollback_policy.enter();
            let _ = rollback_undo(batch, journal, &store, &completed);
            std::panic::resume_unwind(payload)
        }
    }
    if rollback && !report.kept.is_empty() {
        let first = report.kept[0].path.clone();
        let count = report
            .kept
            .iter()
            .map(|k| &k.path)
            .collect::<HashSet<_>>()
            .len();
        return Err(PlanError::RollbackIncomplete {
            cause: Box::new(PlanError::ChangedSinceApplied(first.clone())),
            count,
            first,
        });
    }
    if let Err(cause) = journal.remove_native(batch.id) {
        let _rollback_policy = rollback_policy.enter();
        return match rollback_undo(batch, journal, &store, &completed) {
            Ok(()) => Err(cause.into()),
            Err(other) => Err(recovery(
                &batch.plan.changes[0].path,
                format!("{cause}; Undo rollback also failed: {other}"),
            )),
        };
    }
    Ok(report)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn private_io_admission_derives_capacity_refuses_reentrancy_and_observes_cancel() {
        let mut limits = Budget::process().limits();
        assert_eq!(io_capacity(limits), 2);
        limits.jobs = 1;
        assert_eq!(io_capacity(limits), 0);
        let phase = IoPhase::enter(&NEVER).unwrap();
        assert!(IoPhase::enter(&NEVER).is_err());
        drop(phase);
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            IoPhase::enter(&cancelled),
            Err(PlanError::Cancelled)
        ));
        assert!(IoPhase::enter(&NEVER).is_ok());
    }
}
