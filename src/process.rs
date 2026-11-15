use anyhow::{Context, Result};
use std::{
    fmt,
    process::{ExitStatus, Stdio},
};
use tokio::{io::AsyncWriteExt, process::Command};

#[derive(Debug)]
pub struct CommandFailure {
    pub code: i32,
    program: String,
}
impl fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} exited with status {}", self.program, self.code)
    }
}
impl std::error::Error for CommandFailure {}
fn check(status: ExitStatus, program: String) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(CommandFailure {
            code: status.code().filter(|n| *n > 0).unwrap_or(1),
            program,
        }
        .into())
    }
}
// Killing a cancelled child and restoring terminal modes mirrors the local CLI lifecycle.
pub async fn run(
    command: &mut Command,
    input: Option<&[u8]>,
    capture: bool,
    inherit_stdin: bool,
) -> Result<String> {
    let program = command.as_std().get_program().to_string_lossy().to_string();
    command.kill_on_drop(true).stdin(if input.is_some() {
        Stdio::piped()
    } else if inherit_stdin {
        Stdio::inherit()
    } else {
        Stdio::null()
    });
    command.stdout(if capture {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let mut child = command
        .spawn()
        .with_context(|| format!("could not run {program}"))?;
    let stdin = child.stdin.take();
    let write = async {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            stdin.write_all(input).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let (output, ()) = tokio::try_join!(child.wait_with_output(), write)?;
    check(output.status, program)?;
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
pub async fn command(name: &str, args: &[&str]) -> Result<()> {
    run(Command::new(name).args(args), None, false, false).await?;
    Ok(())
}
pub fn find_program(name: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}
pub struct Terminal(Option<libc::termios>);
impl Terminal {
    pub fn save(enabled: bool) -> Result<Self> {
        use std::io::IsTerminal;
        let mut state = None;
        if enabled && std::io::stdin().is_terminal() {
            let mut value = std::mem::MaybeUninit::uninit();
            // SAFETY: tcgetattr writes a termios value through a valid pointer.
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, value.as_mut_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            // SAFETY: tcgetattr succeeded and initialized value.
            state = Some(unsafe { value.assume_init() });
        }
        Ok(Self(state))
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, state);
            }
        }
    }
}
