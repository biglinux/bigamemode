//! The standard result layout, and the report written from it.
//!
//! Every benchmark session lands in one directory with a fixed shape, so that a
//! result from six months ago can be read by the same code as one from today:
//!
//! ```text
//! benchmarks/2026-09-23-supertuxkart/
//!   system.json        the machine, as it was
//!   benchmark.json     what was run, and how
//!   baseline/run-01/   raw artifacts, one directory per run
//!   booster/run-01/
//!   comparison.json    the verdicts
//!   comparison.csv     the same, for a spreadsheet
//!   report.md          the same, for a person
//! ```
//!
//! The report is written to be read by someone deciding whether to trust it, so
//! it leads with the verdict and the evidence rather than a headline number,
//! and it states a regression in the same plain words it would use for a gain.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::result::{ArmSummary, Comparison, Verdict};

/// A complete benchmark session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Schema version, so an old result stays readable.
    pub schema: String,
    /// Which workload was run.
    pub workload: String,
    /// What the numbers are — `avg_fps`, `score`.
    pub metric: String,
    /// When, as an ISO date.
    pub date: String,
    /// Machine fingerprint, matching `system.json`.
    pub fingerprint: String,
    /// Measured runs per arm, in the order taken. Warm-up already excluded.
    pub arms: BTreeMap<String, Vec<f64>>,
    /// Runs discarded before measuring.
    pub warmup_runs: usize,
    /// Whether arms were interleaved rather than grouped.
    pub alternating: bool,
    /// Anything that qualifies the result.
    pub caveats: Vec<String>,
}

impl Session {
    /// Compare every arm against the named baseline.
    ///
    /// The baseline arm is not compared with itself.
    #[must_use]
    pub fn compare(&self, baseline_arm: &str) -> Vec<Comparison> {
        let Some(baseline) = self
            .arms
            .get(baseline_arm)
            .and_then(|r| ArmSummary::new(baseline_arm, r.clone()))
        else {
            return Vec::new();
        };
        self.arms
            .iter()
            .filter(|(name, _)| name.as_str() != baseline_arm)
            .filter_map(|(name, runs)| {
                let candidate = ArmSummary::new(name.clone(), runs.clone())?;
                Some(Comparison::new(&self.metric, baseline.clone(), candidate))
            })
            .collect()
    }

    /// Write the whole layout to `dir`, which must already hold the run
    /// artifacts.
    ///
    /// # Errors
    /// Returns an error if any file cannot be written.
    pub fn write_layout(
        &self,
        dir: &Path,
        system: &Value,
        baseline_arm: &str,
    ) -> Result<Vec<Comparison>> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let comparisons = self.compare(baseline_arm);

