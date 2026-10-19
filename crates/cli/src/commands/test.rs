use crate::cli::TestArgs;
use anyhow::{Context, Result};

pub fn run(args: TestArgs) -> Result<()> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(test_args(args.package.as_deref()));
    let status = cmd.status().context("failed to run cargo test")?;
    if !status.success() {
        std::process::exit(exit_code_for_status(&status));
    }
    Ok(())
}

pub(crate) fn exit_code_for_status(status: &std::process::ExitStatus) -> i32 {
    if status.success() {
        0
    } else {
        1
    }
}

pub(crate) fn test_args(package: Option<&str>) -> Vec<String> {
    let mut argv = vec!["test".to_owned()];
    match package {
        Some(pkg) => argv.extend(["--package".to_owned(), pkg.to_owned()]),
        None => argv.push("--workspace".to_owned()),
    }
    argv
}

pub(crate) fn format_summary_line(
    label: &str,
    passed: usize,
    modified: usize,
    failed: usize,
) -> String {
    format!("{label}: {passed} passing, {modified} modified, {failed} failed")
}

#[cfg(test)]
mod tests {
    use super::{exit_code_for_status, test_args};

    #[test]
    fn test_args_uses_workspace_mode_by_default() {
        assert_eq!(test_args(None), vec!["test", "--workspace"]);
    }

    #[test]
    fn test_args_uses_package_scoped_mode_when_provided() {
        assert_eq!(
            test_args(Some("soroban-forge-shared-utils")),
            vec!["test", "--package", "soroban-forge-shared-utils"]
        );
    }

    #[test]
    fn failed_test_status_maps_to_nonzero_exit_code() {
        let status = std::process::Command::new("false").status().unwrap();
        assert!(!status.success());
        assert_eq!(exit_code_for_status(&status), 1);
    }
}
