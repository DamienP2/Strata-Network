//! Data folders and atomic file writes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The data folder of an interactive run (per-user).
pub fn user_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "Strata", "Strata Network")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// The data folder of the installed system service.
pub fn service_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/Library/Application Support/Strata Network")
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        base.join("Strata Network")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        PathBuf::from("/var/lib/strata-network")
    }
}

/// The folder the `user …` commands act on: the one given, else the installed service's
/// folder when it exists, else the user's own.
pub fn cli_data_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = explicit {
        return dir;
    }
    let service = service_data_dir();
    if service.join("users.json").exists() || service.join("config.json").exists() {
        service
    } else {
        user_data_dir()
    }
}

/// Writes `bytes` to `<path>.tmp` then renames it over `path` (atomic). `sensitive` files are
/// created readable by their owner only (0600) on Unix.
pub fn write_atomic(path: &Path, bytes: &[u8], sensitive: bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if sensitive {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        #[cfg(unix)]
        if sensitive {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = sensitive;
    std::fs::rename(&tmp, path)
}

/// Pretty JSON written atomically (`<file>.json.tmp` then `rename`).
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T, sensitive: bool) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &bytes, sensitive)
}

/// Reads a JSON file: `Ok(None)` when it does not exist.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// When a command runs as root (sudo) on a data folder owned by someone else, gives the file
/// back to the folder's owner so the service can still read it.
pub fn give_back(path: &Path, data_dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let Ok(meta) = std::fs::metadata(data_dir) else { return };
        if meta.uid() == 0 {
            return;
        }
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else { return };
        // SAFETY: c is a valid NUL-terminated path.
        unsafe {
            libc::chown(c.as_ptr(), meta.uid(), meta.gid());
        }
    }
    #[cfg(not(unix))]
    let _ = (path, data_dir);
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// A new short id: 12 hex characters of a UUID v4.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_json_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sn-paths-{}", new_id()));
        let file = dir.join("a").join("x.json");
        write_json(&file, &serde_json::json!({"a": 1}), true).unwrap();
        let back: serde_json::Value = read_json(&file).unwrap().unwrap();
        assert_eq!(back["a"], 1);
        assert!(!dir.join("a").join("x.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(read_json::<serde_json::Value>(&dir.join("none.json")).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
