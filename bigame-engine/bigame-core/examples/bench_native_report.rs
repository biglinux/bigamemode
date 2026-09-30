//! Report a session measured with a game's own built-in benchmark.
//!
//! Usage: `bench_native_report <session-dir> [baseline-arm] [--vary=KEY,...]`
//!
//! Reads `<session-dir>/<arm>/run-NN/` as written by `scripts/bench-game.sh`:
//! the game's `*_frametimes_*.txt` and summary, and the `gpu.csv` sampled
//! alongside. Writes the standard layout (judged on average frame rate) plus
//! `metrics.md` and `metrics.json`, which judge the 1 % and 0.1 % lows the same
//! way and explain each arm with its clocks, power and temperature.
//!
//! Every run's graphics settings are compared with the first run's before
//! anything is computed. A session in which the game's settings changed is two
//! experiments, and it is refused rather than averaged.
// A report generator: one long linear main, text built by appending, and
// frame counts far below 2^52.
#![allow(
    clippy::too_many_lines,
    clippy::format_push_string,
    clippy::cast_precision_loss
)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bigame_core::benchmark::FrameStats;
use bigame_core::benchmark::lab::Session;
use bigame_core::benchmark::native::{self, NativeRun};
use bigame_core::{hardware::Hardware, inventory};
use serde::Serialize;

/// One measured run.
struct Run {
    name: String,
    native: NativeRun,
    stats: FrameStats,
    gpu: Option<GpuSummary>,
}

/// Means over a run's telemetry, idle samples excluded. A reading the card
/// does not report (the GTX 1050 Ti Mobile has no power sensor) is `None`.
#[derive(Debug, Clone, Serialize)]
struct GpuSummary {
    sclk_mhz: Option<f64>,
    power_w: Option<f64>,
    temp_c: Option<f64>,
    busy_pct: f64,
    cpu_pct: Option<f64>,
}

fn gpu_summary(csv: &Path) -> Option<GpuSummary> {
    let text = std::fs::read_to_string(csv).ok()?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next()?.split(',').collect();
    let col = |n: &str| header.iter().position(|h| *h == n);
    let (sclk, power, temp, busy) = (
        col("sclk_hz")?,
        col("power_uw")?,
        col("temp_mc")?,
        col("busy_pct")?,
    );
    let cpu = col("cpu_pct");
    // Per column: its sum and how many samples had it. Fields are kept by
    // position, so an empty one (a sensor the card lacks) leaves the others.
    let mut sums = [(0.0_f64, 0.0_f64); 5];
    let mut n = 0.0;
    for line in lines {
        let v: Vec<Option<f64>> = line.split(',').map(|x| x.parse().ok()).collect();
        if v.len() != header.len() {
            continue;
        }
        // Samples from the moments between passes, with the GPU idle, would
        // describe the menu rather than the benchmark.
        if !v[busy].is_some_and(|b| b >= 50.0) {
            continue;
        }
        n += 1.0;
        for (slot, (i, scale)) in [
            (Some(sclk), 1e6),
            (Some(power), 1e6),
            (Some(temp), 1e3),
            (Some(busy), 1.0),
            (cpu, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(x) = i.and_then(|i| v[i]) {
                sums[slot].0 += x / scale;
                sums[slot].1 += 1.0;
            }
        }
    }
    let mean = |slot: usize| (sums[slot].1 > 0.0).then(|| sums[slot].0 / sums[slot].1);
    (n > 0.0).then(|| GpuSummary {
        sclk_mhz: mean(0),
        power_w: mean(1),
        temp_c: mean(2),
        busy_pct: mean(3).unwrap_or_default(),
        cpu_pct: mean(4),
    })
}

fn read_run(dir: &Path) -> Result<Option<Run>> {
    // A Crystal Dynamics frame log beside the summary, or a Cyberpunk 2077
    // `benchmark_*` folder copied whole (frames.csv and summary.json).
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .collect();
    let crystal = entries
        .iter()
        .find(|p| p.to_string_lossy().contains("_frametimes_"));
    let cyberpunk = entries.iter().find(|p| {
        p.is_dir()
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("benchmark_"))
    });
    let (native, source) = match (crystal, cyberpunk) {
        (Some(f), _) => (native::read_crystal(f)?, f.clone()),
        (None, Some(d)) => (native::read_cyberpunk(d)?, d.clone()),
        (None, None) => return Ok(None),
    };
    let stats = native
        .capture
        .stats()
        .with_context(|| format!("{}: too few frames", source.display()))?;
    Ok(Some(Run {
        name: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        gpu: gpu_summary(&dir.join("gpu.csv")),
        native,
        stats,
    }))
}

