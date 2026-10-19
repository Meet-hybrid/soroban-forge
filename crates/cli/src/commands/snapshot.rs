use crate::cli::SnapshotArgs;
use crate::commands::test::{exit_code_for_status, format_summary_line, test_args};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMode {
    Check,
    Update,
}

impl SnapshotMode {
    pub fn from_args(args: &SnapshotArgs) -> Result<Self> {
        if args.check && args.update {
            bail!("cannot pass both --check and --update to `soroban-forge snapshot`");
        }
        if args.update {
            Ok(Self::Update)
        } else {
            Ok(Self::Check)
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Update => "update",
        }
    }

    pub const fn insta_update_value(self) -> &'static str {
        match self {
            Self::Check => "no",
            Self::Update => "always",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub mode: SnapshotMode,
    pub passed: usize,
    pub modified: usize,
    pub failed: usize,
}

impl SnapshotSummary {
    pub fn render_terminal_summary(&self) -> String {
        let label = format!("Snapshot summary ({})", self.mode.as_str());
        format_summary_line(&label, self.passed, self.modified, self.failed)
    }
}

pub fn run(args: SnapshotArgs) -> Result<()> {
    let workspace_root = std::env::current_dir().context("failed to resolve current directory")?;
    let status = run_with_command("cargo", &args, &workspace_root)?;
    if status != 0 {
        std::process::exit(status);
    }
    Ok(())
}

pub fn snapshot_cargo_args(package: Option<&str>) -> Vec<String> {
    test_args(package)
}

pub fn snapshot_env_vars(mode: SnapshotMode) -> Vec<(&'static str, &'static str)> {
    vec![
        ("INSTA_UPDATE", mode.insta_update_value()),
        ("SOROBAN_SNAPSHOT_MODE", mode.as_str()),
        (
            "TRYBUILD",
            match mode {
                SnapshotMode::Check => "verify",
                SnapshotMode::Update => "overwrite",
            },
        ),
    ]
}

pub fn run_with_command(program: &str, args: &SnapshotArgs, workspace_root: &Path) -> Result<i32> {
    let mode = SnapshotMode::from_args(args)?;
    let before = collect_snapshot_hashes(workspace_root, args.package.as_deref())?;

    let mut cmd = Command::new(program);
    cmd.current_dir(workspace_root);
    cmd.args(snapshot_cargo_args(args.package.as_deref()));
    for (key, value) in snapshot_env_vars(mode) {
        cmd.env(key, value);
    }

    let output = cmd
        .output()
        .with_context(|| format!("failed to run `{program}` for snapshot testing"))?;
    let after = collect_snapshot_hashes(workspace_root, args.package.as_deref())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    let summary = evaluate_snapshot_run(mode, &before, &after, &output);
    println!("{}", summary.render_terminal_summary());

    let mut code = exit_code_for_status(&output.status);
    if summary.failed > 0 && code == 0 {
        code = 1;
    }
    Ok(code)
}

pub fn collect_snapshot_hashes(
    workspace_root: &Path,
    package: Option<&str>,
) -> Result<BTreeMap<PathBuf, String>> {
    let crates_dir = workspace_root.join("crates");
    let mut hashes = BTreeMap::new();
    if !crates_dir.is_dir() {
        return Ok(hashes);
    }

    let package_dir_filter =
        package.map(|pkg| pkg.strip_prefix("soroban-forge-").unwrap_or(pkg).to_owned());

    for entry in fs::read_dir(&crates_dir)
        .with_context(|| format!("failed to read crates directory {}", crates_dir.display()))?
    {
        let entry = entry.context("failed to read crate directory entry")?;
        let crate_path = entry.path();
        if !crate_path.is_dir() {
            continue;
        }
        let crate_name = entry.file_name().to_string_lossy().into_owned();
        if let Some(expected) = package_dir_filter.as_deref() {
            if crate_name != expected && Some(crate_name.as_str()) != package {
                continue;
            }
        }

        let snapshot_dir = crate_path.join("test_snapshots");
        if snapshot_dir.is_dir() {
            collect_files_recursively(workspace_root, &snapshot_dir, &mut hashes)?;
        }
    }

    Ok(hashes)
}

fn collect_files_recursively(
    workspace_root: &Path,
    dir: &Path,
    hashes: &mut BTreeMap<PathBuf, String>,
) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read snapshot directory {}", dir.display()))?
    {
        let entry = entry.context("failed to read snapshot directory entry")?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursively(workspace_root, &path, hashes)?;
        } else if path.is_file() {
            let bytes = fs::read(&path)
                .with_context(|| format!("failed to read snapshot file {}", path.display()))?;
            let digest = Sha256::digest(&bytes);
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            let rel = path
                .strip_prefix(workspace_root)
                .unwrap_or(&path)
                .to_path_buf();
            hashes.insert(rel, hex);
        }
    }
    Ok(())
}