        write_json(&dir.join("system.json"), system)?;
        write_json(&dir.join("benchmark.json"), self)?;
        write_json(&dir.join("comparison.json"), &comparisons)?;
        std::fs::write(dir.join("comparison.csv"), self.to_csv(&comparisons))
            .context("write comparison.csv")?;
        std::fs::write(dir.join("report.md"), self.to_markdown(&comparisons))
            .context("write report.md")?;
        std::fs::write(dir.join("report.html"), self.to_html(&comparisons))
            .context("write report.html")?;
        Ok(comparisons)
    }

    /// The report as a self-contained HTML page.
    ///
    /// Charts are inline SVG with no script and no external request: a report
    /// is evidence, and evidence that phones home or needs a network to render
    /// is not evidence anyone should have to trust. It also means the file
    /// still works in five years, attached to an email, opened offline.
    ///
    /// Every run is plotted individually rather than only its arm's mean,
    /// because the spread is the part that decides whether the difference
    /// means anything, and a bar chart of two averages hides exactly that.
    #[must_use]
    pub fn to_html(&self, comparisons: &[Comparison]) -> String {
        let mut out = String::new();
        out.push_str(
            "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        );
        let _ = writeln!(
            out,
            "<title>{} — {}</title>",
            esc(&self.workload),
            self.date
        );
        out.push_str(STYLE);
        let _ = writeln!(
            out,
            "<h1>{} <small>{}</small></h1>",
            esc(&self.workload),
            self.date
        );

        if comparisons.is_empty() {
            out.push_str(
                "<p class=\"verdict none\"><strong>NOT TESTED.</strong> No arm produced \
                 enough runs to compare. This is recorded rather than omitted, so that a \
                 missing result is visibly missing.</p>\n</html>\n",
            );
            return out;
        }

        out.push_str("<h2>Verdict</h2>\n");
        for c in comparisons {
            let (class, word) = match c.verdict {
                Verdict::Improvement => ("good", "FASTER"),
                Verdict::Regression => ("bad", "SLOWER"),
                Verdict::WithinNoise => ("none", "NO CHANGE"),
                Verdict::Inconclusive => ("unknown", "INCONCLUSIVE"),
            };
            let _ = writeln!(
                out,
                "<p class=\"verdict {class}\"><b>{}</b> — <b>{word}</b>: {}</p>",
                esc(&c.candidate.arm),
                esc(&c.rationale)
            );
        }

        out.push_str("<h2>Every run</h2>\n");
        out.push_str(&chart(comparisons));

        out.push_str(&html_table(comparisons));

        if !self.caveats.is_empty() {
            out.push_str("<h2>Caveats</h2>\n<ul>\n");
            for caveat in &self.caveats {
                let _ = writeln!(out, "<li>{}", esc(caveat));
            }
            out.push_str("</ul>\n");
        }

        out.push_str("<h2>Method</h2>\n<ul>\n");
        let _ = writeln!(
            out,
            "<li>{} measured runs per arm, {} warm-up discarded.",
            comparisons.first().map_or(0, |c| c.baseline.runs.len()),
            self.warmup_runs
        );
        out.push_str(if self.alternating {
            "<li>Arms were alternated, so drift over the session falls on both equally.\n"
        } else {
            "<li><b>Arms were grouped, not alternated.</b> Drift is confounded with the \
             configuration; treat with caution.\n"
        });
        out.push_str(
            "<li>A difference counts as real only when it exceeds the run-to-run spread of \
             both arms and passes Welch's t-test at 95%.\n",
        );
        let _ = writeln!(
            out,
            "<li>Machine fingerprint <code>{}</code>.",
            esc(&self.fingerprint)
        );
        out.push_str("</ul>\n</html>\n");
        out
    }

    /// The comparison as CSV, one row per arm.
    #[must_use]
    pub fn to_csv(&self, comparisons: &[Comparison]) -> String {
        let mut out =
            String::from("arm,runs,mean,median,min,max,stddev,cov_pct,delta_pct,verdict\n");
        // The baseline first, with no delta of its own to report.
        if let Some(first) = comparisons.first() {
            let b = &first.baseline;
            let _ = writeln!(
                out,
                "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},,baseline",
                b.arm,
                b.runs.len(),
                b.mean,
                b.median,
                b.min,
                b.max,
                b.stddev,
                b.cov * 100.0
            );
        }
        for c in comparisons {
            let a = &c.candidate;
            let _ = writeln!(
                out,
                "{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:+.2},{:?}",
                a.arm,
                a.runs.len(),
                a.mean,
                a.median,
                a.min,
                a.max,
                a.stddev,
                a.cov * 100.0,
                c.delta_pct,
                c.verdict
            );
        }
        out
    }

    /// The report a person reads.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn to_markdown(&self, comparisons: &[Comparison]) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# {} — {}\n", self.workload, self.date);

        if comparisons.is_empty() {
            out.push_str(
                "**NOT TESTED.** No arm produced enough runs to compare.\n\n\
                 This is recorded rather than omitted, so that a missing result \
                 is visibly missing rather than silently absent.\n",
            );
            return out;
        }

        // Lead with the verdict, because it is what the reader came for and
        // burying it under a table invites skipping to the biggest number.
        out.push_str("## Verdict\n\n");
        for c in comparisons {
            let mark = match c.verdict {
                Verdict::Improvement => "FASTER",
                Verdict::Regression => "SLOWER",
                Verdict::WithinNoise => "NO CHANGE",
                Verdict::Inconclusive => "INCONCLUSIVE",
            };
            let _ = writeln!(out, "- **{}** — {}: {}", c.candidate.arm, mark, c.rationale);
        }

        out.push_str("\n## Measurements\n\n");
        let _ = write!(
            out,
            "| Arm | Runs | Mean {m} | Median | Min | Max | Spread | vs baseline |\n\
             |---|---:|---:|---:|---:|---:|---:|---:|\n",
            m = self.metric
        );
        if let Some(first) = comparisons.first() {
            let b = &first.baseline;
            let _ = writeln!(
                out,
                "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1}% | — |",
                b.arm,
                b.runs.len(),
                b.mean,
                b.median,
                b.min,
                b.max,
                b.cov * 100.0
            );
        }
        for c in comparisons {
            let a = &c.candidate;
            // A percentage is quoted only when the verdict supports it.
            let delta = match c.verdict {
                Verdict::Improvement | Verdict::Regression => format!("{:+.1}%", c.delta_pct),
                Verdict::WithinNoise => "within noise".into(),
                Verdict::Inconclusive => "inconclusive".into(),
            };
            let _ = writeln!(
                out,
                "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1}% | {} |",
                a.arm,
                a.runs.len(),
                a.mean,
                a.median,
                a.min,
                a.max,
                a.cov * 100.0,
                delta
            );
        }

        out.push_str("\n## Method\n\n");
        let _ = writeln!(
            out,
            "- {} measured run(s) per arm, {} warm-up run(s) discarded.",
            comparisons.first().map_or(0, |c| c.baseline.runs.len()),
            self.warmup_runs
        );
        out.push_str(if self.alternating {
            "- Arms were alternated (A B A B …) rather than grouped, so that drift \
             over the session — chassis temperature above all — falls on both arms \
             equally instead of on whichever ran last.\n"
        } else {
            "- **Arms were run in groups, not alternated.** Any drift over the \
             session is confounded with the configuration; treat the result with \
             corresponding caution.\n"
        });
        out.push_str(
            "- A difference is called real only when it exceeds the run-to-run \
             spread of both arms *and* passes Welch's t-test at 95%. Anything \
             smaller is reported as no change, not as a small gain.\n",
        );
        let _ = writeln!(out, "- Machine fingerprint `{}`.", self.fingerprint);

        if !self.caveats.is_empty() {
            out.push_str("\n## Caveats\n\n");
            for caveat in &self.caveats {
                let _ = writeln!(out, "- {caveat}");
            }
        }

        out.push_str("\n## Raw runs\n\n");
        for (arm, runs) in &self.arms {
            let formatted: Vec<String> = runs.iter().map(|r| format!("{r:.1}")).collect();
            let _ = writeln!(out, "- `{arm}`: {}", formatted.join(", "));
        }
        out
    }
}

