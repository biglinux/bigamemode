//! Filesystem change notification.
//!
//! Audit finding DBUS-01: the falcond status service re-read
//! `/tmp/falcond_status` every 500 ms for the entire life of the process and
//! diffed the whole string — two wakeups a second, forever, in an application
//! whose stated purpose is to stay out of a game's way.
//!
//! falcond does not own a D-Bus name to subscribe to, so the file really is the
//! only channel. But watching a file and polling it are different things:
//! `inotify` blocks until the kernel has something to say, which costs nothing
//! while nothing is happening.
//!
//! The watch is on the **parent directory**, not the file. falcond rewrites its
//! status by creating a new file and renaming it into place, which replaces the
//! inode — a watch on the old inode would go deaf after the first update, and
//! would never see the file appear if falcond had not started yet.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc;

/// Events worth waking up for.
///
/// `CLOSE_WRITE` covers an in-place rewrite, `MOVED_TO` a rename into place,
/// `CREATE` the file first appearing, and `DELETE` it going away with falcond.
const EVENTS: u32 = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE;

/// A directory watch that reports when one named file changes.
pub struct FileWatch {
    fd: i32,
}

impl FileWatch {
    /// Watch the directory containing `path` for changes to that file.
    ///
    /// Returns `None` if the directory does not exist or inotify is
    /// unavailable, in which case the caller should fall back to polling.
    #[must_use]
    pub fn new(path: &Path) -> Option<Self> {
        let dir = path.parent()?;
        let mut c_dir = dir.as_os_str().as_bytes().to_vec();
        c_dir.push(0);

        // SAFETY: inotify_init1 takes only flags and returns a file descriptor
        // or -1. IN_CLOEXEC keeps the descriptor out of any child process.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        // SAFETY: `fd` is a live inotify descriptor and `c_dir` is a
        // NUL-terminated path that outlives the call.
        let wd = unsafe { libc::inotify_add_watch(fd, c_dir.as_ptr().cast(), EVENTS) };
        if wd < 0 {
            // SAFETY: `fd` was returned by inotify_init1 and is not used again.
            unsafe { libc::close(fd) };
            return None;
        }
        Some(Self { fd })
    }

    /// Block until the kernel reports an event, then drain the queue.
    ///
    /// Returns `false` when the descriptor fails, which tells the caller to
    /// stop watching rather than spin.
    ///
    /// Events are not filtered by name here. Draining and letting the caller
    /// re-read is simpler than parsing variable-length records, and the caller
    /// compares contents anyway — a spurious wakeup costs one file read, while
    /// a missed one costs a stale UI.
    #[must_use]
    pub fn wait(&self) -> bool {
        // Large enough for many queued records; a short read is fine.
        let mut buf = [0u8; 4096];
        // SAFETY: `self.fd` is a live inotify descriptor and `buf` is a valid
        // writable region of the stated length.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
        n > 0
    }
}

impl Drop for FileWatch {
    fn drop(&mut self) {
        // SAFETY: `self.fd` was returned by inotify_init1 and is closed once.
        unsafe { libc::close(self.fd) };
    }
}

/// Watch `path` on a background thread, sending its contents on every change.
///
/// The current contents are sent immediately, then again on each change.
/// Identical consecutive contents are suppressed, because a directory watch
/// sees writes to neighbouring files too.
///
/// Returns `None` when a watch could not be established.
#[must_use]
pub fn watch_file(path: &Path) -> Option<mpsc::Receiver<String>> {
    let watch = FileWatch::new(path)?;
    let (tx, rx) = mpsc::channel();
    let path = path.to_owned();

    std::thread::Builder::new()
        .name("bigame-file-watch".into())
        .spawn(move || {
            let mut last: Option<String> = None;
            loop {
                // Empty content is skipped, not reported.
                //
                // `write(2)` to a new file produces IN_CREATE before the data
                // lands, so a reader woken by that event can observe a
                // zero-byte file. Reporting it would hand the caller an empty
                // status and, worse, record it as the last-seen value — so the
                // real contents arriving a moment later would look like a
                // change from "" rather than the first real reading.
                //
                // A zero-byte status file is never meaningful anyway: the next
                // event carries the actual data.
                let current = std::fs::read_to_string(&path)
                    .ok()
                    .filter(|c| !c.is_empty());
                if let Some(content) = current {
                    if last.as_ref() != Some(&content) {
                        if tx.send(content.clone()).is_err() {
                            return; // receiver dropped
                        }
                        last = Some(content);
                    }
                }
                if !watch.wait() {
                    return;
                }
            }
        })
        .ok()?;
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bigame_watch_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_watch_on_a_missing_directory_fails_cleanly() {
        assert!(FileWatch::new(Path::new("/definitely/not/here/file")).is_none());
        assert!(watch_file(Path::new("/definitely/not/here/file")).is_none());
    }

    #[test]
    fn existing_contents_arrive_immediately() {
        let dir = tempdir("initial");
        let file = dir.join("status");
        std::fs::write(&file, "first").unwrap();

        let rx = watch_file(&file).expect("watch should start");
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, "first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rewrite_wakes_the_watcher() {
        let dir = tempdir("rewrite");
        let file = dir.join("status");
        std::fs::write(&file, "one").unwrap();
        let rx = watch_file(&file).expect("watch should start");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "one");

        std::fs::write(&file, "two").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rename_into_place_wakes_the_watcher() {
        // This is how falcond actually updates its status, and why the watch is
        // on the directory rather than on the file's inode.
        let dir = tempdir("rename");
        let file = dir.join("status");
        std::fs::write(&file, "one").unwrap();
        let rx = watch_file(&file).expect("watch should start");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "one");

        let tmp = dir.join("status.tmp");
        std::fs::write(&tmp, "replaced").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "replaced");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_created_after_the_watch_starts_is_seen() {
        // falcond may not be running yet when the application starts.
        let dir = tempdir("late");
        let file = dir.join("status");
        let rx = watch_file(&file).expect("watch should start even with no file");

        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(&file, "appeared").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "appeared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partially_written_file_is_not_reported_as_empty() {
        // write(2) to a new file emits IN_CREATE before the data lands, so a
        // reader woken by that event can see zero bytes. Reporting it would
        // also poison `last`, making the real contents look like a change
        // from "" rather than the first reading.
        let dir = tempdir("empty");
        let file = dir.join("status");
        let rx = watch_file(&file).expect("watch should start");

        std::fs::write(&file, "").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

        std::fs::write(&file, "real").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "real");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_contents_are_not_resent() {
        let dir = tempdir("dedup");
        let file = dir.join("status");
        std::fs::write(&file, "same").unwrap();
        let rx = watch_file(&file).expect("watch should start");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "same");

        // Rewriting identical content, and touching a neighbour, must not
        // produce a second message.
        std::fs::write(&file, "same").unwrap();
        std::fs::write(dir.join("unrelated"), "x").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
