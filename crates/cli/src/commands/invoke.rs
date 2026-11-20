use crate::cli::InvokeArgs;
use crate::commands::process::{ensure_success, CommandOutput};
use anyhow::{Context, Result};
use std::{io, process::Command};

pub fn run(args: InvokeArgs) -> Result<()> {
    run_with(&args, |command| {
        command.output().map(CommandOutput::from_output)
    })
}

/// Invoke the Stellar CLI through an injected command runner.
///
/// Production passes [`Command::output`]; tests pass a closure that returns a
/// fabricated [`CommandOutput`], so both the success and failure paths run
/// without spawning the Stellar CLI.
fn run_with<F>(args: &InvokeArgs, execute: F) -> Result<()>
where
    F: FnOnce(&mut Command) -> io::Result<CommandOutput>,
{
    let mut command = invoke_command(args);
    let output = execute(&mut command).context("failed to run stellar contract invoke")?;
    ensure_success("contract invocation", &output)?;

    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    Ok(())
}

fn invoke_command(args: &InvokeArgs) -> Command {
    // For MVP, we'll use the stellar CLI as the transport layer
    let mut cmd = Command::new("stellar");
    cmd.arg("contract").arg("invoke");
    cmd.arg("--id").arg(&args.contract);

    if let Some(source) = &args.source {
        cmd.arg("--source-account").arg(source);
    }

    cmd.arg("--network").arg(&args.network);

    // Add the function name
    cmd.arg("--").arg(&args.function);

    // Parse and add arguments
    for arg in &args.args {
        if let Some((key, value)) = arg.split_once('=') {
            cmd.arg(format!("--{}", key)).arg(value);
        } else {
            // Positional argument
            cmd.arg(arg);
        }
    }

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke_args() -> InvokeArgs {
        InvokeArgs {
            contract: "C123".to_owned(),
            function: "get_threshold".to_owned(),
            args: vec!["owner=GABC".to_owned(), "positional".to_owned()],
            network: "testnet".to_owned(),
            source: Some("alice".to_owned()),
        }
    }

    #[test]
    fn invoke_command_preserves_arguments_and_successful_behavior() {
        let command = invoke_command(&invoke_args());
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(args[0], "contract");
        assert_eq!(args[1], "invoke");
        assert!(args.windows(2).any(|pair| pair == ["--id", "C123"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--source-account", "alice"]));
        assert!(args.windows(2).any(|pair| pair == ["--network", "testnet"]));
        assert!(args.windows(2).any(|pair| pair == ["--", "get_threshold"]));
        assert!(args.windows(2).any(|pair| pair == ["--owner", "GABC"]));
        assert!(args.iter().any(|arg| arg == "positional"));
    }

    #[test]
    fn successful_invocation_returns_ok_without_spawning_stellar() {
        run_with(&invoke_args(), |_| Ok(CommandOutput::new(true, "42\n", ""))).unwrap();
    }

    #[test]
    fn non_zero_invocation_status_returns_contextual_error() {
        let error = run_with(&invoke_args(), |_| {
            Ok(CommandOutput::new(
                false,
                "",
                "error: contract invocation reverted",
            ))
        })
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("contract invocation failed"));
        assert!(message.contains("reverted"));
    }

    #[test]
    fn invocation_spawn_failure_returns_contextual_error() {
        let error = run_with(&invoke_args(), |_| {
            Err(io::Error::new(io::ErrorKind::NotFound, "stellar not found"))
        })
        .unwrap_err();

        assert!(format!("{error:#}").contains("failed to run stellar contract invoke"));
    }
}