/// Chart geometry, in SVG user units.
const WIDTH: f64 = 720.0;
/// Space reserved on the left for arm labels.
const LEFT: f64 = 130.0;
/// Vertical distance between arms.
const ROW: f64 = 44.0;

/// The measurements table.
fn html_table(comparisons: &[Comparison]) -> String {
    let mut out = String::new();
    out.push_str(
        "<h2>Measurements</h2>\n<table>\n<tr><th>Arm<th>Runs<th>Mean<th>Median\
                  <th>Min<th>Max<th>Spread<th>vs baseline\n",
    );
    if let Some(first) = comparisons.first() {
        let b = &first.baseline;
        let _ = writeln!(
            out,
            "<tr><td>{}<td>{}<td>{:.1}<td>{:.1}<td>{:.1}<td>{:.1}<td>{:.1}%<td>—",
            esc(&b.arm),
            b.runs.len(),
            b.mean,
            b.median,
            b.min,
            b.max,
            b.cov * 100.0
        );
    }
    for c in comparisons {
        let a = &c.candidate;
        let delta = match c.verdict {
            Verdict::Improvement | Verdict::Regression => format!("{:+.1}%", c.delta_pct),
            Verdict::WithinNoise => "within noise".into(),
            Verdict::Inconclusive => "inconclusive".into(),
        };
        let _ = writeln!(
            out,
            "<tr><td>{}<td>{}<td>{:.1}<td>{:.1}<td>{:.1}<td>{:.1}<td>{:.1}%<td>{delta}",
            esc(&a.arm),
            a.runs.len(),
            a.mean,
            a.median,
            a.min,
            a.max,
            a.cov * 100.0
        );
    }
    out.push_str("</table>\n");
    out
}

