//! The user's directories, per the XDG Base Directory specification.
//!
//! One place, so every file BiGame-mode keeps for the user resolves the same
//! way. With `HOME` unset the home directory comes from the password database,
//! never from a shared location such as `/tmp`, where another user could have
//! prepared the files this process then trusts.

use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

/// The user's home directory: `HOME` when it is an absolute path, otherwise
/// the password database's entry for this user, otherwise `/` (where the user
/// can write nothing, so a failure is loud rather than misplaced).
#[must_use]
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(passwd_home)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn passwd_home() -> Option<PathBuf> {
    let mut buf = vec![0u8; 16 * 1024];
    // SAFETY: getpwuid_r writes only into `pwd` and `buf`, both owned here and
    // sized as passed; `result` is null or points at `pwd`.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            libc::getuid(),
            &raw mut pwd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &raw mut result,
        );
        if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        let dir = CStr::from_ptr(pwd.pw_dir).to_bytes();
        (!dir.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(dir)))
    }
}

fn xdg(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home_dir().join(default))
}

/// `$XDG_CONFIG_HOME`, or `~/.config`.
#[must_use]
pub fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_CACHE_HOME`, or `~/.cache`.
#[must_use]
pub fn cache_home() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
#[must_use]
pub fn data_home() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_STATE_HOME`, or `~/.local/state`.
#[must_use]
pub fn state_home() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_home_directory_is_always_absolute() {
        assert!(super::home_dir().is_absolute());
        // A build container may have no passwd entry; when there is one, it is
        // an absolute path.
        assert!(super::passwd_home().is_none_or(|p| p.is_absolute()));
        assert!(super::config_home().is_absolute());
    }
}
