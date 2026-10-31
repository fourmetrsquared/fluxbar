//! Optional size-rotated file logging for the CLI and daemon.
//!
//! The writer is intentionally small and dependency-free. It opens the log in
//! append mode with owner-only permissions, rotates at a byte threshold, and
//! keeps a bounded number of numbered archives (`fluxbar.log.1`, `.2`, ...).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

/// A `MakeWriter` shared by tracing's formatting layer.
#[derive(Clone, Debug)]
pub struct RotatingMakeWriter {
    state: Arc<Mutex<RotatingState>>,
}

#[derive(Debug)]
struct RotatingState {
    path: PathBuf,
    file: File,
    size: u64,
    max_bytes: u64,
    keep: usize,
}

impl RotatingMakeWriter {
    /// Create a log writer. `max_bytes = 0` disables rotation; `keep` bounds
    /// the number of rotated archives (0 truncates the active file instead).
    pub fn new(path: impl Into<PathBuf>, max_bytes: u64, keep: usize) -> io::Result<Self> {
        let path = path.into();
        ensure_parent(&path)?;
        refuse_symlink(&path)?;
        let file = open_log(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            state: Arc::new(Mutex::new(RotatingState {
                path,
                file,
                size,
                max_bytes,
                keep: keep.min(100),
            })),
        })
    }
}

impl<'a> MakeWriter<'a> for RotatingMakeWriter {
    type Writer = RotatingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriter {
            state: self.state.clone(),
        }
    }
}

/// One writer handle returned for an individual tracing event.
pub struct RotatingWriter {
    state: Arc<Mutex<RotatingState>>,
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?;
        state.file.write_all(buf)?;
        state.size = state.size.saturating_add(buf.len() as u64);

        if state.max_bytes > 0 && state.size >= state.max_bytes {
            // A failed rotation must not stop logging. Retry after the next
            // threshold instead of retrying on every individual write.
            if state.rotate().is_err() {
                state.size = 0;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("log file lock poisoned"))?
            .file
            .flush()
    }
}

impl RotatingState {
    fn rotate(&mut self) -> io::Result<()> {
        if self.keep == 0 {
            self.file.set_len(0)?;
            self.size = 0;
            return Ok(());
        }

        let oldest = rotated_path(&self.path, self.keep);
        remove_if_exists(&oldest)?;
        for index in (1..self.keep).rev() {
            rename_if_exists(
                &rotated_path(&self.path, index),
                &rotated_path(&self.path, index + 1),
            )?;
        }
        fs::rename(&self.path, rotated_path(&self.path, 1))?;
        self.file = open_log(&self.path)?;
        self.size = 0;
        Ok(())
    }
}

fn ensure_parent(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || parent.exists() {
        return Ok(());
    }
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn refuse_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to log through symlink {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn open_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(writer: &RotatingMakeWriter, text: &str) {
        writer.make_writer().write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn rotates_at_the_size_threshold_and_keeps_archives() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.log");
        let writer = RotatingMakeWriter::new(&path, 8, 2).unwrap();

        write(&writer, "first---");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 1)).unwrap(),
            "first---"
        );

        write(&writer, "second--");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 1)).unwrap(),
            "second--"
        );
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 2)).unwrap(),
            "first---"
        );

        write(&writer, "third---");
        assert_eq!(
            fs::read_to_string(rotated_path(&path, 2)).unwrap(),
            "second--"
        );
        assert!(!rotated_path(&path, 3).exists());
    }

    #[test]
    fn keep_zero_truncates_instead_of_archiving() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.log");
        let writer = RotatingMakeWriter::new(&path, 4, 0).unwrap();
        write(&writer, "truncate-me");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(!rotated_path(&path, 1).exists());
    }

    #[test]
    fn zero_threshold_disables_rotation() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.log");
        let writer = RotatingMakeWriter::new(&path, 0, 3).unwrap();
        write(&writer, "keep growing");
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep growing");
        assert!(!rotated_path(&path, 1).exists());
    }

    #[cfg(unix)]
    #[test]
    fn log_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("fluxbar.log");
        let _writer = RotatingMakeWriter::new(&path, 1024, 3).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn symlink_log_path_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("target.log");
        fs::write(&target, "do not touch").unwrap();
        let link = dir.path().join("link.log");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = RotatingMakeWriter::new(&link, 1024, 3).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(fs::read_to_string(&target).unwrap(), "do not touch");
    }
}
