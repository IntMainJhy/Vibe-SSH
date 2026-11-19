//! Caller-side metadata and selected content. Never run on behalf of the Agent socket.
use crate::{Error, ErrorCode, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use vibe_ssh_protocol::{Manifest, ManifestEntry};

pub const ENTRIES: usize = 1000;
pub const PATH_BYTES: usize = 64 * 1024;
pub const FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const HASH_BYTES: u64 = 32 * 1024 * 1024;
pub const DEPTH: usize = 16;
pub const DEFAULT_EXCLUDES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".ssh",
    ".env*",
    "*.pem",
    "*.key",
    "*.p12",
];

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidRequest, message)
}
pub fn rules(extra: &[String]) -> Result<Vec<String>> {
    let mut rules: BTreeSet<String> = DEFAULT_EXCLUDES.iter().map(|s| (*s).into()).collect();
    if extra.len() > 64 {
        return Err(invalid("at most 64 exclude rules"));
    }
    for rule in extra {
        if rule.is_empty()
            || rule.len() > 128
            || rule.contains(['/', '\0'])
            || glob::Pattern::new(rule).is_err()
        {
            return Err(invalid(
                "exclude rules must be component globs, at most 128 bytes, without / or NUL",
            ));
        }
        rules.insert(rule.clone());
    }
    if rules.len() > 64 {
        return Err(invalid("at most 64 effective exclude rules"));
    }
    Ok(rules.into_iter().collect())
}
pub fn excluded(path: &str, rules: &[String]) -> bool {
    path.split('/').any(|part| {
        rules
            .iter()
            .any(|rule| glob::Pattern::new(rule).is_ok_and(|p| p.matches(part)))
    })
}
pub fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains('\0')
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
pub fn validate(manifest: &Manifest) -> Result<()> {
    rules(&manifest.excludes)?;
    if manifest.entries.len() > ENTRIES
        || manifest.entries.iter().map(|e| e.path.len()).sum::<usize>() > PATH_BYTES
    {
        return Err(invalid("manifest exceeds directory preview budget"));
    }
    let mut seen = BTreeSet::new();
    let mut bytes = 0;
    for entry in &manifest.entries {
        if !relative(&entry.path)
            || entry.path.split('/').count() > DEPTH
            || !seen.insert(&entry.path)
            || !["file", "directory", "symlink", "other"].contains(&entry.kind.as_str())
        {
            return Err(invalid("invalid or duplicate manifest path/kind"));
        }
        if let Some(digest) = &entry.digest {
            let size = entry
                .size
                .ok_or_else(|| invalid("digest requires file size"))?;
            bytes += size.min(HASH_BYTES + 1);
            if entry.kind != "file"
                || entry.excluded
                || excluded(&entry.path, &rules(&manifest.excludes)?)
                || size > FILE_BYTES
                || bytes > HASH_BYTES
                || digest.len() != 64
                || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("invalid or over-budget selected digest"));
            }
        }
    }
    Ok(())
}

/// Metadata only, including excluded roots. Never descend into symlinks.
pub fn scan(root: &Path, extra: &[String]) -> Result<Manifest> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(invalid("choose a real directory"));
    }
    let mut result = Manifest {
        entries: vec![],
        excludes: rules(extra)?,
        truncated: false,
    };
    let mut pending = vec![(String::new(), 0)];
    let mut bytes = 0;
    'scan: while let Some((parent, depth)) = pending.pop() {
        // Reject changed directory links before enumeration as well.
        let dir = root.join(&parent);
        if !fs::symlink_metadata(&dir)?.is_dir() {
            return Err(invalid("directory changed during scan"));
        }
        for item in fs::read_dir(dir)? {
            let item = item?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| invalid("non-UTF-8 path cannot be previewed"))?;
            let path = if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            };
            if result.entries.len() == ENTRIES || bytes + path.len() > PATH_BYTES || depth >= DEPTH
            {
                result.truncated = true;
                break 'scan;
            }
            bytes += path.len();
            let meta = fs::symlink_metadata(item.path())?;
            let kind = if meta.is_dir() {
                "directory"
            } else if meta.is_file() {
                "file"
            } else if meta.is_symlink() {
                "symlink"
            } else {
                "other"
            };
            let ignore = excluded(&path, &result.excludes);
            if kind == "directory" && !ignore {
                pending.push((path.clone(), depth + 1));
            }
            result.entries.push(ManifestEntry {
                path,
                kind: kind.into(),
                size: meta.is_file().then_some(meta.len()),
                digest: None,
                excluded: ignore,
            });
        }
    }
    result.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

/// Hash only explicit selections from the metadata list, checking ancestor links.
pub fn select(root: &Path, manifest: &Manifest, selected: &[String]) -> Result<Manifest> {
    validate(manifest)?;
    if selected.len() > ENTRIES {
        return Err(invalid("too many selected paths"));
    }
    let mut result = manifest.clone();
    for e in &mut result.entries {
        e.digest = None;
    }
    let mut budget = HASH_BYTES;
    for path in selected.iter().collect::<BTreeSet<_>>() {
        let entry = result
            .entries
            .iter_mut()
            .find(|e| &e.path == path)
            .ok_or_else(|| invalid("selection was not in the preview"))?;
        if entry.excluded || excluded(path, &result.excludes) || entry.kind != "file" {
            return Err(invalid("cannot read an ignored, linked or special path"));
        }
        if entry.size.is_none_or(|n| n > FILE_BYTES || n > budget) {
            continue;
        }
        let Some(data) = read_selected(root, path, budget.min(FILE_BYTES))? else {
            continue;
        };
        budget -= data.len() as u64;
        entry.size = Some(data.len() as u64);
        entry.digest = Some(format!("{:x}", Sha256::digest(&data)));
    }
    Ok(result)
}

pub(crate) fn read_selected(root: &Path, path: &str, limit: u64) -> Result<Option<Vec<u8>>> {
    if !relative(path) || !fs::symlink_metadata(root)?.is_dir() {
        return Err(invalid("selection root or path changed"));
    }
    let mut current = root.to_path_buf();
    if let Some((parent, _)) = path.rsplit_once('/') {
        for part in parent.split('/') {
            current.push(part);
            if !fs::symlink_metadata(&current)?.is_dir() {
                return Err(invalid("selection ancestor changed or became a link"));
            }
        }
    }
    let full = root.join(path);
    let before = fs::symlink_metadata(&full)?;
    if !before.is_file() {
        return Err(invalid("selected file changed or became a link"));
    }
    if before.len() > limit {
        return Ok(None);
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&full)?;
    let opened = file.metadata()?;
    if !same(&before, &opened) {
        return Err(invalid("selected file changed while opening"));
    }
    let mut data = Vec::new();
    (&file).take(before.len()).read_to_end(&mut data)?;
    if !same(&opened, &file.metadata()?)
        || !same(&opened, &fs::symlink_metadata(&full)?)
        || data.len() as u64 != opened.len()
    {
        return Err(invalid("selected file changed while reading"));
    }
    Ok(Some(data))
}
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.is_file()
        && b.is_file()
        && (
            a.dev(),
            a.ino(),
            a.len(),
            a.mtime(),
            a.mtime_nsec(),
            a.ctime(),
            a.ctime_nsec(),
        ) == (
            b.dev(),
            b.ino(),
            b.len(),
            b.mtime(),
            b.mtime_nsec(),
            b.ctime(),
            b.ctime_nsec(),
        )
}
