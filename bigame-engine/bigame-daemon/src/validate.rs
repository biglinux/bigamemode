//! Argument validation for the privileged helper.
//!
//! Everything here runs **inside** the root process, on the server side of the
//! bus. That placement is the entire point: the previous helper relied on the
//! GUI to reject dangerous input, which an attacker simply bypasses by talking
//! to the bus directly. The audit's SEC-02 proof-of-concept turned the profile
//! name `../../../../../etc/cron.d/pwn` into a root-owned file in `/etc/cron.d`.
//!
//! The approach throughout is allow-listing. Denying known-bad patterns invites
//! an encoding that was not thought of; permitting only a known-good character
//! set does not.

/// Longest accepted profile name. Well under `NAME_MAX` once `.conf` is added.
const MAX_PROFILE_NAME: usize = 128;

/// Largest accepted configuration or profile payload (64 KiB).
///
/// Real profiles are a few hundred bytes; this only exists so an unauthenticated
/// bus peer cannot make the helper write an unbounded file.
const MAX_PAYLOAD: usize = 64 * 1024;

/// Validate a per-game profile name.
///
/// Accepts letters, digits, space, `.`, `_`, `-` and `+` — enough for real
/// titles such as `Arc Raiders` and process names such as `Cyberpunk2077.exe`,
/// while making a path separator unrepresentable.
///
/// # Errors
/// Returns a descriptive error when the name could escape the profile
/// directory or is otherwise unusable as a bare filename.
pub fn profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name is empty".into());
    }
    if name.len() > MAX_PROFILE_NAME {
        return Err(format!("profile name exceeds {MAX_PROFILE_NAME} bytes"));
    }
    // A leading dot would create a hidden file; a name of "." or ".." is a
    // directory reference rather than a file.
    if name.starts_with('.') {
        return Err("profile name may not start with '.'".into());
    }
    if name.contains("..") {
        return Err("profile name may not contain '..'".into());
    }
    if let Some(bad) = name.chars().find(|c| !is_allowed_name_char(*c)) {
        return Err(format!(
            "profile name contains a forbidden character: {bad:?}"
        ));
    }
    // Trailing whitespace produces surprising filenames and confusing UI.
    if name.trim() != name {
        return Err("profile name has leading or trailing whitespace".into());
    }
    Ok(())
}

fn is_allowed_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '+')
}

/// Validate a configuration or profile payload.
///
/// # Errors
/// Returns an error if the payload is too large or contains NUL bytes.
pub fn payload(content: &str) -> Result<(), String> {
    if content.len() > MAX_PAYLOAD {
        return Err(format!("payload exceeds {MAX_PAYLOAD} bytes"));
    }
    if content.contains('\0') {
        return Err("payload contains NUL bytes".into());
    }
    Ok(())
}

/// Validate a CPU governor or Energy Performance Preference value.
///
/// # Errors
/// Returns an error for anything outside `[a-z0-9_-]`.
pub fn cpufreq_value(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 32 {
        return Err("value has an implausible length".into());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("value contains characters no cpufreq attribute accepts".into());
    }
    Ok(())
}

/// Validate a DRM card node name such as `card1`.
///
/// # Errors
/// Returns an error for anything that is not `card` followed by digits, which
/// makes it impossible to redirect the write to another sysfs path.
pub fn drm_card(card: &str) -> Result<(), String> {
    let Some(rest) = card.strip_prefix("card") else {
        return Err("not a DRM card name".into());
    };
    if rest.is_empty() || rest.len() > 3 || !rest.chars().all(|c| c.is_ascii_digit()) {
        return Err("not a DRM card name".into());
    }
    Ok(())
}

/// Values `power_dpm_force_performance_level` accepts, per the amdgpu driver.
const DPM_LEVELS: &[&str] = &[
    "auto",
    "low",
    "high",
    "manual",
    "profile_standard",
    "profile_min_sclk",
    "profile_min_mclk",
    "profile_peak",
];

/// Validate a GPU DPM level against the driver's fixed set.
///
/// # Errors
/// Returns an error for any value outside [`DPM_LEVELS`].
pub fn dpm_level(level: &str) -> Result<(), String> {
    if DPM_LEVELS.contains(&level) {
        Ok(())
    } else {
        Err(format!("unknown DPM level; expected one of {DPM_LEVELS:?}"))
    }
}

