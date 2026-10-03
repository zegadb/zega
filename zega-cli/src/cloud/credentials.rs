//! Where `zega-server cloud login` keeps the token, and how secrets are read.
//!
//! One JSON file, `cloud.json`, in the user's config directory:
//! `$XDG_CONFIG_HOME/zega` (default `~/.config/zega`) on Linux and macOS,
//! `%APPDATA%\zega` on Windows. Mode 0600 in a 0700 directory on Unix; on
//! Windows the profile directory's own ACL keeps other users out. The file
//! holds the token and the API host it belongs to, so a later command talks
//! to the host the token was made for.

use super::api::CloudError;
use serde_json::{json, Value};
use std::{
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

pub struct Stored {
    pub api: String,
    pub token: String,
}

/// The directory that holds `cloud.json`. Reading the home or config
/// directory is how the OS says where a user's files live, not product
/// behavior: nothing here changes what a command does.
fn config_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let appdata = std::env::var_os("APPDATA").filter(|value| !value.is_empty());
        appdata
            .map(|appdata| PathBuf::from(appdata).join("zega"))
            .ok_or_else(|| "cannot find your config directory: %APPDATA% is not set".to_string())
    }
    #[cfg(not(windows))]
    {
        let absolute = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        absolute("XDG_CONFIG_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".config")))
            .map(|config| config.join("zega"))
            .ok_or_else(|| "cannot find your config directory: HOME is not set".to_string())
    }
}

pub fn credentials_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("cloud.json"))
}

pub fn load() -> Result<Option<Stored>, CloudError> {
    let path = credentials_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display()).into()),
    };
    let broken = |why: &str| {
        CloudError::Local(format!(
            "{} is not a zega-server cloud credentials file ({why}); run `zega-server cloud login` to write it again",
            path.display()
        ))
    };
    let value: Value = serde_json::from_str(&text).map_err(|error| broken(&error.to_string()))?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    };
    match (field("api"), field("token")) {
        (Some(api), Some(token)) => Ok(Some(Stored {
            api: api.to_string(),
            token: token.to_string(),
        })),
        _ => Err(broken("it needs an `api` and a `token`")),
    }
}

/// Write the credentials file: never readable by anyone else, not even for an
/// instant (the file is created with its final mode, then renamed into place,
/// so a crash leaves the old file or the new one).
pub fn save(api: &str, token: &str) -> Result<PathBuf, CloudError> {
    let path = credentials_path()?;
    let dir = path
        .parent()
        .ok_or("the credentials file has no directory")?;
    create_private_dir(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let mut partial = path.as_os_str().to_owned();
    partial.push(format!(".{}.partial", std::process::id()));
    let partial = PathBuf::from(partial);
    let body = serde_json::to_string_pretty(&json!({ "api": api, "token": token }))
        .map_err(|error| error.to_string())?;
    let written = open_private(&partial).and_then(|mut file| {
        file.write_all(body.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        // Closed before the rename: Windows refuses to rename an open file.
        drop(file);
        std::fs::rename(&partial, &path)
    });
    if let Err(error) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("cannot write {}: {error}", path.display()).into());
    }
    Ok(path)
}

/// Remove the credentials file. `Ok(None)` when there was none.
pub fn remove() -> Result<Option<PathBuf>, CloudError> {
    let path = credentials_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Some(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display()).into()),
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn open_private(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Read a token from `--token-file`, the way `zega-server start --token-file` does:
/// one nonempty token, a final newline is fine.
pub fn read_token_file(path: &Path) -> Result<String, CloudError> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read token file {}: {error}", path.display()))?;
    one_token(&text).ok_or_else(|| {
        format!(
            "token file {} must contain one nonempty token",
            path.display()
        )
        .into()
    })
}

fn one_token(text: &str) -> Option<String> {
    let token = text.trim();
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then(|| token.to_string())
}

/// A token for `login`: stdin when it is piped, a hidden prompt on a
/// terminal. Never from an argument: arguments show in process listings.
pub fn read_token() -> Result<String, CloudError> {
    let text = read_hidden("API token (input is hidden): ")?;
    one_token(&text)
        .ok_or_else(|| "a token is one nonempty word, with no spaces or line breaks".into())
}

/// A secret's value: stdin or a hidden prompt, exactly as typed except for the
/// one line break that ends it.
pub fn read_secret(name: &str) -> Result<String, CloudError> {
    let text = read_hidden(&format!("Value for secret {name} (input is hidden): "))?;
    if text.is_empty() {
        return Err("the secret's value is empty".into());
    }
    Ok(text)
}

fn read_hidden(prompt: &str) -> Result<String, CloudError> {
    let mut text = if io::stdin().is_terminal() {
        eprint!("{prompt}");
        let _ = io::stderr().flush();
        let text = rpassword::read_password()
            .map_err(|error| format!("cannot read from the terminal: {error}"))?;
        eprintln!();
        text
    } else {
        let mut text = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut text)
            .map_err(|error| format!("cannot read stdin: {error}"))?;
        text
    };
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::one_token;

    #[test]
    fn a_token_is_one_word() {
        assert_eq!(one_token("zc_abc\n").as_deref(), Some("zc_abc"));
        assert_eq!(one_token("  zc_abc  \r\n").as_deref(), Some("zc_abc"));
        assert_eq!(one_token(""), None);
        assert_eq!(one_token("\n"), None);
        assert_eq!(one_token("zc_abc zc_def"), None);
        assert_eq!(one_token("zc_abc\nzc_def"), None);
    }
}