/// A dot plot of every run, one row per arm.
///
/// Every run is plotted individually rather than only its arm's mean, because
/// the spread is what decides whether a difference means anything, and a bar
/// chart of two averages hides exactly that.
fn chart(comparisons: &[Comparison]) -> String {
    // Collect the arms in the order the table shows them.
    let mut arms: Vec<&ArmSummary> = Vec::new();
    if let Some(first) = comparisons.first() {
        arms.push(&first.baseline);
    }
    arms.extend(comparisons.iter().map(|c| &c.candidate));

    let all: Vec<f64> = arms.iter().flat_map(|a| a.runs.iter().copied()).collect();
    let (lo, hi) = match (
        all.iter().copied().fold(f64::INFINITY, f64::min),
        all.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ) {
        (lo, hi) if lo.is_finite() && hi > lo => (lo, hi),
        _ => return String::new(),
    };
    // Pad the axis so points never sit on the frame.
    let pad = (hi - lo) * 0.15;
    let (lo, hi) = (lo - pad, hi + pad);

    #[allow(clippy::cast_precision_loss)]
    let height = ROW * arms.len() as f64 + 34.0;
    let x = |v: f64| LEFT + (v - lo) / (hi - lo) * (WIDTH - LEFT - 20.0);

    let mut svg = format!(
        "<svg viewBox=\"0 0 {WIDTH} {height}\" width=\"100%\" \
         role=\"img\" aria-label=\"every measured run, by configuration\">\n"
    );
    for (i, arm) in arms.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let y = ROW * i as f64 + 24.0;
        let _ = writeln!(
            svg,
            "<text x=\"{}\" y=\"{:.0}\" class=\"lbl\" text-anchor=\"end\">{}</text>",
            LEFT - 10.0,
            y + 4.0,
            esc(&arm.arm)
        );
        // The span of the arm's runs, drawn behind the points.
        let _ = writeln!(
            svg,
            "<line x1=\"{:.1}\" y1=\"{y:.0}\" x2=\"{:.1}\" y2=\"{y:.0}\" class=\"span\"/>",
            x(arm.min),
            x(arm.max)
        );
        for run in &arm.runs {
            let _ = writeln!(
                svg,
                "<circle cx=\"{:.1}\" cy=\"{y:.0}\" r=\"4\" class=\"pt\"><title>{run:.1}\
                 </title></circle>",
                x(*run)
            );
        }
        // The mean, as the only thing drawn differently.
        let _ = writeln!(
            svg,
            "<line x1=\"{:.1}\" y1=\"{:.0}\" x2=\"{:.1}\" y2=\"{:.0}\" class=\"mean\">\
             <title>mean {:.1}</title></line>",
            x(arm.mean),
            y - 11.0,
            x(arm.mean),
            y + 11.0,
            arm.mean
        );
    }
    // Axis ends, so the numbers are readable without a gridline thicket.
    let _ = writeln!(
        svg,
        "<text x=\"{LEFT}\" y=\"{:.0}\" class=\"ax\">{lo:.0}</text>\
         <text x=\"{WIDTH}\" y=\"{:.0}\" class=\"ax\" text-anchor=\"end\">{hi:.0}</text>",
        height - 8.0,
        height - 8.0
    );
    svg.push_str("</svg>\n");
    svg
}

