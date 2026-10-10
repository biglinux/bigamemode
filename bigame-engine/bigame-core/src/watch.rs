//! Filesystem change notification.
//!
//! falcond does not own a D-Bus name to subscribe to, so its status file is the
//! only channel — and it is watched, never polled: an application whose purpose
//! is to stay out of a game's way must not wake up on a timer. `inotify` blocks
//! until the kernel has something to say, which costs nothing while nothing is
//! happening.
//!
//! The watch is on the **parent directory**, not the file. falcond rewrites its
//! status by creating a new file and renaming it into place, which replaces the
//! inode — a watch on the old inode would go deaf after the first update, and
//! would never see the file appear if falcond had not started yet.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc;

/// The most of a watched file that is read. A status file is a few hundred
/// bytes; a file another user made huge must not be read to exhaustion.
pub const MAX_WATCHED_BYTES: u64 = 64 * 1024;

/// Events worth waking up for.
///
/// `CLOSE_WRITE` covers an in-place rewrite, `MOVED_TO` a rename into place,
/// `CREATE` the file first appearing, and `DELETE` it going away with falcond.
const EVENTS: u32 = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE;

/// A directory watch that reports when one named file changes.
pub struct FileWatch {
    fd: i32,
    /// The watched file's name in the directory.
    name: Vec<u8>,
}

impl FileWatch {
    /// Watch the directory containing `path` for changes to that file.
    ///
    /// Returns `None` if the directory does not exist or inotify is
    /// unavailable, in which case the caller should fall back to polling.
    #[must_use]
    pub fn new(path: &Path) -> Option<Self> {
        let dir = path.parent()?;
        let name = path.file_name()?.as_bytes().to_vec();
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
        Some(Self { fd, name })
    }

    /// Block until the kernel reports an event on the file, draining the
    /// queue as it goes.
    ///
    /// Returns `false` when the descriptor fails, which tells the caller to
    /// stop watching rather than spin.
    ///
    /// Events on the directory's other files are passed over: falcond 2.0.2
    /// writes to `/tmp`, where every program's temporary file would
    /// otherwise wake the watcher for a read of an unchanged status.
    #[must_use]
    pub fn wait(&self) -> bool {
        // Large enough for many queued records; a short read is fine.
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: `self.fd` is a live inotify descriptor and `buf` is a
            // valid writable region of the stated length.
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
            let Ok(n) = usize::try_from(n) else {
                return false;
            };
            if n == 0 {
                return false;
            }
            if concerns(&buf[..n], &self.name) {
                return true;
            }
        }
    }
}

/// Whether the inotify records in `events` concern the file `name`, or may:
/// a queue overflow lost events, and a watch the kernel removed (the
/// directory went away) will report nothing again.
fn concerns(events: &[u8], name: &[u8]) -> bool {
    // struct inotify_event: wd (i32), mask, cookie, len (u32), then `len`
    // bytes of NUL-padded name.
    const HEADER: usize = 16;
    let mut rest = events;
    while rest.len() >= HEADER {
        let field =
            |at: usize| u32::from_ne_bytes([rest[at], rest[at + 1], rest[at + 2], rest[at + 3]]);
        let mask = field(4);
        let len = usize::try_from(field(12)).unwrap_or(usize::MAX);
        let Some(record) = rest.get(HEADER..HEADER.saturating_add(len)) else {
            // A record cut short: say yes rather than miss a change.
            return true;
        };
        if mask & (libc::IN_Q_OVERFLOW | libc::IN_IGNORED) != 0 {
            return true;
        }
        let event_name = record.split(|b| *b == 0).next().unwrap_or_default();
        if event_name == name {
            return true;
        }
        rest = &rest[HEADER + len..];
    }
    false
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
/// Identical consecutive contents are suppressed: a rewrite that changes
/// nothing, or the several events of one rename into place, is no change.
///
/// Returns `None` when a watch could not be established.
#[must_use]
pub fn watch_file(path: &Path) -> Option<mpsc::Receiver<String>> {
    fn read_capped(path: &Path) -> Option<String> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let mut text = String::new();
        // Never blocks (a FIFO under the name) and never follows a symlink.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .ok()?
            .take(MAX_WATCHED_BYTES)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }
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
                let current = read_capped(&path).filter(|c| !c.is_empty());
                if let Some(content) = current
                    && last.as_ref() != Some(&content)
                {
                    if tx.send(content.clone()).is_err() {
                        return; // receiver dropped
                    }
                    last = Some(content);
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
    fn only_the_watched_file_wakes_the_watch() {
        let dir = tempdir("neighbours");
        let file = dir.join("status");
        let watch = FileWatch::new(&file).expect("watch should start");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(watch.wait());
        });
        std::thread::sleep(Duration::from_millis(50));
        // Another program's files in the same directory.
        std::fs::write(dir.join("status.other"), "x").unwrap();
        std::fs::write(dir.join("sta"), "x").unwrap();
        std::fs::remove_file(dir.join("sta")).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        std::fs::write(&file, "now").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inotify_records_are_matched_by_name() {
        let record = |mask: u32, name: &[u8], padded: usize| {
            let mut r = Vec::new();
            r.extend_from_slice(&1i32.to_ne_bytes());
            r.extend_from_slice(&mask.to_ne_bytes());
            r.extend_from_slice(&0u32.to_ne_bytes());
            r.extend_from_slice(&u32::try_from(padded).unwrap().to_ne_bytes());
            r.extend_from_slice(name);
            r.resize(16 + padded, 0);
            r
        };
        let other = record(libc::IN_CLOSE_WRITE, b"falcond_status.tmp", 32);
        let ours = record(libc::IN_MOVED_TO, b"falcond_status", 16);
        assert!(!concerns(&other, b"falcond_status"));
        assert!(concerns(&[other.clone(), ours].concat(), b"falcond_status"));
        assert!(!concerns(
            &record(libc::IN_CREATE, b"falcond", 16),
            b"falcond_status"
        ));
        // Events lost, or the watch gone: the file may have changed.
        assert!(concerns(
            &record(libc::IN_Q_OVERFLOW, b"", 0),
            b"falcond_status"
        ));
        assert!(concerns(
            &record(libc::IN_IGNORED, b"", 0),
            b"falcond_status"
        ));
        assert!(!concerns(&[], b"falcond_status"));
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