/// Validate an AMD 3D V-Cache mode.
///
/// # Errors
/// Returns an error for anything other than `frequency` or `cache`.
pub fn vcache_mode(mode: &str) -> Result<(), String> {
    if matches!(mode, "frequency" | "cache") {
        Ok(())
    } else {
        Err("V-Cache mode must be 'frequency' or 'cache'".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_profile_names() {
        for name in [
            "Cyberpunk2077.exe",
            "Arc Raiders",
            "Dead by Daylight",
            "cs2",
            "Civ7_linux_Vulkan_FinalRelease",
            "ffxiv_dx11.exe",
            "Half-Life 2",
            "C++Builder",
        ] {
            assert!(profile_name(name).is_ok(), "should accept {name:?}");
        }
    }

    #[test]
    fn rejects_the_exact_sec_02_payloads() {
        // These are the strings the audit proved wrote into /etc as root.
        for name in [
            "../../../../../etc/cron.d/pwn",
            "../../../../../etc/systemd/system/pwn.service",
            "../../../etc/sudoers.d/pwn",
        ] {
            assert!(profile_name(name).is_err(), "must reject {name:?}");
        }
    }

    #[test]
    fn rejects_every_shape_of_path_escape() {
        for name in [
            "..",
            ".",
            "a/b",
            "/absolute",
            "a\\b",
            "..hidden",
            ".hidden",
            "a\0b",
            "a\nb",
            "a\tb",
            // Percent- and URL-style encodings must not slip through either;
            // they are rejected because '%' is simply not on the allow list.
            "%2e%2e%2fetc",
            "a%00b",
        ] {
            assert!(profile_name(name).is_err(), "must reject {name:?}");
        }
    }

    #[test]
    fn rejects_empty_oversized_and_padded_names() {
        assert!(profile_name("").is_err());
        assert!(profile_name(&"a".repeat(MAX_PROFILE_NAME + 1)).is_err());
        assert!(profile_name(&"a".repeat(MAX_PROFILE_NAME)).is_ok());
        assert!(profile_name(" leading").is_err());
        assert!(profile_name("trailing ").is_err());
    }

    #[test]
    fn a_valid_name_can_never_escape_its_directory() {
        // Property check: for every accepted name, joining it under the
        // profile directory must stay inside that directory.
        let base = std::path::Path::new("/usr/share/falcond/profiles/user");
        for name in [
            "Arc Raiders",
            "cs2",
            "Cyberpunk2077.exe",
            "a.b.c",
            "x+y-z_1",
        ] {
            profile_name(name).unwrap();
            let joined = base.join(format!("{name}.conf"));
            let mut normalized = std::path::PathBuf::new();
            for c in joined.components() {
                match c {
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    other => normalized.push(other.as_os_str()),
                }
            }
            assert!(
                normalized.starts_with(base),
                "{name:?} escaped to {}",
                normalized.display()
            );
            assert_eq!(normalized.parent(), Some(base));
        }
    }

    #[test]
    fn payload_limits() {
        assert!(payload("name = \"x\"\n").is_ok());
        assert!(payload(&"a".repeat(MAX_PAYLOAD)).is_ok());
        assert!(payload(&"a".repeat(MAX_PAYLOAD + 1)).is_err());
        assert!(payload("has\0nul").is_err());
    }

    #[test]
    fn cpufreq_values() {
        assert!(cpufreq_value("performance").is_ok());
        assert!(cpufreq_value("powersave").is_ok());
        assert!(cpufreq_value("balance_performance").is_ok());
        assert!(cpufreq_value("schedutil").is_ok());

        assert!(cpufreq_value("").is_err());
        assert!(
            cpufreq_value("Performance").is_err(),
            "sysfs values are lowercase"
        );
        assert!(cpufreq_value("performance; rm -rf /").is_err());
        assert!(cpufreq_value("../../../etc/shadow").is_err());
        assert!(cpufreq_value(&"a".repeat(33)).is_err());
    }

    #[test]
    fn drm_card_names() {
        assert!(drm_card("card0").is_ok());
        assert!(drm_card("card1").is_ok());
        assert!(drm_card("card127").is_ok());

        // Connector nodes are not cards and must not be writable through here.
        assert!(drm_card("card1-DP-1").is_err());
        assert!(drm_card("card").is_err());
        assert!(drm_card("renderD128").is_err());
        assert!(drm_card("../../../devices").is_err());
        assert!(drm_card("card0/../../..").is_err());
        assert!(drm_card("card99999").is_err());
    }

    #[test]
    fn dpm_levels() {
        assert!(dpm_level("auto").is_ok());
        assert!(dpm_level("high").is_ok());
        assert!(dpm_level("profile_peak").is_ok());
        assert!(dpm_level("turbo").is_err());
        assert!(dpm_level("").is_err());
    }

    #[test]
    fn vcache_modes() {
        assert!(vcache_mode("cache").is_ok());
        assert!(vcache_mode("frequency").is_ok());
        assert!(
            vcache_mode("freq").is_err(),
            "the driver spells it 'frequency'"
        );
        assert!(vcache_mode("none").is_err());
    }
}
