//! Credential storage. Release builds always use the user's normal macOS Keychain.
//! The opt-in native acceptance build uses only a disposable, explicitly named keychain.
use security_framework::passwords;

const SERVICE: &str = "io.github.intmainjhy.vibe-ssh";

#[derive(Clone, Default)]
pub struct Credentials {
    #[cfg(feature = "native-acceptance")]
    isolated: Option<security_framework::os::macos::keychain::SecKeychain>,
}

impl Credentials {
    pub fn open() -> Self {
        #[cfg(feature = "native-acceptance")]
        {
            use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
            use std::path::PathBuf;
            let home = vibe_ssh_core::default_home();
            let root = home.parent().expect("acceptance root");
            assert!(
                root.join("acceptance-root").is_file(),
                "isolated fixture required"
            );
            assert_eq!(home, root.join("data"));
            assert_eq!(
                std::env::var_os("VIBE_SSH_INTEGRATION_HOME").map(PathBuf::from),
                Some(root.join("integration"))
            );
            let path = root.join("acceptance.keychain-db");
            let mut keychain = if path.exists() {
                SecKeychain::open(&path).expect("open acceptance keychain")
            } else {
                CreateOptions::new()
                    .password("disposable-native-test")
                    .prompt_user(false)
                    .create(&path)
                    .expect("create acceptance keychain")
            };
            keychain
                .unlock(Some("disposable-native-test"))
                .expect("unlock acceptance keychain");
            Self {
                isolated: Some(keychain),
            }
        }
        #[cfg(not(feature = "native-acceptance"))]
        Self::default()
    }

    pub fn get(&self, account: &str) -> Option<String> {
        #[cfg(feature = "native-acceptance")]
        if let Some(keychain) = &self.isolated {
            let (password, _) = keychain.find_generic_password(SERVICE, account).ok()?;
            return String::from_utf8(password.to_vec()).ok();
        }
        String::from_utf8(passwords::get_generic_password(SERVICE, account).ok()?).ok()
    }

    pub fn set(&self, account: &str, secret: &[u8]) -> Result<(), security_framework::base::Error> {
        #[cfg(feature = "native-acceptance")]
        if let Some(keychain) = &self.isolated {
            return keychain.set_generic_password(SERVICE, account, secret);
        }
        passwords::set_generic_password(SERVICE, account, secret)
    }

    pub fn delete(&self, account: &str) {
        #[cfg(feature = "native-acceptance")]
        if let Some(keychain) = &self.isolated {
            if let Ok((_, item)) = keychain.find_generic_password(SERVICE, account) {
                item.delete();
            }
            return;
        }
        let _ = passwords::delete_generic_password(SERVICE, account);
    }
}
