use std::env;
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::mem;
#[cfg(unix)]
use std::os::fd::AsRawFd;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use keyring::Entry;

use crate::config::CredentialRef;
use crate::error::CliError;
use crate::redaction::Sensitive;

const SERVICE_NAME: &str = "apollo-cli";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialSource {
    Env,
    File,
    Native,
    None,
}

impl CredentialSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::File => "file",
            Self::Native => "native",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CredentialStatus {
    pub authenticated: bool,
    pub source: CredentialSource,
    pub backend: Option<String>,
    pub key: Option<String>,
}

pub trait CredentialStore {
    fn get(&self, key: &str) -> Result<Option<Sensitive>, String>;
    fn set(&self, key: &str, token: &Sensitive) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

pub struct NativeCredentialStore;

impl CredentialStore for NativeCredentialStore {
    fn get(&self, key: &str) -> Result<Option<Sensitive>, String> {
        if native_disabled_for_tests() {
            return Err("native credential store disabled".to_owned());
        }
        let entry = Entry::new(SERVICE_NAME, key).map_err(|error| error.to_string())?;
        match entry.get_password() {
            Ok(token) => Ok(Some(Sensitive::new(token))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, key: &str, token: &Sensitive) -> Result<(), String> {
        if native_disabled_for_tests() {
            return Err("native credential store disabled".to_owned());
        }
        let entry = Entry::new(SERVICE_NAME, key).map_err(|error| error.to_string())?;
        entry
            .set_password(token.expose_secret())
            .map_err(|error| error.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        if native_disabled_for_tests() {
            return Err("native credential store disabled".to_owned());
        }
        let entry = Entry::new(SERVICE_NAME, key).map_err(|error| error.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

pub struct FileCredentialStore {
    base_dir: PathBuf,
}

impl FileCredentialStore {
    pub fn new(config_path: &Path) -> Self {
        Self {
            base_dir: config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("credentials"),
        }
    }

    pub fn path_for_key(&self, key: &str) -> PathBuf {
        self.base_dir.join(format!("{}.token", key))
    }
}

impl CredentialStore for FileCredentialStore {
    fn get(&self, key: &str) -> Result<Option<Sensitive>, String> {
        let path = self.path_for_key(key);
        if !path.exists() {
            return Ok(None);
        }
        let token = fs::read_to_string(path).map_err(|error| error.to_string())?;
        Ok(Some(Sensitive::new(token.trim().to_owned())))
    }

    fn set(&self, key: &str, token: &Sensitive) -> Result<(), String> {
        fs::create_dir_all(&self.base_dir).map_err(|error| error.to_string())?;
        let path = self.path_for_key(key);
        fs::write(&path, token.expose_secret()).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.path_for_key(key);
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

pub fn status(
    config_path: &Path,
    profile: &str,
    credential: Option<&CredentialRef>,
) -> CredentialStatus {
    if env_token().is_some() {
        return CredentialStatus {
            authenticated: true,
            source: CredentialSource::Env,
            backend: Some("env".to_owned()),
            key: Some("APOLLO_TOKEN".to_owned()),
        };
    }

    let credential = credential_for_profile(profile, credential);
    let store_result = token_from_store(config_path, &credential);

    CredentialStatus {
        authenticated: store_result.ok().flatten().is_some(),
        source: source_from_backend(&credential.backend),
        backend: Some(credential.backend),
        key: Some(credential.key),
    }
}

pub fn resolve_token(
    config_path: &Path,
    profile: Option<&str>,
    credential: Option<&CredentialRef>,
) -> Result<Option<Sensitive>, String> {
    if let Some(token) = env_token() {
        return Ok(Some(token));
    }

    let Some(profile) = profile else {
        return Ok(None);
    };
    let credential = credential_for_profile(profile, credential);
    token_from_store(config_path, &credential)
}

pub fn store_file(
    config_path: &Path,
    key: &str,
    token: &Sensitive,
) -> Result<CredentialRef, String> {
    FileCredentialStore::new(config_path).set(key, token)?;
    Ok(CredentialRef {
        backend: "file".to_owned(),
        key: key.to_owned(),
    })
}

pub fn store_native(key: &str, token: &Sensitive) -> Result<CredentialRef, String> {
    NativeCredentialStore.set(key, token)?;
    Ok(CredentialRef {
        backend: "native".to_owned(),
        key: key.to_owned(),
    })
}

pub fn delete(config_path: &Path, credential: &CredentialRef) -> Result<(), String> {
    match credential.backend.as_str() {
        "file" => FileCredentialStore::new(config_path).delete(&credential.key),
        "native" => NativeCredentialStore.delete(&credential.key),
        "env" => {
            Err("APOLLO_TOKEN is provided by the environment and cannot be removed".to_owned())
        }
        _ => Ok(()),
    }
}

pub fn token_from_env_or_stdin(
    token_stdin: bool,
    format: crate::cli::OutputFormat,
) -> Result<Sensitive, CliError> {
    if token_stdin {
        let stdin = std::io::stdin();
        return token_from_reader(stdin.lock(), format);
    }

    if let Some(token) = env_token() {
        return Ok(token);
    }

    if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        return prompt_token(format);
    }

    Err(CliError::invalid_input(
        "provide a token with interactive prompt, --token-stdin, or APOLLO_TOKEN",
        format,
    ))
}

pub fn prompt_token(format: crate::cli::OutputFormat) -> Result<Sensitive, CliError> {
    let token = prompt_hidden("Consumer token: ", format)?;
    token_from_value(token, format)
}

#[cfg(unix)]
fn prompt_hidden(prompt: &str, format: crate::cli::OutputFormat) -> Result<String, CliError> {
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let mut stderr = std::io::stderr().lock();

    stderr
        .write_all(prompt.as_bytes())
        .and_then(|()| stderr.flush())
        .map_err(|error| CliError::invalid_input(&error.to_string(), format))?;

    let mut echo_guard = TerminalEchoGuard::disable(fd, format)?;
    let mut token = String::new();
    let read_result = stdin.lock().read_line(&mut token);
    echo_guard.restore();
    writeln!(stderr).map_err(|error| CliError::invalid_input(&error.to_string(), format))?;

    read_result.map_err(|error| CliError::invalid_input(&error.to_string(), format))?;
    Ok(token)
}

#[cfg(unix)]
struct TerminalEchoGuard {
    fd: libc::c_int,
    original: libc::termios,
    active: bool,
}

#[cfg(unix)]
impl TerminalEchoGuard {
    fn disable(fd: libc::c_int, format: crate::cli::OutputFormat) -> Result<Self, CliError> {
        let original = unsafe {
            let mut term = mem::MaybeUninit::<libc::termios>::uninit();
            if libc::tcgetattr(fd, term.as_mut_ptr()) != 0 {
                return Err(CliError::invalid_input(
                    &std::io::Error::last_os_error().to_string(),
                    format,
                ));
            }
            term.assume_init()
        };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
            return Err(CliError::invalid_input(
                &std::io::Error::last_os_error().to_string(),
                format,
            ));
        }
        Ok(Self {
            fd,
            original,
            active: true,
        })
    }

    fn restore(&mut self) {
        if self.active {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
            }
            self.active = false;
        }
    }
}

#[cfg(unix)]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(not(unix))]
fn prompt_hidden(prompt: &str, format: crate::cli::OutputFormat) -> Result<String, CliError> {
    rpassword::prompt_password(prompt)
        .map_err(|error| CliError::invalid_input(&error.to_string(), format))
}

fn token_from_reader<R: BufRead>(
    mut reader: R,
    format: crate::cli::OutputFormat,
) -> Result<Sensitive, CliError> {
    let mut token = String::new();
    reader
        .read_line(&mut token)
        .map_err(|error| CliError::invalid_input(&error.to_string(), format))?;
    token_from_value(token, format)
}

fn token_from_value(
    token: String,
    format: crate::cli::OutputFormat,
) -> Result<Sensitive, CliError> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err(CliError::invalid_input("token input was empty", format));
    }
    Ok(Sensitive::new(token))
}

fn source_from_backend(backend: &str) -> CredentialSource {
    match backend {
        "file" => CredentialSource::File,
        "native" => CredentialSource::Native,
        _ => CredentialSource::None,
    }
}

fn credential_for_profile(profile: &str, credential: Option<&CredentialRef>) -> CredentialRef {
    credential.cloned().unwrap_or_else(|| CredentialRef {
        backend: "native".to_owned(),
        key: profile.to_owned(),
    })
}

fn token_from_store(
    config_path: &Path,
    credential: &CredentialRef,
) -> Result<Option<Sensitive>, String> {
    match credential.backend.as_str() {
        "file" => FileCredentialStore::new(config_path).get(&credential.key),
        "native" => NativeCredentialStore.get(&credential.key),
        _ => Ok(None),
    }
}

fn env_token() -> Option<Sensitive> {
    env::var("APOLLO_TOKEN")
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .map(Sensitive::new)
}

fn native_disabled_for_tests() -> bool {
    env::var("APOLLO_CLI_TEST_DISABLE_NATIVE").is_ok()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::io::Cursor;

    use super::CredentialStore;
    use crate::cli::OutputFormat;
    use crate::redaction::Sensitive;

    #[derive(Default)]
    struct InMemoryCredentialStore {
        credentials: RefCell<BTreeMap<String, String>>,
    }

    impl CredentialStore for InMemoryCredentialStore {
        fn get(&self, key: &str) -> Result<Option<Sensitive>, String> {
            Ok(self
                .credentials
                .borrow()
                .get(key)
                .cloned()
                .map(Sensitive::new))
        }

        fn set(&self, key: &str, token: &Sensitive) -> Result<(), String> {
            self.credentials
                .borrow_mut()
                .insert(key.to_owned(), token.expose_secret().to_owned());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.credentials.borrow_mut().remove(key);
            Ok(())
        }
    }

    #[test]
    fn in_memory_store_supports_set_get_delete() {
        let store = InMemoryCredentialStore::default();
        let token = Sensitive::new("secret-token");

        store.set("dev", &token).expect("set token");
        let stored = store.get("dev").expect("get token").expect("stored token");
        assert_eq!(stored.expose_secret(), "secret-token");

        store.delete("dev").expect("delete token");
        assert!(store.get("dev").expect("get after delete").is_none());
    }

    #[test]
    fn token_from_reader_accepts_enter_terminated_token() {
        let token = super::token_from_reader(
            Cursor::new("secret-from-stdin\nignored-second-line\n"),
            OutputFormat::Json,
        )
        .expect("token");

        assert_eq!(token.expose_secret(), "secret-from-stdin");
    }

    #[test]
    fn token_from_reader_rejects_empty_token() {
        let error = super::token_from_reader(Cursor::new("\n"), OutputFormat::Json)
            .expect_err("empty token should fail");

        assert_eq!(error.exit_code(), 1);
    }
}
