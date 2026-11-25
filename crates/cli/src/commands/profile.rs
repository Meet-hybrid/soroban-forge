use crate::cli::ProfileArgs;
use crate::commands::process::{ensure_success, CommandOutput};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{io, process::Command};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Metric {
    pub consumed: u64,
    pub maximum: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileReport {
    pub contract: String,
    pub function: String,
    pub instructions: Metric,
    pub memory_bytes: Metric,
    pub read_entries: Metric,
    pub write_entries: Metric,
    pub within_budget: bool,
}

pub fn run(args: ProfileArgs) -> Result<()> {
    run_with(&args, |command| {
        command.output().map(CommandOutput::from_output)
    })
}

fn run_with<F>(args: &ProfileArgs, execute: F) -> Result<()>
where
    F: FnOnce(&mut Command) -> io::Result<CommandOutput>,
{
    let mut command = profile_command(args)?;
    let output = execute(&mut command).context("failed to run stellar contract simulation")?;
    ensure_success("contract simulation", &output)?;
    let report = parse_report(args, &output)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }
    if !report.within_budget {
        bail!("simulated resources exceed one or more configured budget thresholds");
    }
    Ok(())
}

fn profile_command(args: &ProfileArgs) -> Result<Command> {
    let network = crate::network_profiles::resolve_network(
        args.network.as_deref(),
        args.network_profile.as_deref(),
        args.rpc_url.as_deref(),
        args.network_passphrase.as_deref(),
    )?;
    let mut command = Command::new("stellar");
    command
        .arg("contract")
        .arg("invoke")
        .arg("--id")
        .arg(&args.contract)
        .arg("--send")
        .arg("no")
        .arg("--cost")
        .arg("--network")
        .arg(&network.name);
    if let Some(source) = &args.source {
        command.arg("--source-account").arg(source);
    }
    if let Some(rpc_url) = network.rpc_url {
        command.arg("--rpc-url").arg(rpc_url);
    }
    if let Some(passphrase) = network.network_passphrase {
        command.arg("--network-passphrase").arg(passphrase);
    }
    command.arg("--").arg(&args.function);
    for arg in &args.args {
        if let Some((key, value)) = arg.split_once('=') {
            command.arg(format!("--{key}")).arg(value);
        } else {
            command.arg(arg);
        }
    }
    Ok(command)
}

fn parse_report(args: &ProfileArgs, output: &CommandOutput) -> Result<ProfileReport> {
    let text = format!("{}\n{}", output.stdout, output.stderr);
    let instructions = find_metric(&text, &["cpu instructions", "instructions"])?;
    let memory_bytes = find_metric(&text, &["memory bytes", "memory"])?;
    let read_entries = find_metric(&text, &["read entries", "ledger reads"])?;
    let write_entries = find_metric(&text, &["write entries", "ledger writes"])?;
    let report = ProfileReport {
        contract: args.contract.clone(),
        function: args.function.clone(),
        instructions: Metric {
            consumed: instructions,
            maximum: args.max_instructions,
        },
        memory_bytes: Metric {
            consumed: memory_bytes,
            maximum: args.max_memory_bytes,
        },
        read_entries: Metric {
            consumed: read_entries,
            maximum: args.max_read_entries,
        },
        write_entries: Metric {
            consumed: write_entries,
            maximum: args.max_write_entries,
        },
        within_budget: instructions <= args.max_instructions
            && memory_bytes <= args.max_memory_bytes
            && read_entries <= args.max_read_entries
            && write_entries <= args.max_write_entries,
    };
    Ok(report)
}

fn find_metric(text: &str, labels: &[&str]) -> Result<u64> {
    for line in text.lines() {
        let normalized = line.to_ascii_lowercase();
        if labels.iter().any(|label| normalized.contains(label)) {
            if let Some(value) = normalized
                .split(|character: char| !character.is_ascii_digit())
                .find(|part| !part.is_empty())
            {
                return value
                    .parse()
                    .context("stellar returned a non-numeric resource metric");
            }
        }
    }
    bail!(
        "stellar simulation did not report one of the required resource metrics: {}",
        labels.join(" or ")
    )
}

fn print_report(report: &ProfileReport) {
    println!("Contract profile: {}::{}", report.contract, report.function);
    println!(
        "{:<20} {:>14} {:>14} {:>10}",
        "Resource", "Consumed", "Maximum", "Status"
    );
    for (name, metric) in [
        ("CPU instructions", report.instructions),
        ("Memory bytes", report.memory_bytes),
        ("Read entries", report.read_entries),
        ("Write entries", report.write_entries),
    ] {
        let status = if metric.consumed <= metric.maximum {
            "OK"
        } else {
            "EXCEEDED"
        };
        println!(
            "{name:<20} {:>14} {:>14} {status:>10}",
            metric.consumed, metric.maximum
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> ProfileArgs {
        ProfileArgs {
            contract: "C123".to_owned(),
            function: "balance".to_owned(),
            args: vec!["token=C456".to_owned()],
            network: Some("testnet".to_owned()),
            network_profile: None,
            rpc_url: None,
            network_passphrase: None,
            source: Some("alice".to_owned()),
            json: true,
            max_instructions: 100,
            max_memory_bytes: 200,
            max_read_entries: 3,
            max_write_entries: 2,
        }
    }

    #[test]
    fn command_is_a_non_mutating_simulation() {
        let command = profile_command(&args()).unwrap();
        let argv: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(argv.windows(2).any(|pair| pair == ["--send", "no"]));
        assert!(argv.contains(&"--cost".to_owned()));
        assert!(argv
            .windows(2)
            .any(|pair| pair == ["--source-account", "alice"]));
    }

    #[test]
    fn parser_extracts_cost_metrics_and_thresholds() {
        let output = CommandOutput::new(
            true,
            "",
            "CPU instructions: 90\nMemory bytes: 150\nRead entries: 2\nWrite entries: 1\n",
        );
        let report = parse_report(&args(), &output).unwrap();
        assert!(report.within_budget);
        assert_eq!(report.instructions.consumed, 90);
    }

    #[test]
    fn parser_marks_threshold_breach() {
        let output = CommandOutput::new(
            true,
            "",
            "CPU instructions: 101\nMemory bytes: 150\nRead entries: 2\nWrite entries: 1\n",
        );
        assert!(!parse_report(&args(), &output).unwrap().within_budget);
    }

    #[test]
    fn malformed_simulation_output_is_an_error() {
        let error =
            parse_report(&args(), &CommandOutput::new(true, "", "cost unavailable")).unwrap_err();
        assert!(error.to_string().contains("did not report"));
    }
}
