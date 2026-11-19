//! Durable bounded task observations, never restored connections or write authority.
use crate::job_archive::{self, LogStore, Spool};
use crate::{Error, ErrorCode, Result, output::OutputLog};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use vibe_ssh_protocol::{
    DeploymentStepStatus, HistoryDetail, HistoryPage, HistoryRecord, JobStatus, LogPage, LogStream,
    MAX_LOG_READ, Owner,
};

pub(crate) const OUTPUT: usize = 64 * 1024;
struct Db {
    conn: Connection,
    _lock: File,
    logs: std::result::Result<LogStore, String>,
}

pub(crate) struct History {
    home: PathBuf,
    db: OnceLock<std::result::Result<Mutex<Db>, String>>,
    error: Mutex<Option<String>>,
}
fn failure(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Internal,
        format!("任务记录不可用：{error}。请保留数据目录并检查磁盘和应用版本。"),
    )
}
pub(crate) fn short(text: &str, max: usize) -> (String, bool) {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].into(), end < text.len())
}
impl History {
    pub fn new(home: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            home,
            db: OnceLock::new(),
            error: Mutex::new(None),
        })
    }
    pub fn status(&self) -> Value {
        json!({"initialized":self.db.get().is_some(),"error":self.error.lock().unwrap().clone().or_else(||self.db.get().and_then(|r|r.as_ref().err()).cloned())})
    }
    async fn call<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&mut Db) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let db = store.db.get_or_init(|| {
                Db::open(&store.home)
                    .map(Mutex::new)
                    .map_err(|e| e.to_string())
            });
            let db = db
                .as_ref()
                .map_err(|message| Error::new(ErrorCode::Internal, message.clone()))?;
            work(&mut db.lock().unwrap())
        })
        .await
        .map_err(failure)?
    }
    pub async fn save(
        self: &Arc<Self>,
        record: HistoryRecord,
        out: Vec<u8>,
        err: Vec<u8>,
    ) -> Result<()> {
        let result = self.call(move |db| db.save(&record, &out, &err)).await;
        *self.error.lock().unwrap() = result.as_ref().err().map(ToString::to_string);
        result
    }
    pub async fn spool(self: &Arc<Self>, id: String) -> Result<Spool> {
        self.call(move |db| {
            db.logs
                .as_ref()
                .map_err(failure)?
                .spool(&id)
                .map_err(failure)
        })
        .await
    }

    pub async fn log(
        self: &Arc<Self>,
        allowed: Option<BTreeSet<String>>,
        id: String,
        stream: LogStream,
        offset: u64,
        limit: usize,
    ) -> Result<LogPage> {
        if !(4..=MAX_LOG_READ).contains(&limit) {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "log read limit must be 4..=65536 bytes",
            ));
        }
        let (file, record) = self.open_log(allowed, id, stream).await?;
        let total = match stream {
            LogStream::Stdout => record.stdout_bytes,
            LogStream::Stderr => record.stderr_bytes,
        };
        let issue = record.archive.and_then(|archive| archive.issue);
        tokio::task::spawn_blocking(move || {
            job_archive::page(file, offset, limit, total, record.status, issue).map_err(failure)
        })
        .await
        .map_err(failure)?
    }

    pub async fn open_log(
        self: &Arc<Self>,
        allowed: Option<BTreeSet<String>>,
        id: String,
        stream: LogStream,
    ) -> Result<(File, HistoryRecord)> {
        let info = self.get(allowed, id.clone(), 0).await?.record;
        if info.archive.is_none() {
            return Err(Error::new(
                ErrorCode::NotFound,
                "this record only retains the output tail",
            ));
        }
        let home = self.home.clone();
        let file = tokio::task::spawn_blocking(move || {
            job_archive::open_log(&home, &id, stream).map_err(failure)
        })
        .await
        .map_err(failure)??;
        Ok((file, info))
    }
    pub async fn list(
        self: &Arc<Self>,
        allowed: Option<BTreeSet<String>>,
        offset: usize,
        limit: usize,
    ) -> Result<HistoryPage> {
        if !(1..=50).contains(&limit) || offset > 512 {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "history limit must be 1..=50, offset 0..=512",
            ));
        }
        self.call(move |db| {
            let mut statement = db
                .conn
                .prepare("SELECT meta FROM records ORDER BY started DESC,id DESC")
                .map_err(failure)?;
            let rows = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(failure)?;
            let mut records = Vec::new();
            for row in rows {
                let mut record: HistoryRecord =
                    serde_json::from_str(&row.map_err(failure)?).map_err(failure)?;
                if visible(&record, &allowed) {
                    let (command, cut) = short(record.command.as_deref().unwrap_or_default(), 512);
                    if record.command.is_some() {
                        record.command = Some(command);
                        record.command_truncated |= cut;
                    }
                    records.push(record);
                }
            }
            let more = records.len() > offset + limit;
            Ok(HistoryPage {
                records: records.into_iter().skip(offset).take(limit).collect(),
                more,
            })
        })
        .await
    }
    pub async fn get(
        self: &Arc<Self>,
        allowed: Option<BTreeSet<String>>,
        id: String,
        tail: usize,
    ) -> Result<HistoryDetail> {
        if id.len() != 33
            || !id.starts_with('h')
            || !id[1..].bytes().all(|b| b.is_ascii_hexdigit())
            || tail > vibe_ssh_protocol::MAX_READ
        {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                "invalid history id or tail budget",
            ));
        }
        self.call(move |db| {
            use rusqlite::OptionalExtension;
            let row = db
                .conn
                .query_row("SELECT meta,out,err FROM records WHERE id=?1", [id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })
                .optional()
                .map_err(failure)?;
            let missing = || Error::new(ErrorCode::NotFound, "no such retained task record");
            let (meta, out, err) = row.ok_or_else(missing)?;
            let mut record: HistoryRecord = serde_json::from_str(&meta).map_err(failure)?;
            if !visible(&record, &allowed) {
                return Err(missing());
            }
            for step in &mut record.deployment {
                let Some(id) = step.job_record.as_deref() else {
                    continue;
                };
                let child: Option<String> = db
                    .conn
                    .query_row("SELECT meta FROM records WHERE id=?1", [id], |row| {
                        row.get(0)
                    })
                    .optional()
                    .map_err(failure)?;
                let Some(child) = child else { continue };
                let child: HistoryRecord = serde_json::from_str(&child).map_err(failure)?;
                if visible(&child, &allowed) && child.host == record.host {
                    step.status = match child.status {
                        JobStatus::Running => DeploymentStepStatus::Running,
                        JobStatus::Succeeded => DeploymentStepStatus::Succeeded,
                        JobStatus::Failed => DeploymentStepStatus::Failed,
                        JobStatus::Unknown => DeploymentStepStatus::Unknown,
                    };
                    step.exit_code = child.exit_code;
                    step.reason = child.reason;
                }
            }
            if out.len() > OUTPUT
                || err.len() > OUTPUT
                || record.stdout_bytes < out.len() as u64
                || record.stderr_bytes < err.len() as u64
            {
                return Err(failure("invalid retained output lengths"));
            }
            let stdout = OutputLog::restored(
                out,
                record.stdout_bytes,
                record.status != JobStatus::Running,
            )
            .tail(tail.min(OUTPUT))
            .0;
            let stderr = OutputLog::restored(
                err,
                record.stderr_bytes,
                record.status != JobStatus::Running,
            )
            .tail(tail.min(OUTPUT))
            .0;
            Ok(HistoryDetail {
                record,
                stdout,
                stderr,
            })
        })
        .await
    }
}
fn visible(record: &HistoryRecord, allowed: &Option<BTreeSet<String>>) -> bool {
    allowed
        .as_ref()
        .is_none_or(|hosts| record.origin == Owner::Agent && hosts.contains(&record.host))
}
impl Db {
    fn open(home: &PathBuf) -> Result<Self> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(home)?;
        fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(home.join("history.lock"))?;
        lock.try_lock().map_err(failure)?;
        lock.set_permissions(fs::Permissions::from_mode(0o600))?;
        let path = home.join("history.sqlite");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        let mut conn = Connection::open(&path).map_err(failure)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(failure)?;
        let schema: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(failure)?;
        if schema != 0 && schema != 1 {
            return Err(failure(format!(
                "unsupported schema {schema}; no changes made"
            )));
        }
        if schema == 0 {
            let tables: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .map_err(failure)?;
            if tables != 0 {
                return Err(failure("unrecognized schema; no changes made"));
            }
            conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE records(id TEXT PRIMARY KEY,started INTEGER NOT NULL,terminal INTEGER NOT NULL,meta TEXT NOT NULL,out BLOB NOT NULL,err BLOB NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(failure)?;
        }
        conn.execute_batch("PRAGMA synchronous=FULL; PRAGMA journal_mode=DELETE;")
            .map_err(failure)?;
        let size: i64 = conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(failure)?;
        conn.pragma_update(None, "max_page_count", 96 * 1024 * 1024 / size)
            .map_err(failure)?;
        let tx = conn.transaction().map_err(failure)?;
        let unfinished: Vec<(String, String)> = {
            let mut statement = tx
                .prepare("SELECT id,meta FROM records WHERE terminal=0")
                .map_err(failure)?;
            statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(failure)?
                .collect::<rusqlite::Result<_>>()
                .map_err(failure)?
        };
        for (id, meta) in unfinished {
            let mut record: HistoryRecord = serde_json::from_str(&meta).map_err(failure)?;
            record.status = JobStatus::Unknown;
            record.reason = Some("client_restarted".into());
            record.exit_code = None;
            record.exit_signal = None;
            for step in &mut record.deployment {
                if step.status == DeploymentStepStatus::Running {
                    step.status = DeploymentStepStatus::Unknown;
                    step.reason = Some("client_restarted".into());
                } else if step.status == DeploymentStepStatus::Pending {
                    step.status = DeploymentStepStatus::Skipped;
                }
            }
            tx.execute(
                "UPDATE records SET terminal=1,meta=?2 WHERE id=?1",
                params![id, serde_json::to_string(&record).map_err(failure)?],
            )
            .map_err(failure)?;
        }
        tx.execute("DELETE FROM records WHERE terminal=1 AND id NOT IN (SELECT id FROM records WHERE terminal=1 ORDER BY started DESC,id DESC LIMIT 256)",[]).map_err(failure)?;
        tx.commit().map_err(failure)?;
        let retained = {
            let mut statement = conn.prepare("SELECT id FROM records").map_err(failure)?;
            statement
                .query_map([], |row| row.get(0))
                .map_err(failure)?
                .collect::<rusqlite::Result<BTreeSet<String>>>()
                .map_err(failure)?
        };
        let logs = LogStore::open(home, &retained).map_err(|error| error.to_string());
        Ok(Self {
            conn,
            _lock: lock,
            logs,
        })
    }
    fn save(&mut self, record: &HistoryRecord, out: &[u8], err: &[u8]) -> Result<()> {
        if out.len() > OUTPUT || err.len() > OUTPUT {
            return Err(failure("output budget exceeded"));
        }
        let tx = self.conn.transaction().map_err(failure)?;
        if record.status == JobStatus::Running {
            let active: i64 = tx
                .query_row(
                    "SELECT count(*) FROM records WHERE terminal=0 AND id!=?1",
                    [&record.record],
                    |r| r.get(0),
                )
                .map_err(failure)?;
            if active >= 256 {
                return Err(Error::new(
                    ErrorCode::ResourceLimit,
                    "too many unfinished task records; no remote command was submitted",
                ));
            }
        }
        tx.execute("INSERT INTO records(id,started,terminal,meta,out,err) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET terminal=excluded.terminal,meta=excluded.meta,out=excluded.out,err=excluded.err WHERE records.terminal=0 OR excluded.terminal=1",params![record.record,record.started_unix_ms as i64,record.status!=JobStatus::Running,serde_json::to_string(record).map_err(failure)?,out,err]).map_err(failure)?;
        let evicted = {
            let mut statement = tx.prepare("DELETE FROM records WHERE terminal=1 AND id NOT IN (SELECT id FROM records WHERE terminal=1 ORDER BY started DESC,id DESC LIMIT 256) RETURNING id").map_err(failure)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(failure)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(failure)?
        };
        tx.commit().map_err(failure)?;
        if let Ok(logs) = &self.logs {
            for id in evicted {
                logs.remove(&id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(origin: Owner, status: JobStatus) -> HistoryRecord {
        HistoryRecord {
            record: format!("h{}", uuid::Uuid::new_v4().simple()),
            host: "box".into(),
            host_name: "Box".into(),
            space_name: "task".into(),
            actor: "test".into(),
            origin,
            command: Some("echo test".into()),
            cwd: None,
            command_truncated: false,
            pty: false,
            started_unix_ms: 1,
            elapsed_ms: 1,
            status,
            reason: None,
            exit_code: None,
            exit_signal: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            transfer: None,
            deployment: Vec::new(),
            archive: None,
        }
    }
    #[tokio::test]
    async fn recovery_privacy_unicode_and_single_writer() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().to_path_buf();
        let store = History::new(path.clone());
        let mut job = record(Owner::Agent, JobStatus::Running);
        job.stdout_bytes = 1;
        store.save(job.clone(), vec![0xf0], vec![]).await.unwrap();
        let pending = store.get(None, job.record.clone(), 8192).await.unwrap();
        assert!(
            pending.stdout.text.is_empty() && !pending.stdout.invalid_utf8,
            "a running snapshot holds an incomplete UTF-8 suffix"
        );
        let output = "较早的输出🚀完成".repeat(10000).into_bytes();
        job.stdout_bytes = output.len() as u64;
        store
            .save(
                job.clone(),
                output[output.len() - OUTPUT..].to_vec(),
                vec![],
            )
            .await
            .unwrap();
        let personal = record(Owner::Human, JobStatus::Succeeded);
        store.save(personal.clone(), vec![], vec![]).await.unwrap();
        assert!(
            History::new(path.clone()).list(None, 0, 50).await.is_err(),
            "second writer cannot recover a live instance"
        );
        let allowed = Some(BTreeSet::from(["box".into()]));
        assert_eq!(
            store
                .list(allowed.clone(), 0, 50)
                .await
                .unwrap()
                .records
                .len(),
            1
        );
        assert!(
            store
                .get(allowed.clone(), personal.record, 8192)
                .await
                .is_err()
        );
        assert!(
            store
                .get(Some(BTreeSet::new()), job.record.clone(), 8192)
                .await
                .is_err()
        );
        let id = job.record.clone();
        drop(store);
        let recovered = History::new(path);
        let detail = recovered.get(allowed, id, OUTPUT).await.unwrap();
        assert_eq!(detail.record.status, JobStatus::Unknown);
        assert_eq!(detail.record.reason.as_deref(), Some("client_restarted"));
        assert!(detail.stdout.truncated);
        assert!(!detail.stdout.invalid_utf8);
        assert!(detail.stdout.text.ends_with("🚀完成"));
        assert_eq!(
            fs::metadata(home.path().join("history.sqlite"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn corrupt_future_schema_and_disk_full_do_not_overwrite_existing_records() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().to_path_buf();
        fs::write(path.join("history.sqlite"), b"not a database").unwrap();
        assert!(Db::open(&path).is_err());
        assert_eq!(
            fs::read(path.join("history.sqlite")).unwrap(),
            b"not a database"
        );
        fs::remove_file(path.join("history.sqlite")).unwrap();
        let connection = Connection::open(path.join("history.sqlite")).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        assert!(Db::open(&path).is_err());
        let connection = Connection::open(path.join("history.sqlite")).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            99
        );
        drop(connection);
        fs::remove_file(path.join("history.sqlite")).unwrap();
        let mut db = Db::open(&path).unwrap();
        let first = record(Owner::Agent, JobStatus::Succeeded);
        db.save(&first, &[], &[]).unwrap();
        let pages: i64 = db
            .conn
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        db.conn
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let mut second = record(Owner::Agent, JobStatus::Succeeded);
        second.stdout_bytes = OUTPUT as u64;
        assert!(db.save(&second, &vec![255; OUTPUT], &[]).is_err());
        assert_eq!(
            db.conn
                .query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1,
            "failed transaction retains original record"
        );
        db.conn.pragma_update(None, "query_only", true).unwrap();
        assert!(db.save(&second, &[], &[]).is_err());
    }
}
