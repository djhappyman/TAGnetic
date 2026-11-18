//! Read-only local M3U/M3U8 resolution. Paths in a playlist never grant access.
//! The result is a proposed file list; it does not replace the live session,
//! staged edits or Undo history.
use super::*;
use crate::field_operations::invalid;
use tagrex_core::text_codec::{self, TextEncoding};

const MAX_ENTRIES: usize = 100_000;
const MAX_RESOLVED_BYTES: usize = 64 * 1024 * 1024;

struct ApprovedRoot {
    lexical: PathBuf,
    canonical: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaylistLoadOptions {
    /// Explicit codepage for M3U; M3U8 always requires UTF-8.
    pub encoding: String,
}
impl Default for PlaylistLoadOptions {
    fn default() -> Self {
        Self {
            encoding: "utf-8".into(),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistIssueKind {
    Duplicate,
    Missing,
    OutsideScope,
    UnsafePath,
    UnsupportedReference,
    UnsupportedExtension,
    Unreadable,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaylistIssue {
    pub line: usize,
    pub reference: String,
    pub kind: PlaylistIssueKind,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaylistLoadPreview {
    pub paths: Vec<PathBuf>,
    pub issues: Vec<PlaylistIssue>,
}

impl App {
    /// Resolve local entries in first-occurrence order within caller-approved
    /// roots. Does not load files into the app or mutate the current stage.
    pub fn preview_playlist_load(
        &self,
        playlist: &Path,
        allowed_roots: &[PathBuf],
        options: &PlaylistLoadOptions,
    ) -> Result<PlaylistLoadPreview, AppError> {
        self.preview_playlist_load_with_cancel(playlist, allowed_roots, options, || false)
    }

    pub fn preview_playlist_load_with_cancel(
        &self,
        playlist: &Path,
        allowed_roots: &[PathBuf],
        options: &PlaylistLoadOptions,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlaylistLoadPreview, AppError> {
        let check_cancel = || -> Result<(), AppError> {
            if cancelled() {
                Err(tagrex_core::plan::PlanError::Cancelled.into())
            } else {
                Ok(())
            }
        };
        check_cancel()?;
        let ext = playlist
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !["m3u", "m3u8"]
            .iter()
            .any(|suffix| ext.eq_ignore_ascii_case(suffix))
        {
            return Err(invalid("Choose a local M3U or M3U8 playlist."));
        }
        let encoding =
            TextEncoding::from_label(&options.encoding).map_err(|e| invalid(e.to_string()))?;
        if ext.eq_ignore_ascii_case("m3u8") && encoding != TextEncoding::Utf8 {
            return Err(invalid("M3U8 playlists require UTF-8."));
        }
        if allowed_roots.is_empty() || allowed_roots.len() > 256 {
            return Err(invalid("Choose between 1 and 256 approved playlist roots."));
        }
        let roots = allowed_roots
            .iter()
            .map(|root| {
                check_cancel()?;
                let canonical = root.canonicalize()?;
                if !canonical.is_dir() {
                    return Err(invalid("Playlist roots must be directories."));
                }
                let absolute = if root.is_absolute() {
                    root.clone()
                } else {
                    std::env::current_dir()?.join(root)
                };
                let lexical = normalize(&absolute)
                    .map_err(|_| invalid("Playlist root has an unsafe path."))?;
                Ok(ApprovedRoot { lexical, canonical })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let parent = playlist
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = parent.canonicalize()?;
        let filename = playlist
            .file_name()
            .ok_or_else(|| invalid("Playlist has no filename."))?;
        let mut file = output::read_under(&parent, Path::new(filename))?;
        let bytes = read_text_list_bytes(&mut file, playlist)?;
        let text = text_codec::decode_with_cancel(
            &bytes,
            encoding,
            TEXT_LIST_MAX_BYTES as usize,
            &cancelled,
        )
        .map_err(|error| match error {
            text_codec::TextError::Cancelled => tagrex_core::plan::PlanError::Cancelled.into(),
            other => invalid(other.to_string()),
        })?;
        if text.contains('\0') {
            return Err(invalid("Playlists cannot contain NUL."));
        }
        if text
            .lines()
            .any(|line| line.trim_start().starts_with("#EXT-X-"))
        {
            return Err(invalid(
                "Streaming/HLS playlists are not local music playlists.",
            ));
        }
        let mut preview = PlaylistLoadPreview {
            paths: Vec::new(),
            issues: Vec::new(),
        };
        let mut seen = HashSet::new();
        let mut total_bytes = 0usize;
        let mut entries = 0usize;
        for (index, line) in text.lines().enumerate() {
            check_cancel()?;
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(invalid("Playlists are limited to 100000 entries."));
            }
            let resolved = resolve(line, &parent, &roots);
            match resolved {
                Ok(path) if seen.insert(path.clone()) => {
                    // The result and deduplication index both own this path.
                    total_bytes = total_bytes
                        .saturating_add(path.capacity().saturating_mul(2))
                        .saturating_add(128);
                    preview.paths.push(path);
                }
                other => {
                    total_bytes = total_bytes.saturating_add(line.len()).saturating_add(96);
                    let kind = match other {
                        Ok(_) => PlaylistIssueKind::Duplicate,
                        Err(kind) => kind,
                    };
                    preview.issues.push(PlaylistIssue {
                        line: index + 1,
                        reference: line.into(),
                        kind,
                    });
                }
            }
            if total_bytes > MAX_RESOLVED_BYTES {
                return Err(invalid(
                    "Resolved playlist metadata exceeds its 64 MiB budget.",
                ));
            }
        }
        check_cancel()?;
        Ok(preview)
    }
}

fn resolve(
    reference: &str,
    base: &Path,
    roots: &[ApprovedRoot],
) -> Result<PathBuf, PlaylistIssueKind> {
    use PlaylistIssueKind as Issue;
    if reference.len() > 4096 || reference.chars().any(char::is_control) {
        return Err(Issue::UnsafePath);
    }
    // Treat backslashes as portable playlist separators, never escape syntax.
    // Do not reinterpret a Windows drive/UNC or URI as a relative Unix filename.
    #[cfg(not(windows))]
    if reference.starts_with(['\\'])
        || reference.starts_with("//")
        || reference.as_bytes().get(1) == Some(&b':')
    {
        return Err(Issue::UnsupportedReference);
    }
    if let Some((scheme, _)) = reference.split_once(':') {
        let uri = !scheme.is_empty()
            && scheme.as_bytes()[0].is_ascii_alphabetic()
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b));
        #[cfg(windows)]
        let uri = uri && scheme.len() != 1;
        if uri {
            return Err(Issue::UnsupportedReference);
        }
    }
    let path = PathBuf::from(reference.replace('\\', "/"));
    let joined = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let normalized = normalize(&joined)?;
    if normalized.to_str().is_none() {
        return Err(Issue::UnsafePath);
    }
    if !scanner::is_supported_audio(&normalized) {
        return Err(Issue::UnsupportedExtension);
    }
    // Accept both spellings of an explicitly approved root (for example,
    // macOS /var and /private/var). Resolve its suffix beneath the canonical
    // root; do not canonicalize arbitrary entries before the no-follow check.
    let (root, relative) = roots
        .iter()
        .find_map(|root| {
            normalized
                .strip_prefix(&root.canonical)
                .or_else(|_| normalized.strip_prefix(&root.lexical))
                .ok()
                .map(|relative| (&root.canonical, relative))
        })
        .ok_or(Issue::OutsideScope)?;
    // A pinned, regular, no-follow input prevents final links and devices from
    // being accepted as media. Escaping parent links fail confinement here.
    let _file = output::read_under(root, relative).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Issue::Missing,
        std::io::ErrorKind::PermissionDenied => Issue::OutsideScope,
        _ => Issue::Unreadable,
    })?;
    let canonical = root
        .join(relative)
        .canonicalize()
        .map_err(|_| Issue::Unreadable)?;
    if !roots
        .iter()
        .any(|root| canonical.starts_with(&root.canonical))
    {
        return Err(Issue::OutsideScope);
    }
    Ok(canonical)
}

fn normalize(path: &Path) -> Result<PathBuf, PlaylistIssueKind> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(PlaylistIssueKind::UnsafePath);
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}
