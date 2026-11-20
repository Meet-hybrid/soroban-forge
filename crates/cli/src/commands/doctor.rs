use crate::cli::DoctorArgs;
use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Output};

const WASM_TARGET: &str = "wasm32v1-none";

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct CheckResult {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix_hint: Option<&'static str>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput>;
}

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(command_output(output))
    }
}

fn command_output(output: Output) -> CommandOutput {
    CommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn run(args: DoctorArgs) -> Result<()> {
    run_with_runner(&SystemCommandRunner, args, io::stdout())
}

fn run_with_runner<W: Write>(
    runner: &impl CommandRunner,
    args: DoctorArgs,
    mut output: W,
) -> Result<()> {
    let results = collect_checks(runner, args.wasm.as_deref());
    if args.json {
        print_json_report(&mut output, &results)?;
    } else {
        print_report(&mut output, &results)?;
    }
    if results.iter().all(|result| result.ok) {
        Ok(())
    } else {
        anyhow::bail!("environment preflight failed")
    }
}

pub fn collect_checks(runner: &impl CommandRunner, wasm: Option<&Path>) -> Vec<CheckResult> {
    let mut results = vec![check_expected_base_branch()];
    results.push(check_versioned_tool(runner, "stellar", &["--version"]));
    results.push(check_versioned_tool(runner, "cargo", &["--version"]));
    results.push(check_wasm_target(runner));
    if let Some(path) = wasm {
        results.push(check_wasm_artifact(path));
    }
    results
}

fn check_expected_base_branch() -> CheckResult {
    CheckResult {
        name: "PR base",
        ok: true,
        detail: "develop".to_owned(),
        fix_hint: None,
    }
}

fn check_versioned_tool(
    runner: &impl CommandRunner,
    name: &'static str,
    args: &[&str],
) -> CheckResult {
    match runner.output(name, args) {
        Ok(output) => {
            let version = output
                .stdout
                .lines()
                .chain(output.stderr.lines())
                .map(str::trim)
                .find(|line| !line.is_empty());
            CheckResult {
                name,
                ok: true,
                detail: version
                    .map(str::to_owned)
                    .unwrap_or_else(|| "version unknown".to_owned()),
                fix_hint: None,
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => CheckResult {
            name,
            ok: false,
            detail: "not found on PATH".to_owned(),
            fix_hint: Some(if name == "stellar" {
                "cargo install --locked stellar-cli"
            } else {
                "install Rust and Cargo"
            }),
        },
        Err(_) => CheckResult {
            name,
            ok: true,
            detail: "version unknown".to_owned(),
            fix_hint: None,
        },
    }
}

fn check_wasm_target(runner: &impl CommandRunner) -> CheckResult {
    match runner.output("rustup", &["target", "list", "--installed"]) {
        Ok(output) if output.success => {
            let installed = output.stdout.lines().any(|line| line.trim() == WASM_TARGET);
            CheckResult {
                name: WASM_TARGET,
                ok: installed,
                detail: if installed {
                    "installed".to_owned()
                } else {
                    "not installed".to_owned()
                },
                fix_hint: (!installed).then_some("rustup target add wasm32v1-none"),
            }
        }
        Ok(_) | Err(_) => CheckResult {
            name: WASM_TARGET,
            ok: false,
            detail: "could not verify rustup target list".to_owned(),
            fix_hint: Some("rustup target add wasm32v1-none"),
        },
    }
}

fn check_wasm_artifact(path: &Path) -> CheckResult {
    let name = "WASM artifact";
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => CheckResult {
            name,
            ok: false,
            detail: format!("{} is not a regular file", path.display()),
            fix_hint: Some("stellar contract build --locked"),
        },
        Ok(metadata) if metadata.len() == 0 => CheckResult {
            name,
            ok: false,
            detail: format!("{} is empty", path.display()),
            fix_hint: Some("stellar contract build --locked"),
        },
        Ok(_) => CheckResult {
            name,
            ok: true,
            detail: format!("{} is a non-empty file", path.display()),
            fix_hint: None,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => CheckResult {
            name,
            ok: false,
            detail: format!("{} does not exist", path.display()),
            fix_hint: Some("stellar contract build --locked"),
        },
        Err(error) => CheckResult {
            name,
            ok: false,
            detail: format!("could not inspect {}: {error}", path.display()),
            fix_hint: Some("stellar contract build --locked"),
        },
    }
}

fn print_report<W: Write>(mut output: W, results: &[CheckResult]) -> io::Result<()> {
    writeln!(output, "Check                    Status  Details")?;
    writeln!(
        output,
        "-----------------------  ------  ------------------------------"
    )?;
    for result in results {
        writeln!(
            output,
            "{:<23}  {:<6}  {}",
            result.name,
            if result.ok { "PASS" } else { "FAIL" },
            result.detail
        )?;
        if let Some(fix) = result.fix_hint {
            writeln!(output, "  fix: {fix}")?;
        }
    }
    let failed: Vec<_> = results
        .iter()
        .filter(|result| !result.ok)
        .map(|result| result.name)
        .collect();
    if failed.is_empty() {
        writeln!(output, "Summary: all checks passed")?;
    } else {
        writeln!(output, "Summary: failed checks: {}", failed.join(", "))?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct JsonDoctorReport<'a> {
    checks: &'a [CheckResult],
}

fn print_json_report<W: Write>(mut output: W, results: &[CheckResult]) -> Result<()> {
    serde_json::to_writer(&mut output, &JsonDoctorReport { checks: results })?;
    writeln!(output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;

    struct SequenceRunner {
        responses: RefCell<VecDeque<io::Result<CommandOutput>>>,
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl SequenceRunner {
        fn new(responses: Vec<io::Result<CommandOutput>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl CommandRunner for SequenceRunner {
        fn output(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            self.responses
                .borrow_mut()
                .pop_front()
                .expect("response available")
        }
    }

    fn output(stdout: &str, success: bool) -> io::Result<CommandOutput> {
        Ok(CommandOutput {
            success,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        })
    }

    #[test]
    fn checks_run_in_required_order_and_all_pass() {
        let runner = SequenceRunner::new(vec![
            output("stellar 1.0", false),
            output("cargo 1.0", true),
            output("x86_64-unknown-linux-gnu\nwasm32v1-none\n", true),
        ]);
        let results = collect_checks(&runner, None);
        assert_eq!(
            results.iter().map(|result| result.name).collect::<Vec<_>>(),
            vec!["PR base", "stellar", "cargo", "wasm32v1-none"]
        );
        assert!(results.iter().all(|result| result.ok));
        assert_eq!(results[0].detail, "develop");
        assert_eq!(results[1].detail, "stellar 1.0");
        assert_eq!(results[2].detail, "cargo 1.0");
        assert_eq!(results[3].detail, "installed");
        assert_eq!(
            runner.calls(),
            vec![
                ("stellar".to_owned(), vec!["--version".to_owned()]),
                ("cargo".to_owned(), vec!["--version".to_owned()]),
                (
                    "rustup".to_owned(),
                    vec![
                        "target".to_owned(),
                        "list".to_owned(),
                        "--installed".to_owned(),
                    ]
                ),
            ]
        );
    }

    #[test]
    fn missing_tool_and_target_are_reported_without_short_circuiting() {
        let runner = SequenceRunner::new(vec![
            Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
            output("cargo 1.0", true),
            output("x86_64-unknown-linux-gnu\n", true),
        ]);
        let results = collect_checks(&runner, None);
        assert_eq!(results.len(), 4);
        assert!(results[0].ok);
        assert!(!results[1].ok);
        assert_eq!(results[1].detail, "not found on PATH");
        assert_eq!(
            results[1].fix_hint,
            Some("cargo install --locked stellar-cli")
        );
        assert!(results[2].ok);
        assert!(!results[3].ok);
        assert_eq!(results[3].detail, "not installed");
        assert_eq!(results[3].fix_hint, Some("rustup target add wasm32v1-none"));
    }

    #[test]
    fn rustup_failure_is_reported_with_fix_hint() {
        let runner = SequenceRunner::new(vec![output("", false)]);
        let result = check_wasm_target(&runner);
        assert!(!result.ok);
        assert_eq!(result.detail, "could not verify rustup target list");
        assert_eq!(result.fix_hint, Some("rustup target add wasm32v1-none"));
    }

    #[test]
    fn run_returns_ok_when_all_required_checks_pass() {
        let runner = SequenceRunner::new(vec![
            output("stellar 1.0", false),
            output("cargo 1.0", true),
            output("wasm32v1-none\n", true),
        ]);
        let mut report = Vec::new();
        run_with_runner(
            &runner,
            DoctorArgs {
                wasm: None,
                json: false,
            },
            &mut report,
        )
        .unwrap();
        let report = String::from_utf8(report).unwrap();
        assert!(report.contains("Summary: all checks passed"));
        assert!(!report.contains("FAIL"));
    }

    #[test]
    fn run_fails_when_a_required_check_fails() {
        let runner = SequenceRunner::new(vec![
            Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
            output("cargo 1.0", true),
            output("x86_64-unknown-linux-gnu\n", true),
        ]);
        let mut report = Vec::new();
        let error = run_with_runner(
            &runner,
            DoctorArgs {
                wasm: None,
                json: false,
            },
            &mut report,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "environment preflight failed");
        let report = String::from_utf8(report).unwrap();
        assert!(report.contains("Summary: failed checks: stellar, wasm32v1-none"));
        assert!(report.contains("fix: cargo install --locked stellar-cli"));
        assert!(report.contains("fix: rustup target add wasm32v1-none"));
    }

    #[test]
    fn optional_artifact_distinguishes_missing_empty_directory_and_valid_file() {
        let root = std::env::temp_dir().join(format!("soroban-doctor-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let missing = check_wasm_artifact(&root.join("missing.wasm"));
        assert!(missing.detail.contains("does not exist"));
        let empty = root.join("empty.wasm");
        fs::write(&empty, []).unwrap();
        assert!(check_wasm_artifact(&empty).detail.contains("empty"));
        let directory = root.join("directory.wasm");
        fs::create_dir(&directory).unwrap();
        assert!(check_wasm_artifact(&directory)
            .detail
            .contains("regular file"));
        let valid = root.join("valid.wasm");
        fs::write(&valid, [1_u8]).unwrap();
        assert!(check_wasm_artifact(&valid).ok);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn json_report_has_stable_fields_for_missing_tool_target_and_artifact() {
        let root = std::env::temp_dir().join(format!("soroban-doctor-json-{}", std::process::id()));
        let missing_target = root.join("missing.wasm");
        let runner = SequenceRunner::new(vec![
            Err(io::Error::new(io::ErrorKind::NotFound, "missing stellar")),
            output("cargo 1.0", true),
            output("x86_64-unknown-linux-gnu\n", true),
        ]);
        let mut report = Vec::new();
        let error = run_with_runner(
            &runner,
            DoctorArgs {
                wasm: Some(missing_target.clone()),
                json: true,
            },
            &mut report,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "environment preflight failed");
        let json = String::from_utf8(report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let checks = value["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 5);
        assert_eq!(checks[0]["name"], "PR base");
        assert_eq!(checks[0]["detail"], "develop");
        assert_eq!(checks[1]["name"], "stellar");
        assert_eq!(checks[1]["ok"], false);
        assert_eq!(checks[1]["fix_hint"], "cargo install --locked stellar-cli");
        assert_eq!(checks[3]["name"], "wasm32v1-none");
        assert_eq!(checks[3]["ok"], false);
        assert_eq!(checks[3]["fix_hint"], "rustup target add wasm32v1-none");
        assert_eq!(checks[4]["name"], "WASM artifact");
        assert_eq!(checks[4]["ok"], false);
        assert_eq!(
            checks[4]["detail"],
            format!("{} does not exist", missing_target.display())
        );
    }
}
