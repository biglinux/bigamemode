//! The sched-ext schedulers a profile can ask for.

/// Schedulers a falcond profile can name, with `"none"` first.
///
/// falcond lists the schedulers it can switch to in its status
/// (`AVAILABLE_SCX_SCHEDULERS`); only those that are also installed are
/// offered. Every `/usr/bin/scx_*` is not a choice: `scx_loader` is the loader
/// itself, and scx-scheds ships schedulers (flow, forge, mlfq, pandemonium,
/// chaos, layered on the lab laptop) that falcond 2.0.2 does not know, so a
/// profile naming one does not get it. Without a status to read, the installed
/// binaries are offered, less the loader.
#[must_use]
pub fn detect_installed() -> Vec<String> {
    let installed = installed_binaries(std::path::Path::new("/usr/bin"));
    let known = crate::status::read()
        .map(|s| s.available_scx)
        .unwrap_or_default();
    choices(&installed, &known)
}

fn installed_binaries(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    name.strip_prefix("scx_").map(str::to_owned)
                })
                .filter(|n| !n.is_empty() && n != "loader")
                .collect()
        })
        .unwrap_or_default()
}

/// `"none"`, then the installed schedulers falcond knows (all installed ones
/// when falcond's list is unknown), sorted.
fn choices(installed: &[String], falcond_knows: &[String]) -> Vec<String> {
    let mut list: Vec<String> = installed
        .iter()
        .filter(|n| {
            falcond_knows.is_empty()
                || falcond_knows
                    .iter()
                    .any(|k| k.strip_prefix("scx_").unwrap_or(k) == n.as_str())
        })
        .cloned()
        .collect();
    list.sort();
    list.dedup();
    list.insert(0, "none".to_owned());
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn only_schedulers_falcond_knows_and_that_are_installed_are_offered() {
        // The lab laptop: scx-scheds 1.1.3, falcond 2.0.2.
        let installed = v(&[
            "beerland", "bpfland", "chaos", "flow", "lavd", "layered", "mlfq", "p2dq",
        ]);
        let falcond = v(&[
            "scx_beerland",
            "scx_bpfland",
            "scx_lavd",
            "scx_p2dq",
            "scx_rusty",
        ]);
        assert_eq!(
            choices(&installed, &falcond),
            v(&["none", "beerland", "bpfland", "lavd", "p2dq"])
        );
    }

    #[test]
    fn without_falcond_s_list_the_installed_ones_are_offered_but_never_the_loader() {
        let d = tempfile::tempdir().unwrap();
        for f in ["scx_loader", "scx_lavd", "scx_bpfland", "other"] {
            std::fs::write(d.path().join(f), "").unwrap();
        }
        let installed = installed_binaries(d.path());
        assert_eq!(choices(&installed, &[]), v(&["none", "bpfland", "lavd"]));
    }
}