pub fn evaluate_snapshot_run(
    mode: SnapshotMode,
    before: &BTreeMap<PathBuf, String>,
    after: &BTreeMap<PathBuf, String>,
    output: &Output,
) -> SnapshotSummary {
    let mut unchanged_files = 0usize;
    let mut changed_files = 0usize;

    for (path, before_hash) in before {
        match after.get(path) {
            Some(after_hash) if after_hash == before_hash => unchanged_files += 1,
            _ => changed_files += 1,
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            changed_files += 1;
        }
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let (cargo_passed, cargo_failed) = parse_cargo_test_counts(&combined);

    let (passed, modified, failed) = match mode {
        SnapshotMode::Check => {
            let failed_count = cargo_failed
                + changed_files
                + usize::from(!output.status.success() && cargo_failed == 0 && changed_files == 0);
            let passed_count = if unchanged_files > 0 {
                unchanged_files.saturating_sub(cargo_failed)
            } else {
                cargo_passed
            };
            (passed_count, 0, failed_count)
        }
        SnapshotMode::Update => {
            let failed_count =
                cargo_failed + usize::from(!output.status.success() && cargo_failed == 0);
            let passed_count = if unchanged_files > 0 || changed_files > 0 {
                unchanged_files
            } else {
                cargo_passed
            };
            (passed_count, changed_files, failed_count)
        }
    };

    SnapshotSummary {
        mode,
        passed,
        modified,
        failed,
    }
}

pub fn parse_cargo_test_counts(output: &str) -> (usize, usize) {
    let mut passed = 0usize;
    let mut failed = 0usize;

    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("test result:") {
            continue;
        }
        for segment in trimmed.split(';') {
            for token_pair in segment.split(',') {
                let parts: Vec<&str> = token_pair.split_whitespace().collect();
                for window in parts.windows(2) {
                    if let Ok(count) = window[0].parse::<usize>() {
                        match window[1] {
                            "passed" => passed += count,
                            "failed" => failed += count,
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    (passed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn snapshot_mode_defaults_to_check_and_supports_update() {
        let default_args = SnapshotArgs {
            package: None,
            check: false,
            update: false,
        };
        assert_eq!(
            SnapshotMode::from_args(&default_args).unwrap(),
            SnapshotMode::Check
        );

        let check_args = SnapshotArgs {
            package: Some("soroban-forge-escrow".to_owned()),
            check: true,
            update: false,
        };
        assert_eq!(
            SnapshotMode::from_args(&check_args).unwrap(),
            SnapshotMode::Check
        );

        let update_args = SnapshotArgs {
            package: None,
            check: false,
            update: true,
        };
        assert_eq!(
            SnapshotMode::from_args(&update_args).unwrap(),
            SnapshotMode::Update
        );
    }

    #[test]
    fn snapshot_mode_rejects_conflicting_check_and_update_flags() {
        let conflicting = SnapshotArgs {
            package: None,
            check: true,
            update: true,
        };
        let err = SnapshotMode::from_args(&conflicting).unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot pass both --check and --update"));
    }

    #[test]
    fn snapshot_cargo_args_supports_workspace_and_package_scopes() {
        assert_eq!(snapshot_cargo_args(None), vec!["test", "--workspace"]);
        assert_eq!(
            snapshot_cargo_args(Some("soroban-forge-escrow")),
            vec!["test", "--package", "soroban-forge-escrow"]
        );
    }

    #[test]
    fn snapshot_env_vars_set_insta_and_mode_flags() {
        let check_env = snapshot_env_vars(SnapshotMode::Check);
        assert!(check_env.contains(&("INSTA_UPDATE", "no")));
        assert!(check_env.contains(&("SOROBAN_SNAPSHOT_MODE", "check")));

        let update_env = snapshot_env_vars(SnapshotMode::Update);
        assert!(update_env.contains(&("INSTA_UPDATE", "always")));
        assert!(update_env.contains(&("SOROBAN_SNAPSHOT_MODE", "update")));
    }

    #[test]
    fn collect_snapshot_hashes_reads_and_filters_test_snapshots() {
        let temp = TempDir::new().unwrap();
        let escrow_snap = temp.path().join("crates/escrow/test_snapshots/init.json");
        let vesting_snap = temp
            .path()
            .join("crates/vesting/test_snapshots/schedule.json");
        fs::create_dir_all(escrow_snap.parent().unwrap()).unwrap();
        fs::create_dir_all(vesting_snap.parent().unwrap()).unwrap();
        fs::write(&escrow_snap, br#"{"trace":"escrow"}"#).unwrap();
        fs::write(&vesting_snap, br#"{"trace":"vesting"}"#).unwrap();

        let all_hashes = collect_snapshot_hashes(temp.path(), None).unwrap();
        assert_eq!(all_hashes.len(), 2);

        let escrow_only =
            collect_snapshot_hashes(temp.path(), Some("soroban-forge-escrow")).unwrap();
        assert_eq!(escrow_only.len(), 1);
        assert!(escrow_only
            .keys()
            .next()
            .unwrap()
            .to_string_lossy()
            .contains("escrow"));
    }

    #[test]
    fn evaluate_snapshot_run_tracks_passing_modified_and_failed_snapshots() {
        let mut before = BTreeMap::new();
        before.insert(
            PathBuf::from("crates/escrow/test_snapshots/a.json"),
            "hash1".to_owned(),
        );
        before.insert(
            PathBuf::from("crates/escrow/test_snapshots/b.json"),
            "hash2".to_owned(),
        );

        let mut after = before.clone();
        after.insert(
            PathBuf::from("crates/escrow/test_snapshots/b.json"),
            "hash2-updated".to_owned(),
        );

        let ok_output = Command::new("rustc")
            .arg("--version")
            .output()
            .expect("rustc must exist");

        let update_summary =
            evaluate_snapshot_run(SnapshotMode::Update, &before, &after, &ok_output);
        assert_eq!(update_summary.passed, 1);
        assert_eq!(update_summary.modified, 1);
        assert_eq!(update_summary.failed, 0);
        assert_eq!(
            update_summary.render_terminal_summary(),
            "Snapshot summary (update): 1 passing, 1 modified, 0 failed"
        );

        let check_summary = evaluate_snapshot_run(SnapshotMode::Check, &before, &after, &ok_output);
        assert_eq!(check_summary.passed, 1);
        assert_eq!(check_summary.modified, 0);
        assert_eq!(check_summary.failed, 1);
        assert_eq!(
            check_summary.render_terminal_summary(),
            "Snapshot summary (check): 1 passing, 0 modified, 1 failed"
        );
    }

    #[test]
    fn run_with_command_propagates_missing_binary_and_nonzero_exit() {
        let temp = TempDir::new().unwrap();
        let args = SnapshotArgs {
            package: None,
            check: true,
            update: false,
        };

        let missing = run_with_command("non-existent-soroban-snapshot-bin", &args, temp.path());
        assert!(missing.is_err());

        let failing_code = run_with_command("rustc", &args, temp.path()).unwrap();
        assert_eq!(failing_code, 1);
    }
}