/// A reading for the report: "—" where there is none.
fn shown(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "—".into(), |v| format!("{v:.decimals$}"))
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.collect();
    if v.is_empty() {
        None
    } else {
        Some(v.iter().sum::<f64>() / v.len() as f64)
    }
}

#[derive(Serialize)]
struct MetricReport {
    metric: String,
    comparisons: Vec<bigame_core::benchmark::result::Comparison>,
}

fn main() -> Result<()> {
    // `--vary=KEY,KEY`: game settings that may differ *between* arms because
    // they are what is being compared (an upscaler setting). Within an arm
    // every setting must still match, and across arms every other one.
    let vary: Vec<String> = std::env::args()
        .find_map(|a| a.strip_prefix("--vary=").map(str::to_owned))
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let positional: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();
    let dir = PathBuf::from(
        positional
            .first()
            .context("usage: bench_native_report <session-dir> [baseline-arm] [--vary=KEY,...]")?,
    );
    let baseline = positional
        .get(1)
        .cloned()
        .unwrap_or_else(|| "baseline".into());

    let mut arms: BTreeMap<String, Vec<Run>> = BTreeMap::new();
    for arm in std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.path().is_dir())
    {
        let name = arm.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let mut runs = Vec::new();
        // Run folders only: an arm may also keep files beside them (a
        // component's log from that session).
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(arm.path())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for run_dir in dirs {
            if let Some(run) = read_run(&run_dir)? {
                runs.push(run);
            }
        }
        if !runs.is_empty() {
            arms.insert(name, runs);
        }
    }
    anyhow::ensure!(!arms.is_empty(), "no runs found under {}", dir.display());

    // Same experiment throughout, or nothing — except the settings named in
    // --vary, which may differ between arms but not within one.
    let reference = &arms.values().next().unwrap()[0].native.settings;
    for (arm, runs) in &arms {
        let arm_reference = &runs[0].native.settings;
        for run in runs {
            let mut differ: Vec<String> = native::settings_differ(reference, &run.native.settings)
                .into_iter()
                .filter(|d| !vary.iter().any(|k| d.starts_with(&format!("{k}: "))))
                .collect();
            differ.extend(native::settings_differ(arm_reference, &run.native.settings));
            if !differ.is_empty() {
                bail!(
                    "{arm}/{}: the game's settings changed during the session ({}); \
                     that is a different experiment and cannot be compared",
                    run.name,
                    differ.join(", ")
                );
            }
            if run.native.frame_generation {
                bail!(
                    "{arm}/{}: frame generation was on; presented frames are not rendered frames",
                    run.name
                );
            }
        }
    }

    let hw = Hardware::detect();
    let workload = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let caveats: Vec<String> = std::fs::read_to_string(dir.join("caveats.txt"))
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let session = |metric: &str, pick: &dyn Fn(&FrameStats) -> Option<f64>| Session {
        schema: "bigame.benchmark/1".into(),
        workload: workload.clone(),
        metric: metric.into(),
        date: workload.split('-').take(3).collect::<Vec<_>>().join("-"),
        fingerprint: inventory::fingerprint(&hw),
        arms: arms
            .iter()
            .map(|(a, runs)| {
                (
                    a.clone(),
                    runs.iter().filter_map(|r| pick(&r.stats)).collect(),
                )
            })
            .collect(),
        warmup_runs: 1,
        alternating: true,
        caveats: caveats.clone(),
    };

    let avg = session("avg_fps", &|s| Some(s.avg_fps));
    let comparisons = avg.write_layout(&dir, &inventory::build(&hw), &baseline)?;
    let lows = [
        session("low_1_fps", &|s| Some(s.low_1_fps)),
        session("low_0_1_fps", &|s| s.low_0_1_fps),
    ];

    let mut md = format!("# {workload}: every metric\n\n");
    if !vary.is_empty() {
        md.push_str(&format!(
            "The arms differ by design in {} (the settings being compared); every \
             other setting is identical across all runs, and every setting is \
             identical within each arm.\n\n",
            vary.join(", ")
        ));
    }
    md.push_str(&format!(
        "Graphics settings identical across all {} runs: {} at {}x{}, VSync {}.\n\n",
        arms.values().map(Vec::len).sum::<usize>(),
        reference
            .get("AA")
            .map_or("?".into(), |a| format!("AA {a}")),
        reference.get("FullscreenWidth").map_or("?", String::as_str),
        reference
            .get("FullscreenHeight")
            .map_or("?", String::as_str),
        reference.get("VSync").map_or("?", String::as_str),
    ));
    md.push_str("## Per run\n\n| arm | run | avg fps | 1% low | 0.1% low | p99 ms | stutters | transitions | sclk MHz | power W | temp °C | GPU busy |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    for (arm, runs) in &arms {
        for r in runs {
            let g = r.gpu.as_ref();
            md.push_str(&format!(
                "| {arm} | {} | {:.1} | {:.1} | {} | {:.2} | {} | {} | {} | {} | {} | {} |\n",
                r.name,
                r.stats.avg_fps,
                r.stats.low_1_fps,
                r.stats
                    .low_0_1_fps
                    .map_or("—".into(), |v| format!("{v:.1}")),
                r.stats.p99_ms,
                r.stats.stutters,
                r.native.transitions,
                shown(g.and_then(|g| g.sclk_mhz), 0),
                shown(g.and_then(|g| g.power_w), 0),
                shown(g.and_then(|g| g.temp_c), 1),
                g.map_or("—".into(), |g| format!("{:.0}%", g.busy_pct)),
            ));
        }
    }
    md.push_str("\n## Per arm (telemetry means while the GPU was busy)\n\n| arm | sclk MHz | power W | temp °C |\n|---|---:|---:|---:|\n");
    for (arm, runs) in &arms {
        let gs: Vec<&GpuSummary> = runs.iter().filter_map(|r| r.gpu.as_ref()).collect();
        md.push_str(&format!(
            "| {arm} | {} | {} | {} |\n",
            shown(mean(gs.iter().filter_map(|g| g.sclk_mhz)), 0),
            shown(mean(gs.iter().filter_map(|g| g.power_w)), 0),
            shown(mean(gs.iter().filter_map(|g| g.temp_c)), 1),
        ));
    }
    let mut reports = vec![MetricReport {
        metric: "avg_fps".into(),
        comparisons: comparisons.clone(),
    }];
    for s in &lows {
        reports.push(MetricReport {
            metric: s.metric.clone(),
            comparisons: s.compare(&baseline),
        });
    }
    md.push_str(&format!("\n## Verdicts against `{baseline}`\n\n| metric | arm | mean → mean | change | verdict | why |\n|---|---|---|---:|---|---|\n"));
    for report in &reports {
        for c in &report.comparisons {
            md.push_str(&format!(
                "| {} | {} | {:.1} → {:.1} | {:+.1}% | {} | {} |\n",
                report.metric,
                c.candidate.arm,
                c.baseline.mean,
                c.candidate.mean,
                c.delta_pct,
                c.verdict.describe(),
                c.rationale
            ));
        }
    }
    std::fs::write(dir.join("metrics.md"), &md)?;
    std::fs::write(
        dir.join("metrics.json"),
        serde_json::to_string_pretty(&reports)?,
    )?;
    println!("{md}");
    Ok(())
}
