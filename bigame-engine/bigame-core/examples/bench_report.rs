//! Turn a benchmark session's run directories into the standard layout.
//!
//! Usage: `bench_report <session-dir> [baseline-arm]`
//!
//! Reads `<session-dir>/<arm>/run-NN/fps.txt`, one number per file, and writes
//! `system.json`, `benchmark.json`, `comparison.json`, `comparison.csv` and
//! `report.md` beside them.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bigame_core::benchmark::calibration::Calibration;
use bigame_core::benchmark::history::{Entry, History};
use bigame_core::benchmark::lab::Session;
use bigame_core::{hardware::Hardware, inventory};

/// Every `run-NN/fps.txt` under one arm, in run order.
fn arm_runs(dir: &Path) -> Vec<f64> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut runs: Vec<(String, f64)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let value = std::fs::read_to_string(e.path().join("fps.txt")).ok()?;
            Some((name, value.trim().parse().ok()?))
        })
        .collect();
    runs.sort_by(|a, b| a.0.cmp(&b.0));
    runs.into_iter().map(|(_, v)| v).collect()
}

fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("usage: bench_report <session-dir> [baseline-arm]"))?,
    );
    let baseline = std::env::args().nth(2).unwrap_or_else(|| "baseline".into());

    let mut arms = BTreeMap::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let runs = arm_runs(&entry.path());
        if !runs.is_empty() {
            arms.insert(name, runs);
        }
    }
    anyhow::ensure!(!arms.is_empty(), "no runs found under {}", dir.display());

    let hw = Hardware::detect();
    let workload = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let session = Session {
        schema: "bigame.benchmark/1".into(),
        workload: workload.clone(),
        metric: "avg_fps".into(),
        date: workload.split('-').take(3).collect::<Vec<_>>().join("-"),
        fingerprint: inventory::fingerprint(&hw),
        arms,
        warmup_runs: 1,
        alternating: true,
        caveats: std::fs::read_to_string(dir.join("caveats.txt"))
            .map(|t| {
                t.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };

    let comparisons = session.write_layout(&dir, &inventory::build(&hw), &baseline)?;
    println!("{}", session.to_markdown(&comparisons));

    // Check this session's baseline against the last comparable one, then
    // record it. This is what catches a machine getting slower on its own --
    // a kernel or driver update -- which no single A/B can see.
    if let Some(path) = History::default_path() {
        let mut history = History::load(&path)?;
        let entry = Entry {
            date: session.date.clone(),
            workload: session.workload.clone(),
            fingerprint: session.fingerprint.clone(),
            kernel: hw.kernel.clone(),
            runs: session.arms.get(&baseline).cloned().unwrap_or_default(),
        };
        println!("## Against previous sessions\n\n{}\n", history.check(&entry).describe());
        history.record(entry);
        history.save(&path)?;
    }

    // An isolation matrix is only worth running if its verdicts change what
    // the Booster does. Recording them as a calibration is what closes that
    // loop: each arm is one knob, and its verdict decides whether that knob is
    // applied on this machine.
    let mut calibration = Calibration::new(session.fingerprint.clone(), session.date.clone());
    for comparison in &comparisons {
        calibration.record(&session.workload, comparison);
    }
    calibration.save(&dir.join("calibration.json"))?;
    if let Some(path) = Calibration::default_path() {
        calibration.save(&path)?;
        println!("## Calibration\n\n{}\n", calibration.describe());
        for finding in calibration.harmful() {
            println!(
                "- `{}` is **not** applied on this machine: {}",
                finding.knob, finding.rationale
            );
        }
        for finding in calibration.beneficial() {
            println!("- `{}` is applied: {}", finding.knob, finding.rationale);
        }
        println!("\nSaved to {}", path.display());
    }
    Ok(())
}
