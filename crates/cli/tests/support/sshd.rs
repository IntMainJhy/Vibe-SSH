//! A throwaway OpenSSH server on loopback with temporary keys, shared by end-to-end tests.
//! It never reads the user's SSH configuration; on drop it is killed and its directory removed.
#![allow(dead_code)] // each test binary uses a different part

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use tempfile::TempDir;
use vibe_ssh_core::Host;

pub const PASSPHRASE: &str = "temporary passphrase";

pub struct Sshd {
    pub dir: TempDir,
    pub port: u16,
    pub fingerprint: String,
    child: Child,
}

fn keygen(path: &Path, passphrase: &str) {
    let status = Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

impl Sshd {
    /// Accepts `id_plain` and the passphrase-protected `id_encrypted` for the current user.
    pub fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str| dir.path().join(name);
        keygen(&at("host_key"), "");
        keygen(&at("id_plain"), "");
        keygen(&at("id_encrypted"), PASSPHRASE);
        let keys = [at("id_plain.pub"), at("id_encrypted.pub")]
            .map(|p| std::fs::read_to_string(p).unwrap());
        std::fs::write(at("authorized_keys"), keys.concat()).unwrap();
        let listing = Command::new("/usr/bin/ssh-keygen")
            .arg("-lf")
            .arg(at("host_key.pub"))
            .output()
            .unwrap();
        let fingerprint = String::from_utf8(listing.stdout).unwrap();
        let fingerprint = fingerprint.split_whitespace().nth(1).unwrap().to_owned();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\n\
             PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nLogLevel ERROR\n",
            at("host_key").display(),
            at("sshd.pid").display(),
            at("authorized_keys").display(),
        );
        std::fs::write(at("sshd_config"), config).unwrap();
        let child = Command::new("/usr/sbin/sshd")
            .args(["-D", "-e", "-f"])
            .arg(at("sshd_config"))
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Self {
            dir,
            port,
            fingerprint,
            child,
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A host entry for this server, signing in with the named key file.
    pub fn host(&self, id: &str, key: &str, agent_access: bool) -> Host {
        Host {
            id: id.into(),
            name: id.into(),
            host: "127.0.0.1".into(),
            port: self.port,
            username: std::env::var("USER").unwrap(),
            identity_file: Some(self.path(key)),
            host_key_sha256: self.fingerprint.clone(),
            agent_access,
        }
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
