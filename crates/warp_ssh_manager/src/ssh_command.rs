//! Assembles `SshServerInfo` into an `ssh ...` command and spawns child processes for connection testing.
//!
//! When writing to a PTY, call `build_ssh_command_line`, which shell-escapes each arg to prevent
//! spaces or single quotes in usernames, hosts, or key paths from breaking the command line.
//!
//! ## Password Authentication Security & Cross-Platform Compatibility
//!
//! **Non-Windows**: `ssh` in pipe-stdin mode can read the password from stdin normally, so a
//! one-shot stdin injection is used (`build_password_auth_stdin`). The password is held in memory
//! only as a `Zeroizing<String>` — it never appears in argv, `/proc/<pid>/cmdline`, `ps`, or
//! other on-machine process information readable by peers (fixing the sshpass `-p` mode issue).
//!
//! **Windows**: Win32-OpenSSH rejects reading the password from stdin even when stdin is a pipe,
//! because `CREATE_NO_WINDOW` (no console) causes it to print
//! `GetConsoleMode on STD_INPUT_HANDLE failed` and hang — see
//! PowerShell/Win32-OpenSSH issue #1470. The workaround is `SSH_ASKPASS`:
//! write a temporary .cmd script; ssh spawns it and reads its stdout as the password, bypassing
//! stdin and the console entirely. `SSH_ASKPASS_REQUIRE=force` forces the askpass path even when
//! ssh detects a TTY. The password is passed to the askpass script via a temporary file (not an
//! env var, to reduce the exposure surface); the entire lifecycle is managed by the `AskpassSession`
//! RAII guard, which cleans up immediately after ssh exits.

use crate::types::{AuthType, ConnectionStatus, SshServerInfo};
#[cfg(not(windows))]
use futures_lite::io::AsyncWriteExt as _;
use std::borrow::Cow;
use std::process::Stdio;
use std::time::Duration;
use zeroize::Zeroizing;

pub fn build_ssh_args(server: &SshServerInfo) -> Vec<String> {
    let mut args: Vec<String> = vec!["ssh".into()];
    if server.port != 22 {
        args.push("-p".into());
        args.push(server.port.to_string());
    }
    if matches!(server.auth_type, AuthType::Key | AuthType::OneKey) {
        if let Some(path) = server.key_path.as_deref() {
            if !path.is_empty() {
                args.push("-i".into());
                args.push(path.to_string());
            }
        }
    }
    let target = if server.username.is_empty() {
        server.host.clone()
    } else {
        format!("{}@{}", server.username, server.host)
    };
    args.push(target);
    args
}

/// Append the `-L/-R/-D <spec>` options for the server's configured port forwards.
///
/// Forwards whose spec cannot be rendered (e.g. a Local/Remote forward missing its
/// target) are skipped with a warning rather than aborting the whole connection.
/// Callers must invoke this while `args` still ends with options (i.e. before the
/// `--`/destination separator) so ssh parses the flags as options, not as the remote
/// command.
fn push_port_forward_args(args: &mut Vec<String>, server: &SshServerInfo) {
    for forward in &server.advanced.port_forwards {
        match forward.to_ssh_spec() {
            Some(spec) => {
                args.push(forward.ssh_flag().to_string());
                args.push(spec);
            }
            None => {
                log::warn!(
                    "skipping invalid port forward (missing target): kind={:?} bind={}:{}",
                    forward.kind,
                    forward.bind_host,
                    forward.bind_port
                );
            }
        }
    }
}

