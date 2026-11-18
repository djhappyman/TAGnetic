//! Portable reproduction of Darwin descriptor-duplication cursor semantics.
use super::*;
use std::io::{Read, Seek, SeekFrom};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tagrex-descriptor-io-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        // Direct private Scope callers must supply the same canonical roots as
        // public Executor admission (including Windows verbatim path prefixes).
        Self(std::fs::canonicalize(root).unwrap())
    }
    fn wav(&self) -> PathBuf {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(356u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(320u32.to_le_bytes());
        bytes.extend([0x31; 320]);
        let path = self.0.join("track.wav");
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn shared_description_at_eof_reproduces_lofty_format_failure() {
    use lofty::tag::{ItemKey, Tag, TagExt, TagType};
    let fixture = Fixture::new();
    let path = fixture.wav();
    let mut original = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    original.seek(SeekFrom::End(0)).unwrap();
    let mut duplicate = original.try_clone().unwrap();
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::TrackTitle, "Admitted title".into());
    assert!(tag.save_to(&mut duplicate, Default::default()).is_err());
    duplicate.rewind().unwrap();
    tag.save_to(&mut duplicate, Default::default()).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "Admitted title"
    );
}

#[test]
fn metadata_sessions_keep_logical_hints_and_independent_cursors() {
    let fixture = Fixture::new();
    let path = fixture.wav();
    let _paths =
        pinned_paths::Scope::enter(std::slice::from_ref(&fixture.0), [path.clone()]).unwrap();
    let _step = pinned_paths::Step::enter([path.clone()]).unwrap();
    with_bound_path(&path, true, |logical| {
        assert_eq!(
            logical, path,
            "descriptor names lose parser extension hints"
        );
        let mut first = open_bound_file(logical, true, false).unwrap();
        first.seek(SeekFrom::End(0)).unwrap();
        let end = first.stream_position().unwrap();
        let mut second = open_bound_file(logical, false, false).unwrap();
        assert_eq!(second.stream_position().unwrap(), 0);
        let mut magic = [0; 4];
        second.read_exact(&mut magic).unwrap();
        assert_eq!(&magic, b"RIFF");
        assert_eq!(first.stream_position().unwrap(), end);
        let mut track = TagEngine::read(logical)?;
        track
            .tags
            .insert(TagField::Title, "Independent sessions".into());
        TagEngine::write(&track)?;
        assert_eq!(
            TagEngine::read(logical)?.tags[&TagField::Title],
            "Independent sessions"
        );
        assert_eq!(first.stream_position().unwrap(), end);
        Ok(())
    })
    .unwrap();
}

#[test]
fn metadata_scope_unwind_restores_the_next_request() {
    let fixture = Fixture::new();
    let path = fixture.wav();
    let _paths =
        pinned_paths::Scope::enter(std::slice::from_ref(&fixture.0), [path.clone()]).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), PlanError> = with_bound_path(&path, false, |_| {
            panic!("owned metadata scope unwind");
        });
    }));
    assert!(result.is_err());
    assert!(METADATA_PATHS.with(|paths| paths.borrow().is_empty()));
    with_bound_path(&path, false, |logical| {
        assert_eq!(logical, path);
        TagEngine::read(logical)?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn exact_candidate_source_precedes_the_original_metadata_binding() {
    let fixture = Fixture::new();
    let original = fixture.wav();
    let candidate = fixture.0.join("candidate.wav");
    std::fs::copy(&original, &candidate).unwrap();
    let mut file = TagEngine::read(&candidate).unwrap();
    file.tags.insert(TagField::Title, "Candidate only".into());
    TagEngine::write(&file).unwrap();
    let original_bytes = std::fs::read(&original).unwrap();
    let _paths =
        pinned_paths::Scope::enter(std::slice::from_ref(&fixture.0), [original.clone()]).unwrap();
    let _step = pinned_paths::Step::enter([original.clone()]).unwrap();
    with_bound_path(&original, false, |logical| {
        let exact =
            TagEngine::read_exact_file(std::fs::File::open(&candidate).unwrap(), logical).unwrap();
        assert_eq!(exact.legacy().file.tags[&TagField::Title], "Candidate only");
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), original_bytes);
}
