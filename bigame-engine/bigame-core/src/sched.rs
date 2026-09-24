//! Installed sched-ext schedulers.

/// Detect installed sched-ext schedulers by scanning `/usr/bin/scx_*`.
///
/// Returns a sorted list of scheduler names (e.g. `["bpfland", "lavd", "rusty"]`).
/// Always includes "none" as the first entry.
#[must_use]
pub fn detect_installed() -> Vec<String> {
    let mut schedulers = vec!["none".to_owned()];
    if let Ok(entries) = std::fs::read_dir("/usr/bin") {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(sched) = name.strip_prefix("scx_") {
                if !sched.is_empty() {
                    schedulers.push(sched.to_owned());
                }
            }
        }
    }
    schedulers.sort();
    schedulers.dedup();
    schedulers
}

#[cfg(test)]
mod tests {
    #[test]
    fn none_is_always_offered() {
        assert!(super::detect_installed().iter().any(|s| s == "none"));
    }
}
