//! Private, bounded archive files. The in-memory live output remains independent.
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::io::{AsyncWriteExt, BufWriter};
use vibe_ssh_protocol::{JobStatus, LogArchive, LogPage, LogStream};

use crate::output::partial_suffix;

const STREAM_LIMIT: u64 = 64 * 1024 * 1024;
const STORE_LIMIT: u64 = 512 * 1024 * 1024;

pub(crate) struct LogStore {
    root: PathBuf,
    used: Arc<AtomicU64>,
}

pub(crate) fn suffix(stream: LogStream) -> &'static str {
    match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
    }
}

fn path(root: &Path, id: &str, stream: LogStream) -> PathBuf {
    root.join(format!("{id}.{}.raw", suffix(stream)))
}

fn record_id(id: &str) -> bool {
    id.len() == 33 && id.starts_with('h') && id[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl LogStore {
    pub(crate) fn open(home: &Path, retained: &BTreeSet<String>) -> io::Result<Self> {
        let root = home.join("logs");
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if !fs::symlink_metadata(&root)?.is_dir() {
            return Err(io::Error::other("log directory is not a plain directory"));
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        let mut used = 0_u64;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some((id, kind)) = name.split_once('.') else {
                continue;
            };
            if !record_id(id) || !matches!(kind, "stdout.raw" | "stderr.raw") {
                continue;
            }
            let meta = entry.metadata()?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if retained.contains(id) || fs::remove_file(entry.path()).is_err() {
                used = used.saturating_add(meta.len());
            }
        }
        Ok(Self {
            root,
            used: Arc::new(AtomicU64::new(used)),
        })
    }

    pub(crate) fn spool(&self, id: &str) -> io::Result<Spool> {
        let create = |stream| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path(&self.root, id, stream))
        };
        let out = create(LogStream::Stdout)?;
        let err = create(LogStream::Stderr).inspect_err(|_| {
            let _ = fs::remove_file(path(&self.root, id, LogStream::Stdout));
        })?;
        Ok(Spool {
            out: Writer::new(out),
            err: Writer::new(err),
            used: self.used.clone(),
        })
    }

    pub(crate) fn remove(&self, id: &str) {
        for stream in [LogStream::Stdout, LogStream::Stderr] {
            let path = path(&self.root, id, stream);
            if let Ok(metadata) = fs::symlink_metadata(&path)
                && metadata.is_file()
                && fs::remove_file(path).is_ok()
            {
                #[allow(deprecated, reason = "Rust 1.89 does not have AtomicU64::try_update")]
                let _ = self
                    .used
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                        Some(used.saturating_sub(metadata.len()))
                    });
            }
        }
    }
}

struct Writer {
    file: BufWriter<tokio::fs::File>,
    len: u64,
    issue: Option<String>,
}

impl Writer {
    fn new(file: File) -> Self {
        Self {
            file: BufWriter::with_capacity(64 * 1024, tokio::fs::File::from_std(file)),
            len: 0,
            issue: None,
        }
    }

    async fn write(&mut self, data: &[u8], used: &AtomicU64) {
        if self.issue.is_some() {
            return;
        }
        let desired = (STREAM_LIMIT - self.len).min(data.len() as u64);
        #[allow(deprecated, reason = "Rust 1.89 does not have AtomicU64::try_update")]
        let before = used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n + desired.min(STORE_LIMIT.saturating_sub(n)))
            })
            .expect("the reservation always returns a value");
        let size = desired.min(STORE_LIMIT.saturating_sub(before)) as usize;
        if let Err(error) = self.file.write_all(&data[..size]).await {
            self.issue = Some(format!("log write failed: {error}"));
            return;
        }
        self.len += size as u64;
        if size < data.len() {
            self.issue = Some("log archive size limit reached".into());
        }
    }

    async fn sync(&mut self, final_write: bool) {
        let result = async {
            self.file.flush().await?;
            if final_write {
                self.file.get_ref().sync_data().await?;
            }
            Ok::<(), io::Error>(())
        }
        .await;
        if let Err(error) = result {
            self.issue = Some(format!("log flush failed: {error}"));
            self.len = self
                .file
                .get_ref()
                .metadata()
                .await
                .map(|meta| meta.len())
                .unwrap_or(0)
                .min(self.len);
        }
    }
}

pub(crate) struct Spool {
    out: Writer,
    err: Writer,
    used: Arc<AtomicU64>,
}

impl Spool {
    pub(crate) async fn append(&mut self, stderr: bool, data: &[u8]) {
        let writer = if stderr { &mut self.err } else { &mut self.out };
        writer.write(data, &self.used).await;
    }

    pub(crate) async fn sync(&mut self, final_write: bool) -> LogArchive {
        tokio::join!(self.out.sync(final_write), self.err.sync(final_write));
        LogArchive {
            stdout_bytes: self.out.len,
            stderr_bytes: self.err.len,
            issue: self.out.issue.as_ref().or(self.err.issue.as_ref()).cloned(),
        }
    }
}

pub(crate) fn open_log(home: &Path, id: &str, stream: LogStream) -> io::Result<File> {
    let root = home.join("logs");
    if !record_id(id) || !fs::symlink_metadata(&root)?.is_dir() {
        return Err(io::Error::other("log is unavailable"));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path(&root, id, stream))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("log is unavailable"));
    }
    Ok(file)
}

pub(crate) fn page(
    mut file: File,
    mut offset: u64,
    limit: usize,
    observed: u64,
    status: JobStatus,
    issue: Option<String>,
) -> io::Result<LogPage> {
    let archived = file.metadata()?.len().min(STREAM_LIMIT);
    offset = offset.min(archived);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(limit);
    file.take((archived - offset).min(limit as u64))
        .read_to_end(&mut bytes)?;
    if offset + (bytes.len() as u64) < archived {
        bytes.truncate(bytes.len() - partial_suffix(&bytes));
    }
    Ok(LogPage {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        offset,
        next_offset: offset + bytes.len() as u64,
        archived_bytes: archived,
        observed_bytes: observed.max(archived),
        complete: matches!(status, JobStatus::Succeeded | JobStatus::Failed)
            && observed == archived
            && issue.is_none(),
        invalid_utf8: std::str::from_utf8(&bytes).is_err(),
        issue,
    })
}