/// The report's stylesheet. Inline, because a report has to render offline.
const STYLE: &str = "<style>\
body{font:15px/1.6 system-ui,sans-serif;max-width:56rem;margin:2rem auto;padding:0 1rem;\
color:#1a1a1a;background:#fff}\
h1{font-size:1.6rem;margin-bottom:.2rem}h1 small{font-weight:400;color:#666;font-size:1rem}\
h2{font-size:1.1rem;margin-top:2rem;border-bottom:1px solid #ddd;padding-bottom:.3rem}\
table{border-collapse:collapse;width:100%}th,td{text-align:right;padding:.4rem .6rem;\
border-bottom:1px solid #eee}th:first-child,td:first-child{text-align:left}\
th{font-weight:600;color:#555}\
.verdict{padding:.6rem .8rem;border-left:4px solid #bbb;background:#fafafa;margin:.5rem 0}\
.verdict.good{border-color:#2e7d32}.verdict.bad{border-color:#c62828}\
.verdict.unknown{border-color:#ef6c00}\
svg{max-width:100%;height:auto;margin:1rem 0}\
.lbl{font:13px system-ui,sans-serif;fill:#333}.ax{font:11px system-ui,sans-serif;fill:#888}\
.span{stroke:#ccc;stroke-width:2}.pt{fill:#4a6fa5;fill-opacity:.75}\
.mean{stroke:#c62828;stroke-width:2}\
code{background:#f3f3f3;padding:.1rem .3rem;border-radius:3px}\
@media(prefers-color-scheme:dark){body{background:#16181c;color:#e6e6e6}\
h2{border-color:#333}td,th{border-color:#2a2d33}th{color:#aaa}\
.verdict{background:#1e2126;border-color:#555}.lbl{fill:#ddd}.span{stroke:#444}\
.pt{fill:#7aa2d6}code{background:#24272d}}\
</style>\n";

