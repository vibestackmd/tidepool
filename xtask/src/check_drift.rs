//! `check-drift` — structured comparison of the working tree's
//! fixtures + schemas against the committed versions. Does not hit
//! the network on its own; the typical flow is:
//!
//! 1. `cargo xtask record-helius` (writes fresh fixtures)
//! 2. `cargo xtask derive-schemas` (regenerates schemas)
//! 3. `cargo xtask check-drift`    (diffs vs. git + reports)
//!
//! Steps 1-3 together replace the ad-hoc bash in the weekly drift
//! workflow. Keeping the reporting here gives us a single source of
//! truth for what "drift" means, testable with `cargo test`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use clap::Parser;
use tracing::info;

#[derive(Parser)]
pub struct Args {
    /// Paths to compare. Defaults to the standard contract dirs —
    /// fixtures + schemas. Extend (or narrow) when a CI job wants
    /// to scope the check.
    #[arg(long, default_values_t = ["contracts/fixtures".to_string(), "contracts/schemas".to_string()])]
    paths: Vec<String>,
    /// When set, exit 0 even if drift is detected. Still prints the
    /// summary — useful for a local "just show me the diff" run.
    #[arg(long)]
    no_fail: bool,
}

#[derive(Debug, Default)]
pub struct DriftReport {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
}

impl DriftReport {
    pub fn has_drift(&self) -> bool {
        !(self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty())
    }

    pub fn total(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }
}

#[allow(clippy::unused_async)] // uniform async shape across xtask subcommands
pub async fn run(args: Args) -> anyhow::Result<()> {
    let report = detect_drift(&args.paths).context("running git status")?;

    if !report.has_drift() {
        info!("no drift — fixtures + schemas match the committed tree");
        return Ok(());
    }

    info!(
        added = report.added.len(),
        removed = report.removed.len(),
        modified = report.modified.len(),
        "drift detected"
    );
    for p in &report.added {
        info!(path = %p.display(), "added");
    }
    for p in &report.removed {
        info!(path = %p.display(), "removed");
    }
    for p in &report.modified {
        info!(path = %p.display(), "modified");
    }

    if args.no_fail {
        Ok(())
    } else {
        bail!("drift in {} file(s); commit or investigate", report.total());
    }
}

/// Parse `git status --porcelain=v1 -- <paths...>` into a structured
/// report. Works on any git repo root; callers don't need to pre-cd.
pub fn detect_drift(paths: &[String]) -> anyhow::Result<DriftReport> {
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain=v1")
        .arg("--")
        .args(paths)
        .output()
        .context("spawning git")?;
    if !output.status.success() {
        bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8(output.stdout).context("git output was not utf-8")?;
    let mut report = DriftReport::default();
    for line in stdout.lines() {
        // Each line is `XY path` where X/Y are single chars. See
        // git-status(1) for the full table; we only care about the
        // handful below.
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let path = Path::new(line[3..].trim()).to_path_buf();
        match status.trim() {
            "A" | "??" => report.added.push(path),
            "D" => report.removed.push(path),
            "M" | "MM" | "AM" => report.modified.push(path),
            // Anything else (renames, copies, index-only state) is
            // noise for drift detection at this layer.
            _ => {}
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_drift_matches_total() {
        let empty = DriftReport::default();
        assert!(!empty.has_drift());
        assert_eq!(empty.total(), 0);

        let filled = DriftReport {
            added: vec!["a".into()],
            removed: vec![],
            modified: vec!["b".into(), "c".into()],
        };
        assert!(filled.has_drift());
        assert_eq!(filled.total(), 3);
    }
}