pub fn build_ssh_command_line(server: &SshServerInfo) -> String {
    let mut args = build_ssh_args(server);
    // Insert `--` right before the destination so a host/username starting with
    // `-` (e.g. `-oProxyCommand=...`) can never be parsed by ssh as an option.
    let target = args
        .pop()
        .expect("build_ssh_args always ends with the SSH destination");
    // Port forwards are options and must precede `--`/destination. They apply only to
    // the real connection — the connection-test paths build a bare target without them.
    push_port_forward_args(&mut args, server);
    args.push("--".into());
    args.push(target);
    args.iter()
        .map(|a| shell_escape::unix::escape(Cow::Borrowed(a.as_str())).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build a `ssh <alias>` command line for connecting to a `~/.ssh/config` host
/// without importing it: OpenSSH resolves HostName/User/Port/IdentityFile/
/// ProxyJump/etc. from the file itself, so we pass only the alias. `--` guards
/// against an alias that begins with `-` being parsed as an option, and the
/// alias is shell-escaped before reaching the terminal.
pub fn build_ssh_alias_command_line(alias: &str) -> String {
    let escaped = shell_escape::unix::escape(Cow::Borrowed(alias));
    format!("ssh -- {escaped}")
}

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectionTestResult {
    pub status: ConnectionStatus,
    pub latency_ms: Option<u64>,
    pub error_message: Option<String>,
}

pub async fn test_connection(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
) -> ConnectionTestResult {
    let start = instant::Instant::now();

    let result = match server.auth_type {
        AuthType::Key => test_key_auth(server).await,
        AuthType::Password | AuthType::OneKey => test_password_auth(server, password).await,
    };

    let latency = start.elapsed().as_millis() as u64;

    match result {
        Ok(()) => ConnectionTestResult {
            status: ConnectionStatus::Online,
            latency_ms: Some(latency),
            error_message: None,
        },
        Err(e) => ConnectionTestResult {
            status: ConnectionStatus::Offline,
            latency_ms: Some(latency),
            error_message: Some(e),
        },
    }
}

async fn test_key_auth(server: &SshServerInfo) -> Result<(), String> {
    let mut args = build_ssh_args(server);
    // build_ssh_args ends with the destination (user@host). The `-o` options must
    // be inserted before the destination, otherwise ssh treats `-o` as part of the
    // remote command rather than as its own option.
    let target = args
        .pop()
        .expect("build_ssh_args always ends with the SSH destination");
    args.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    // `--` guards against option injection via a `-`-leading host/username.
    args.push("--".into());
    args.push(target);
    args.push("echo ok".into());
    let cmd_args = args;

    match tokio::time::timeout(TEST_TIMEOUT, run_ssh_test(&cmd_args)).await {
        Ok(Ok(output)) => {
            // Strict match against `echo ok` output — avoids false positives where a
            // banner/motd happens to end with "ok".
            if output.trim() == "ok" {
                Ok(())
            } else {
                Err(format!("Unexpected output: {}", output.trim()))
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Connection timeout".into()),
    }
}

async fn test_password_auth(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
) -> Result<(), String> {
    let password = password.ok_or("Password not provided")?;

    // Build ssh command arguments (note: -o options must be inserted before the destination — see function comment)
    let cmd_args = build_password_auth_cmd_args(server);

    // Platform branch: Windows uses SSH_ASKPASS; other platforms use stdin injection
    #[cfg(windows)]
    return test_password_auth_windows(cmd_args, &password).await;
    #[cfg(not(windows))]
    test_password_auth_unix(cmd_args, &password).await
}

/// Non-Windows: `ssh` can read the password normally from a pipe stdin.
#[cfg(not(windows))]
async fn test_password_auth_unix(
    cmd_args: Vec<String>,
    password: &Zeroizing<String>,
) -> Result<(), String> {
    let stdin_bytes = build_password_auth_stdin(password);

    let mut child = command::r#async::Command::new("ssh")
        .args(&cmd_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start ssh: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&stdin_bytes)
            .await
            .map_err(|e| format!("Failed to write password: {e}"))?;
    }

    let output = match tokio::time::timeout(TEST_TIMEOUT, child.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Failed to read ssh output: {e}")),
        Err(_) => return Err("Connection timeout".into()),
    };

    finalize_password_test_result(&output)
}

/// Windows: use the SSH_ASKPASS mechanism to pass the password to ssh, bypassing stdin/console entirely.
#[cfg(windows)]
async fn test_password_auth_windows(
    cmd_args: Vec<String>,
    password: &Zeroizing<String>,
) -> Result<(), String> {
    let askpass =
        AskpassSession::new(password).map_err(|e| format!("Failed to prepare askpass: {e}"))?;

    let mut cmd = command::r#async::Command::new("ssh");
    cmd.args(&cmd_args)
        // ssh no longer needs to read the password from stdin; set to null so ssh
        // does not mistakenly assume a tty is present
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    askpass.apply_env(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start ssh: {e}"))?;

    // When the timeout fires, child is dropped → kill_on_drop automatically kills ssh.
    // The askpass guard is dropped at the end of the function, cleaning up temp files.
    let output = match tokio::time::timeout(TEST_TIMEOUT, child.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Failed to read ssh output: {e}")),
        Err(_) => return Err("Connection timeout".into()),
    };
    drop(askpass);

    finalize_password_test_result(&output)
}

/// Parse the ssh child process output and apply unified success/failure logic (shared by both platforms).
fn finalize_password_test_result(output: &std::process::Output) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr_trimmed = String::from_utf8_lossy(&output.stderr).trim().to_string();

    // Always log the real ssh stderr, even on success, to aid post-hoc diagnosis of
    // "why did the server accept the password but the UI reported success" discrepancies.
    if !stderr_trimmed.is_empty() {
        log::warn!("ssh test stderr: {stderr_trimmed}");
    }

    // Success check: strict match against `echo ok` output. The previous `ends_with("ok")`
    // fallback could produce false positives when a banner/motd happened to end with "ok" — removed here.
    if output.status.success() && stdout.trim() == "ok" {
        Ok(())
    } else if stderr_trimmed.contains("Permission denied")
        || stderr_trimmed.contains("Authentication failed")
    {
        // Include a trimmed stderr snippet (<= 200 chars) in the error so the user can
        // tell whether the server has password auth disabled, kbd-only AuthenticationMethods set, etc.
        let detail = if stderr_trimmed.is_empty() {
            String::new()
        } else {
            let snippet: String = stderr_trimmed.chars().take(200).collect();
            if stderr_trimmed.chars().count() > 200 {
                format!(" ({snippet}...)")
            } else {
                format!(" ({snippet})")
            }
        };
        Err(format!("Authentication failed: wrong password{detail}"))
    } else {
        Err(format!(
            "Unexpected output: stdout={} stderr={}",
            stdout.trim(),
            stderr_trimmed
        ))
    }
}

/// Encode the password as the byte stream to write to ssh's stdin: password UTF-8 bytes + newline.
/// Extracted as a pure function so unit tests can assert "stdin contains the literal password + newline".
/// Only the unix branch calls this in production (Windows uses SSH_ASKPASS), but the function
/// compiles cross-platform so `build_password_auth_stdin_*` unit tests can run on Windows CI too.
// On Windows only tests call this function; the production path uses SSH_ASKPASS — suppress dead_code
#[cfg_attr(windows, allow(dead_code))]
fn build_password_auth_stdin(password: &Zeroizing<String>) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(Vec::with_capacity(password.len() + 1));
    v.extend_from_slice(password.as_bytes());
    v.push(b'\n');
    v
}

/// Build the complete argv for the ssh child process used in password-auth connection tests.
///
/// Unlike `build_ssh_args`, this function skips the first entry `"ssh"` (we specify it
/// explicitly via `Command::new("ssh")`), and appends test `-o` options plus the `echo ok`
/// remote command.
///
/// Key option rationale:
/// - `BatchMode=no`: allows ssh to read the password from stdin / askpass (required when not using askpass)
/// - `PreferredAuthentications=password`: declares **only** password auth, excluding
///   `keyboard-interactive`. Without this, the server-side PAM would trigger a kbd-interactive
///   fallback after the password attempt; each kbd-int sub-prompt that gets no response causes
///   a `pam_faildelay` (~2s/attempt), which can exhaust the full `TEST_TIMEOUT` of ~8-10s.
/// - `KbdInteractiveAuthentication=no`: client-side capability switch that disables the entire
///   kbd-int protocol. `PreferredAuthentications` alone is insufficient — it only constrains
///   the password sub-method prompt count while kbd-int can still proceed; both switches together
///   provide defense in depth.
/// - `NumberOfPasswordPrompts=1`: allow only one password retry under the password sub-method.
/// - `ConnectTimeout=5`: per-TCP-connection timeout.
/// - `StrictHostKeyChecking=no`: do not block on known_hosts changes (avoids false negatives in
///   test scenarios; real terminal connections take a different code path).
/// - `LogLevel=ERROR`: suppress host-key prompts, banners, and other noise.
///
/// `echo ok` is used as the remote command; strict stdout matching determines success
/// (avoids false positives from banners/motd that happen to contain "ok").
///
/// author: logic
/// date: 2026-06-01
fn build_password_auth_cmd_args(server: &SshServerInfo) -> Vec<String> {
    // skip(1) drops "ssh" itself (Command::new specifies it), leaving
    // ["-p","2222","user@host"]. The -o options must be inserted before
    // the destination, otherwise SSH treats -o as part of the remote
    // command rather than as its own option.
    let mut args: Vec<String> = build_ssh_args(server).into_iter().skip(1).collect();
    let target = args
        .pop()
        .expect("build_ssh_args always ends with the SSH destination");
    args.extend([
        "-o".into(),
        "BatchMode=no".into(),
        "-o".into(),
        "PreferredAuthentications=password".into(),
        "-o".into(),
        "KbdInteractiveAuthentication=no".into(),
        "-o".into(),
        "NumberOfPasswordPrompts=1".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    // `--` guards against option injection via a `-`-leading host/username.
    args.push("--".into());
    args.push(target);
    args.push("echo ok".into());
    args
}

async fn run_ssh_test(args: &[String]) -> Result<String, std::io::Error> {
    // Always spawn via command::r#async so that on Windows the child gets CREATE_NO_WINDOW,
    // preventing a console window from flashing (see .clippy.toml ban on tokio::process::Command).
    let output = command::r#async::Command::new(&args[0])
        .args(&args[1..])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Success check: process exit code is 0, or the remote `echo ok` output has been received
    // (some sshpass warnings cause a non-zero exit code even though stdout still contains "ok").
    if output.status.success() || stdout.contains("ok") {
        Ok(stdout)
    } else {
        Err(std::io::Error::other(stderr))
    }
}

/// Windows-only askpass session: creates a password file and an askpass helper script in the
/// temp directory, exposes them to `ssh` via the `SSH_ASKPASS` environment variable, and
/// automatically cleans up both files on drop.
///
/// `ssh.exe` on Windows rejects reading the password from stdin even when stdin is a pipe,
/// because the absence of a console causes it to print
/// `GetConsoleMode on STD_INPUT_HANDLE failed` and hang —
/// see PowerShell/Win32-OpenSSH issue #1470. The workaround is `SSH_ASKPASS`:
/// when `ssh` sees that environment variable it spawns the specified program and reads its
/// stdout as the password, bypassing stdin and the console entirely.
/// `SSH_ASKPASS_REQUIRE=force` forces ssh to take the askpass path even when a TTY is detected.
///
/// The password is passed to the askpass script via a temporary file (not an env var, to
/// reduce the exposure surface): env vars are visible to the `ssh` child process and all of
/// its descendants. The askpass process lifetime is extremely short (ssh forks, execs,
/// reads, and exits), so the on-disk window is controllable to millisecond precision.
///
/// **Security trade-off**: the two temp files do not have `FILE_ATTRIBUTE_HIDDEN` set and
/// their ACLs are not tightened. They live under Windows `%TEMP%` with its default per-user
/// isolation (`C:\Users\<user>\AppData\Local\Temp`, separate for each user). An earlier
/// version tried hidden attribute + icacls restricted to `(R)`, but `FILE_ATTRIBUTE_HIDDEN`
/// caused `posix_spawnp` to return `ERROR_ACCESS_DENIED` (error 5) at the `CreateProcessW`
/// stage, preventing askpass from starting at all and silently sending no password to the
/// server's password prompt (users saw "wrong password" even though nothing was transmitted).
/// The per-user isolation of the Windows temp dir is sufficient; simplicity and reliability
/// take priority over "defense in depth" here.
///
/// author: logic
/// date: 2026-06-01
#[cfg(windows)]
struct AskpassSession {
    password_path: std::path::PathBuf,
    script_path: std::path::PathBuf,
}

#[cfg(windows)]
impl AskpassSession {
    fn new(password: &Zeroizing<String>) -> std::io::Result<Self> {
        use std::io::Write as _;

        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let suffix = format!("{pid}-{nanos}");

        let password_path = dir.join(format!("warp-ssh-askpass-{suffix}.txt"));
        let script_path = dir.join(format!("warp-ssh-askpass-{suffix}.cmd"));

        // Write the password to a temp file (no hidden attribute, no ACL changes — see type-level security trade-off)
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&password_path)?;
            f.write_all(password.as_bytes())?;
            f.sync_all()?;
        }

        // Write the askpass helper script: reads the first line of the file pointed to by
        // %WARP_SSH_ASKPASS_FILE% and echoes it to stdout. `set /p` reads the first line
        // (stripping the newline); `echo !PW!` outputs it.
        // Uses `setlocal enabledelayedexpansion` + `!PW!` delayed expansion to prevent
        // passwords containing cmd special characters (&, |, <, >, ^) from being truncated
        // by the immediate expansion of %PW%.
        let body = "@echo off\r\nsetlocal enabledelayedexpansion\r\nset /p PW=<\"%WARP_SSH_ASKPASS_FILE%\"\r\necho !PW!\r\nendlocal\r\n";
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&script_path)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
        }

        Ok(Self {
            password_path,
            script_path,
        })
    }

    /// Attach the environment variables required by SSH_ASKPASS to the ssh child process.
    fn apply_env(&self, cmd: &mut command::r#async::Command) {
        cmd.env("SSH_ASKPASS", &self.script_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("WARP_SSH_ASKPASS_FILE", &self.password_path)
            .env_remove("DISPLAY");
    }
}

#[cfg(windows)]
impl Drop for AskpassSession {
    fn drop(&mut self) {
        // Delete both temp files immediately after ssh exits to minimize the window during
        // which the password exists on disk. Errors are swallowed: cleanup failure must not
        // affect the return value of the main flow.
        let _ = std::fs::remove_file(&self.password_path);
        let _ = std::fs::remove_file(&self.script_path);
    }
}

#[cfg(test)]
#[path = "ssh_command_tests.rs"]
mod tests;
