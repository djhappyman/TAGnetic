//! Unix aliases reproduce the second canonical-parent probe Windows entry IO
//! performs internally. Original Windows process-death coverage stays public.
use super::*;
use std::os::unix::fs::symlink;

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    alias: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base = std::env::temp_dir().join(format!(
            "tagrex-optional-parent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join("root");
        let alias = base.join("alias");
        std::fs::create_dir(&root).unwrap();
        symlink(&root, &alias).unwrap();
        Self { base, root, alias }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn canonical_optional_alias_survives_absence_and_creation_consumes_all_spellings() {
    let f = Fixture::new();
    let logical = f.alias.join("missing/deep/file.bin");
    let canonical = f.root.join("missing/deep/file.bin");
    let _scope = Scope::enter(
        std::slice::from_ref(&f.root),
        [logical.clone(), f.alias.join("missing/peer.bin")],
    )
    .unwrap();
    assert_eq!(
        entry(&canonical).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    validate().expect("a second spelling of the admitted absent parent must remain optional");
    create_all(logical.parent().unwrap()).unwrap();
    ACTIVE.with(|slot| {
        let slot = slot.borrow();
        let authority = slot.as_ref().unwrap();
        assert!(!authority.optional.contains(&f.root.join("missing")));
        assert!(!authority.optional.contains(&f.root.join("missing/deep")));
    });
    validate().unwrap();
    std::fs::remove_dir_all(f.root.join("missing")).unwrap();
    assert!(
        validate().is_err(),
        "creation must consume absence authority across aliases"
    );
}

#[test]
fn unplanned_missing_parent_does_not_borrow_optional_authority() {
    let f = Fixture::new();
    let _scope = Scope::enter(
        std::slice::from_ref(&f.root),
        [f.alias.join("missing/file.bin")],
    )
    .unwrap();
    assert_eq!(
        entry(&f.root.join("unplanned/file.bin")).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(validate().is_err());
}

#[test]
fn optional_absence_cannot_hide_redirected_parent_authority() {
    let f = Fixture::new();
    let foreign = f.base.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("sentinel"), b"foreign bytes").unwrap();
    let _scope = Scope::enter(
        std::slice::from_ref(&f.root),
        [f.alias.join("missing/file.bin")],
    )
    .unwrap();
    std::fs::remove_file(&f.alias).unwrap();
    symlink(&foreign, &f.alias).unwrap();
    assert!(
        validate().is_err(),
        "absence must not bypass the recorded parent binding"
    );
    assert_eq!(
        std::fs::read(foreign.join("sentinel")).unwrap(),
        b"foreign bytes"
    );
}

#[test]
fn optional_absence_revalidates_existing_ancestor_identity() {
    for replace in [false, true] {
        let f = Fixture::new();
        let present = f.root.join("present");
        let saved = f.root.join("saved-present");
        std::fs::create_dir(&present).unwrap();
        std::fs::write(present.join("sentinel"), b"retained ancestor bytes").unwrap();
        let logical_missing = f.alias.join("present/missing");
        let _scope = Scope::enter(
            std::slice::from_ref(&f.root),
            [logical_missing.join("file.bin")],
        )
        .unwrap();
        std::fs::rename(&present, &saved).unwrap();
        if replace {
            std::fs::create_dir(&present).unwrap();
        }
        assert!(
            validate().is_err(),
            "an unchanged name cannot endorse a missing or replacement ancestor"
        );
        assert!(create_all(&logical_missing).is_err());
        assert_eq!(
            std::fs::read(saved.join("sentinel")).unwrap(),
            b"retained ancestor bytes"
        );
        if replace {
            assert!(std::fs::read_dir(&present).unwrap().next().is_none());
        } else {
            assert!(!present.exists(), "refusal must precede recreation");
        }
    }
}

#[test]
fn retiring_directory_removes_only_matching_optional_descendants() {
    let f = Fixture::new();
    let _scope = Scope::enter(
        std::slice::from_ref(&f.root),
        [
            f.alias.join("a/missing/file.bin"),
            f.alias.join("aa/missing/file.bin"),
        ],
    )
    .unwrap();
    create_all(&f.root.join("a")).unwrap();
    remove(&f.alias.join("a"), true).unwrap();
    ACTIVE.with(|slot| {
        let slot = slot.borrow();
        let authority = slot.as_ref().unwrap();
        assert!(!authority.optional.contains(&f.root.join("a/missing")));
        assert!(authority.optional.contains(&f.root.join("aa/missing")));
    });
    assert_eq!(
        entry(&f.root.join("aa/missing/file.bin")).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    validate().unwrap();
}

