//! Persistence of the Skool cookie header in the app data directory.
//!
//! Unix: the file is created with mode 0600. Windows: the app data folder
//! (`%APPDATA%`) is already private to the user through its inherited ACL; no
//! extra ACL is applied here (unverified on Windows). The header is never
//! logged and never lives in the repository or the library folder.

use std::io::{self, Write};
use std::path::PathBuf;

pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Session file of `platform` inside the app data directory:
    /// `<dir>/<platform>-session`.
    pub fn for_platform(dir: &std::path::Path, platform: &str) -> Self {
        Self::new(dir.join(format!("{platform}-session")))
    }

    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let text = text.trim();
                Ok((!text.is_empty()).then(|| text.to_string()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes through a private temporary file and renames it, so the header
    /// is never readable by other users, not even briefly.
    pub fn save(&self, header: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp)?;
            #[cfg(unix)]
            std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
            file.write_all(header.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    pub fn clear(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &std::path::Path) -> SessionStore {
        SessionStore::new(dir.join("nested/skool-session"))
    }

    #[test]
    fn each_platform_gets_its_own_private_file_in_the_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skool = SessionStore::for_platform(tmp.path(), "skool");
        let hotmart = SessionStore::for_platform(tmp.path(), "hotmart");
        assert_eq!(skool.path(), tmp.path().join("skool-session"));
        assert_eq!(hotmart.path(), tmp.path().join("hotmart-session"));
        skool.save("a").unwrap();
        assert_eq!(hotmart.load().unwrap(), None, "sessions never mix");
        hotmart.save("b").unwrap();
        assert_eq!(skool.load().unwrap().as_deref(), Some("a"));
    }

    #[test]
    fn missing_or_blank_file_is_no_session() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        assert_eq!(s.load().unwrap(), None);
        s.save("  \n").unwrap();
        assert_eq!(s.load().unwrap(), None);
    }

    #[test]
    fn save_then_load_round_trips_and_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.save("auth_token=a").unwrap();
        assert_eq!(s.load().unwrap().as_deref(), Some("auth_token=a"));
        s.save("auth_token=b").unwrap();
        assert_eq!(s.load().unwrap().as_deref(), Some("auth_token=b"));
    }

    #[test]
    fn clear_removes_the_file_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.clear().unwrap();
        s.save("auth_token=a").unwrap();
        s.clear().unwrap();
        assert!(!s.path().exists());
        assert_eq!(s.load().unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private_to_the_user_even_after_overwrite() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.save("auth_token=a").unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(s.path()), 0o600);
        std::fs::set_permissions(s.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        s.save("auth_token=b").unwrap();
        assert_eq!(mode(s.path()), 0o600);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path());
        s.save("auth_token=a").unwrap();
        let names: Vec<_> = std::fs::read_dir(tmp.path().join("nested"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["skool-session"]);
    }
}
