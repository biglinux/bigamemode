//! Report a session measured with a game's own built-in benchmark.
//!
//! Usage: `bench_native_report <session-dir> [baseline-arm]`
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
//!
//! `--record-graphics=<game-key> --setups=<arm>=<setup>,…` also records the
//! arms in AI Graphics' local measurements, where the game's plan reads them
//! (`setup`: `none`, `native:xess`, `optiscaler:xess:fsr`; the `OptiScaler`
//! version with `--optiscaler=0.9.4`).
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
use bigame_core::benchmark::calibration::Calibration;
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

/// Means over a run's telemetry, idle samples excluded.
#[derive(Debug, Clone, Serialize)]
struct GpuSummary {
    sclk_mhz: f64,
    power_w: f64,
    temp_c: f64,
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
    let mut sums = [0.0_f64; 5];
    let mut n = 0.0;
    for line in lines {
        let v: Vec<f64> = line.split(',').filter_map(|x| x.parse().ok()).collect();
        if v.len() != header.len() {
            continue;
        }
        // Samples from the moments between passes, with the GPU idle, would
        // describe the menu rather than the benchmark.
        if v[busy] < 50.0 {
            continue;
        }
        sums[0] += v[sclk] / 1e6;
        sums[1] += v[power] / 1e6;
        sums[2] += v[temp] / 1e3;
        sums[3] += v[busy];
        sums[4] += cpu.map_or(0.0, |i| v[i]);
        n += 1.0;
    }
    (n > 0.0).then(|| GpuSummary {
        sclk_mhz: sums[0] / n,
        power_w: sums[1] / n,
        temp_c: sums[2] / n,
        busy_pct: sums[3] / n,
        cpu_pct: cpu.map(|_| sums[4] / n),
    })
}

fn read_run(dir: &Path) -> Result<Option<Run>> {
    let Some(frametimes) = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains("_frametimes_"))
    else {
        return Ok(None);
    };
    let native = native::read_crystal(&frametimes)?;
    let stats = native
        .capture
        .stats()
        .with_context(|| format!("{}: too few frames", frametimes.display()))?;
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

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = values.collect();
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
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
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(arm.path())?
            .flatten()
            .map(|e| e.path())
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
    // The machine a session was first reported on, before this run rewrites
    // benchmark.json: measurements are recorded only on the machine that took
    // them, under its own GPU.
    let measured_on: Option<String> = std::fs::read_to_string(dir.join("benchmark.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("fingerprint")?.as_str().map(str::to_owned));
    let record_game =
        std::env::args().find_map(|a| a.strip_prefix("--record-graphics=").map(str::to_owned));
    if record_game.is_some() {
        let here = inventory::fingerprint(&hw);
        if let Some(there) = measured_on.as_deref().filter(|f| *f != here) {
            bail!(
                "this session was measured on another machine (fingerprint {there}, this one is \
                 {here}); its results are not recorded under this machine's GPU"
            );
        }
    }
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
                g.map_or("—".into(), |g| format!("{:.0}", g.sclk_mhz)),
                g.map_or("—".into(), |g| format!("{:.0}", g.power_w)),
                g.map_or("—".into(), |g| format!("{:.1}", g.temp_c)),
                g.map_or("—".into(), |g| format!("{:.0}%", g.busy_pct)),
            ));
        }
    }
    md.push_str("\n## Per arm (telemetry means while the GPU was busy)\n\n| arm | sclk MHz | power W | temp °C |\n|---|---:|---:|---:|\n");
    for (arm, runs) in &arms {
        let gs: Vec<&GpuSummary> = runs.iter().filter_map(|r| r.gpu.as_ref()).collect();
        md.push_str(&format!(
            "| {arm} | {:.0} | {:.0} | {:.1} |\n",
            mean(gs.iter().map(|g| g.sclk_mhz)),
            mean(gs.iter().map(|g| g.power_w)),
            mean(gs.iter().map(|g| g.temp_c)),
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
    // Arms that isolate one knob feed the Booster's calibration, merged into
    // what earlier sessions found rather than replacing it. Only the frame
    // rate verdict is recorded, as `bench_report` does, so a knob is judged on
    // the same metric whichever workload measured it.
    let knob_arms = ["cpu_governor", "gpu_dpm_level"];
    if let Some(path) = Calibration::default_path() {
        let fingerprint = inventory::fingerprint(&hw);
        let mut calibration = Calibration::load(&path, &fingerprint)?
            .unwrap_or_else(|| Calibration::new(fingerprint.clone(), avg.date.clone()));
        let mut recorded = Vec::new();
        for c in comparisons
            .iter()
            .filter(|c| knob_arms.contains(&c.candidate.arm.as_str()))
        {
            calibration.record(&workload, c);
            recorded.push(c.candidate.arm.clone());
        }
        if !recorded.is_empty() {
            calibration.save(&path)?;
            md.push_str(&format!(
                "\n## Calibration\n\nRecorded for this machine: {}.\n\n{}\n",
                recorded.join(", "),
                calibration.describe()
            ));
        }
    }
    std::fs::write(dir.join("metrics.md"), &md)?;
    std::fs::write(
        dir.join("metrics.json"),
        serde_json::to_string_pretty(&reports)?,
    )?;
    println!("{md}");
    if let Some(game) = record_game {
        use bigame_core::graphics::outcomes::{self, Measurement, Setup};
        let setups: BTreeMap<String, Setup> = std::env::args()
            .find_map(|a| a.strip_prefix("--setups=").map(str::to_owned))
            .context("--record-graphics needs --setups=<arm>=<setup>,...")?
            .split(',')
            .map(|pair| {
                let (arm, setup) = pair.split_once('=').context("--setups: <arm>=<setup>")?;
                let setup = Setup::parse(setup).with_context(|| format!("unknown setup {setup:?}"))?;
                anyhow::ensure!(arms.contains_key(arm), "--setups names arm {arm:?}, which has no runs");
                Ok((arm.to_owned(), setup))
            })
            .collect::<Result<_>>()?;
        let optiscaler = std::env::args().find_map(|a| a.strip_prefix("--optiscaler=").map(str::to_owned));
        let gpu = bigame_core::graphics::report::render_gpu_name(&hw).context("no GPU")?;
        let resolution = [("FullscreenWidth", "FullscreenHeight"), ("renderWidth", "renderHeight")]
            .iter()
            .find_map(|(w, h)| reference.get(*w).zip(reference.get(*h)))
            .map(|(w, h)| format!("{w}x{h}"));
        let date = workload.split('-').take(3).collect::<Vec<_>>().join("-");
        let measurements: Vec<Measurement> = setups
            .into_iter()
            .map(|(arm, setup)| Measurement {
                date: date.clone(),
                game: game.clone(),
                gpu: gpu.clone(),
                optiscaler_version: matches!(setup, Setup::OptiScaler { .. })
                    .then(|| optiscaler.clone())
                    .flatten(),
                setup,
                resolution: resolution.clone(),
                avg_fps: arms[&arm].iter().map(|r| r.stats.avg_fps).collect(),
                low_1pct: arms[&arm].iter().map(|r| r.stats.low_1_fps).collect(),
            })
            .collect();
        outcomes::record(&outcomes::path(), &measurements)?;
        println!("recorded {} arms for {game} on {gpu}", measurements.len());
    }
    Ok(())
}