/// Escape text for HTML.
///
/// Arm names and rationales are generated here rather than typed by a user, but
/// a caveat file is read from disk and a workload name comes from a directory
/// name, so neither is guaranteed safe to interpolate raw.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .with_context(|| format!("serialise {}", path.display()))?;
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut arms = BTreeMap::new();
        arms.insert("baseline".to_owned(), vec![400.0, 402.0, 398.0]);
        arms.insert("booster".to_owned(), vec![520.0, 524.0, 518.0]);
        Session {
            schema: "bigame.benchmark/1".into(),
            workload: "SuperTuxKart".into(),
            metric: "avg_fps".into(),
            date: "2026-09-23".into(),
            fingerprint: "0123456789abcdef".into(),
            arms,
            warmup_runs: 1,
            alternating: true,
            caveats: vec!["The frame cap was lifted for this test.".into()],
        }
    }

    #[test]
    fn the_baseline_is_not_compared_with_itself() {
        let comparisons = session().compare("baseline");
        assert_eq!(comparisons.len(), 1);
        assert_eq!(comparisons[0].candidate.arm, "booster");
    }

    #[test]
    fn an_unknown_baseline_yields_nothing_rather_than_a_wrong_answer() {
        assert!(session().compare("no-such-arm").is_empty());
    }

    #[test]
    fn a_gain_reaches_the_report_with_its_percentage() {
        let s = session();
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("FASTER"));
        // 520.667 against 400.0 is +30.2%.
        assert!(
            md.contains("+30.2%"),
            "the report must quote the measured delta"
        );
        assert!(md.contains("alternated"));
    }

    #[test]
    fn a_difference_within_noise_is_never_quoted_as_a_percentage() {
        let mut s = session();
        s.arms.insert("booster".into(), vec![405.0, 399.0, 403.0]);
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("NO CHANGE"));
        assert!(md.contains("within noise"));
        // The whole point: no "+0.5%" anywhere in the comparison column.
        assert!(
            !md.contains("| +"),
            "a noise-level delta must not be shown as a gain"
        );
    }

    #[test]
    fn a_regression_is_stated_as_plainly_as_a_gain() {
        let mut s = session();
        s.arms.insert("booster".into(), vec![300.0, 302.0, 298.0]);
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("SLOWER"));
        assert!(
            md.contains("-25"),
            "the size of the loss is reported, not hidden"
        );
    }

    #[test]
    fn an_empty_session_says_not_tested() {
        let mut s = session();
        s.arms.clear();
        let md = s.to_markdown(&[]);
        assert!(md.contains("NOT TESTED"));
    }

    #[test]
    fn grouped_runs_are_flagged_as_weaker_evidence() {
        let mut s = session();
        s.alternating = false;
        let c = s.compare("baseline");
        let md = s.to_markdown(&c);
        assert!(md.contains("not alternated"));
        assert!(md.contains("caution"));
    }

    #[test]
    fn csv_carries_every_arm_including_the_baseline() {
        let s = session();
        let csv = s.to_csv(&s.compare("baseline"));
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header plus both arms");
        assert!(lines[0].starts_with("arm,runs,mean"));
        assert!(lines[1].starts_with("baseline,3,"));
        assert!(lines[2].starts_with("booster,3,"));
        assert!(lines[2].contains("Improvement"));
    }

    #[test]
    fn the_layout_is_written_whole() {
        let dir = std::env::temp_dir().join(format!("bigame_lab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let s = session();
        let system = serde_json::json!({"schema": "bigame.system/1"});
        let comparisons = s.write_layout(&dir, &system, "baseline").unwrap();
        assert_eq!(comparisons.len(), 1);

        for name in [
            "system.json",
            "benchmark.json",
            "comparison.json",
            "comparison.csv",
            "report.md",
            "report.html",
        ] {
            assert!(dir.join(name).is_file(), "{name} was not written");
        }

        // The session must survive a round trip, or an old result becomes
        // unreadable by a later build.
        let text = std::fs::read_to_string(dir.join("benchmark.json")).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.arms, s.arms);
        assert_eq!(back.workload, "SuperTuxKart");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_html_report_is_self_contained_and_escaped() {
        let mut s = session();
        s.caveats = vec!["a <script>alert(1)</script> & an ampersand".into()];
        let c = s.compare("baseline");
        let html = s.to_html(&c);

        // Nothing fetched, nothing executed.
        assert!(
            !html.contains("<script"),
            "the caveat must be escaped, not run"
        );
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&amp; an ampersand"));

        // The chart plots each individual run, not just the means.
        assert_eq!(
            html.matches("<circle").count(),
            6,
            "3 runs in each of 2 arms"
        );
        assert!(html.contains("FASTER"));
    }

    #[test]
    fn the_html_report_withholds_a_percentage_within_noise() {
        let mut s = session();
        s.arms.insert("booster".into(), vec![405.0, 399.0, 403.0]);
        let html = s.to_html(&s.compare("baseline"));
        assert!(html.contains("NO CHANGE"));
        assert!(html.contains("within noise"));
    }

    #[test]
    fn an_empty_html_report_says_not_tested() {
        let mut s = session();
        s.arms.clear();
        assert!(s.to_html(&[]).contains("NOT TESTED"));
    }

    #[test]
    fn caveats_reach_the_report() {
        let s = session();
        let md = s.to_markdown(&s.compare("baseline"));
        assert!(md.contains("frame cap was lifted"));
    }
}
