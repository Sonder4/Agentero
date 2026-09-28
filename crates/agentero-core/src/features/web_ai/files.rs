use crate::error::AppError;
use crate::paths::web_ai_scratch_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

pub const MAX_PDF_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
pub const SCRATCH_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Pdf,
    Image,
}

#[derive(Debug, Clone)]
pub struct PreparedAttachment {
    pub path: PathBuf,
    pub kind: AttachmentKind,
    pub size: u64,
    pub sha256: String,
}

pub fn prepare_attachment(
    path: &Path,
    kind: AttachmentKind,
) -> Result<PreparedAttachment, AppError> {
    cleanup_expired_scratch(SCRATCH_TTL);
    if contains_symlink(path) {
        return Err(AppError::message(
            "symbolic-link attachment paths are not allowed",
        ));
    }
    let path = path
        .canonicalize()
        .map_err(|_| AppError::message("attachment path does not exist"))?;
    if !path.is_file() {
        return Err(AppError::message("attachment is not a file"));
    }
    let metadata = fs::metadata(&path)?;
    let max = match kind {
        AttachmentKind::Pdf => MAX_PDF_BYTES,
        AttachmentKind::Image => MAX_IMAGE_BYTES,
    };
    if metadata.len() == 0 || metadata.len() > max {
        return Err(AppError::message(format!(
            "attachment size must be between 1 and {max} bytes"
        )));
    }
    let bytes = fs::read(&path)?;
    match kind {
        AttachmentKind::Pdf if !bytes.starts_with(b"%PDF-") => {
            return Err(AppError::message("file is not a PDF"));
        }
        AttachmentKind::Image if !looks_like_image(&bytes) => {
            return Err(AppError::message("file is not a supported image"));
        }
        _ => {}
    }
    let mut digest = Sha256::new();
    digest.update(&bytes);
    let sha256 = hex::encode(digest.finalize());
    let root = web_ai_scratch_dir().join(Uuid::new_v4().to_string());
    fs::create_dir_all(&root)?;
    let staged = root.join(path.file_name().unwrap_or_default());
    fs::copy(&path, &staged)?;
    Ok(PreparedAttachment {
        path: staged,
        kind,
        size: metadata.len(),
        sha256,
    })
}

/// Remove abandoned request directories.  Cleanup is deliberately best effort:
/// an inability to scan the cache must never make a user-selected attachment
/// fail after it has passed validation.
pub fn cleanup_expired_scratch(max_age: Duration) {
    let root = web_ai_scratch_dir();
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() > max_age {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn contains_symlink(path: &Path) -> bool {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let Ok(cwd) = std::env::current_dir() else {
            return true;
        };
        cwd.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff]) // JPEG
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validates_pdf_magic_and_stages_file() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("paper.pdf");
        fs::write(&input, b"%PDF-1.7\nbody").unwrap();
        let prepared = prepare_attachment(&input, AttachmentKind::Pdf).unwrap();
        assert_eq!(prepared.kind, AttachmentKind::Pdf);
        assert!(prepared.path.is_file());
        assert_eq!(prepared.sha256.len(), 64);
    }

    #[test]
    fn rejects_fake_pdf() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("paper.pdf");
        fs::write(&input, b"not a pdf").unwrap();
        assert!(prepare_attachment(&input, AttachmentKind::Pdf).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let outside = dir.path().join("outside.pdf");
        fs::write(&outside, b"%PDF-1.7\nbody").unwrap();
        let link = dir.path().join("link.pdf");
        symlink(&outside, &link).unwrap();
        assert!(prepare_attachment(&link, AttachmentKind::Pdf).is_err());
    }
}
