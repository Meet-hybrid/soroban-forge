use anyhow::{bail, Result};

/// Captured outcome of an external CLI invocation.
///
/// Handlers keep this small, process-free value so their success and failure
/// paths can be exercised in unit tests without spawning the Stellar CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub(crate) fn from_output(output: std::process::Output) -> Self {
        Self {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(success: bool, stdout: &str, stderr: &str) -> Self {
        Self {
            success,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }
}

/// Turn a non-zero child-process status into an `Err` with useful context.
///
/// `operation` names the failed action (for example `"event retrieval"`) so
/// callers can tell which command failed without inspecting the child argv.
pub(crate) fn ensure_success(operation: &str, output: &CommandOutput) -> Result<()> {
    if output.success {
        return Ok(());
    }

    let stderr = output.stderr.trim();
    if stderr.is_empty() {
        bail!("{operation} failed: stellar exited with a non-zero status");
    }
    bail!("{operation} failed: {stderr}");
}
