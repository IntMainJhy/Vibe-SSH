use std::{path::Path, process::Stdio};

use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, Command},
};
use vibe_ssh_protocol::{ErrorCode, MAX_INPUT};

use crate::{Error, Result};

/// A local file or directory being packed by the system `tar` in the calling process, so the
/// files are read with the caller's own permissions and never with the service's.
pub struct Archive(Child);

impl Archive {
    /// A directory is packed as its contents, a file as itself.
    pub fn pack(local: &Path, excludes: &[String]) -> Result<Self> {
        let local = local
            .canonicalize()
            .map_err(|e| Error::new(ErrorCode::NotFound, format!("local path: {e}")))?;
        let (root, entry) = match (local.is_dir(), local.parent(), local.file_name()) {
            (true, ..) => (local.as_path(), ".".as_ref()),
            (false, Some(parent), Some(name)) => (parent, name),
            _ => {
                return Err(Error::new(
                    ErrorCode::InvalidRequest,
                    "local path must be a file or directory",
                ));
            }
        };
        let mut tar = Command::new("/usr/bin/tar");
        // Keep macOS resource forks and extended attributes out of the archive.
        tar.env("COPYFILE_DISABLE", "1")
            .args(["--no-xattrs", "--no-mac-metadata", "-cf", "-", "-C"])
            .arg(root);
        for pattern in excludes {
            tar.arg("--exclude").arg(pattern);
        }
        // ponytail: stderr is only read at the end; drain it concurrently if tar ever warns by the megabyte.
        tar.arg("--")
            .arg(entry)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map(Self)
            .map_err(|e| Error::new(ErrorCode::Internal, format!("local tar: {e}")))
    }

    /// Writes the archive to `out` as frames: a big-endian u32 length, then that many bytes.
    /// The closing zero length is written only after `tar` succeeded, so a stream that stops
    /// for any other reason is never mistaken for a complete upload.
    ///
    /// Fails only for local reasons. If the receiver stops reading, its reply says why.
    pub async fn send<W: AsyncWrite + Unpin>(mut self, out: &mut W) -> Result<()> {
        let local = |e: std::io::Error| Error::new(ErrorCode::Internal, format!("local tar: {e}"));
        let mut archive = self.0.stdout.take().expect("piped");
        let mut chunk = vec![0; MAX_INPUT];
        loop {
            let read = archive.read(&mut chunk).await.map_err(local)?;
            if read == 0 {
                break;
            }
            let frame = async {
                out.write_u32(read as u32).await?;
                out.write_all(&chunk[..read]).await
            };
            if frame.await.is_err() {
                return Ok(());
            }
        }
        drop(archive);
        let output = self.0.wait_with_output().await.map_err(local)?;
        if !output.status.success() {
            return Err(Error::new(
                ErrorCode::InvalidRequest,
                format!(
                    "local tar failed, the remote directory holds a partial copy: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        let _ = out.write_u32(0).await;
        let _ = out.flush().await;
        Ok(())
    }
}
