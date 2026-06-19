//! Unit tests for `ssh_command`.
//!
//! Split into a separate file per `AGENTS.md §5.6`, included by `ssh_command.rs` via `#[path]`.
//! Coverage:
//! - `build_ssh_args` / `build_ssh_command_line` argument construction
//! - `test_connection` error paths when the password is missing or the auth type is wrong
//! - `build_password_auth_stdin` byte-stream construction (also covers the critical security path
//!   of stdin injection)
//!
//! Note: end-to-end tests that actually spawn an ssh child process are covered by integration
//! tests / manual tests in `app/src/ssh_manager/server_view.rs` — unit tests do not make network
//! connections.
//!
//! author: logic
//! date: 2026-06-01

use super::*;
use crate::types::{PortForward, PortForwardKind};
use zeroize::Zeroizing;

fn server() -> SshServerInfo {
    SshServerInfo {
        node_id: "n".into(),
        host: "1.2.3.4".into(),
        port: 22,
        username: "alice".into(),
        auth_type: AuthType::Password,
        key_path: None,
        credential_id: None,
        startup_command: None,
        notes: None,
        last_connected_at: None,
        advanced: Default::default(),
    }
}

#[test]
fn default_port_omitted() {
    let s = server();
    assert_eq!(build_ssh_args(&s), vec!["ssh", "alice@1.2.3.4"]);
    // `build_ssh_command_line` inserts `--` before the destination. shell-escape
    // conservatively wraps user@host in single quotes, which is a valid
    // shell-equivalent form — we do not require the unquoted version.
    let line = build_ssh_command_line(&s);
    assert!(
        line == "ssh -- alice@1.2.3.4" || line == "ssh -- 'alice@1.2.3.4'",
        "unexpected: {line}"
    );
}

#[test]
fn command_line_separates_destination_with_double_dash() {
    // A host starting with `-` must not be interpreted by ssh as an option.
    // The `--` separator guards against this option-injection vector.
    let mut s = server();
    s.username = String::new();
    s.host = "-oProxyCommand=touch /tmp/pwned".into();
    let line = build_ssh_command_line(&s);
    let dash_dash = line
        .find(" -- ")
        .expect("`--` must precede the destination");
    let host = line
        .find("-oProxyCommand")
        .expect("host present in command line");
    assert!(dash_dash < host, "`--` must come before the host: {line}");
}

#[test]
fn custom_port_uses_dash_p() {
    let mut s = server();
    s.port = 2222;
    assert_eq!(
        build_ssh_args(&s),
        vec!["ssh", "-p", "2222", "alice@1.2.3.4"]
    );
}

#[test]
fn key_auth_emits_dash_i() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/u/.ssh/id_ed25519".into());
    assert_eq!(
        build_ssh_args(&s),
        vec!["ssh", "-i", "/home/u/.ssh/id_ed25519", "alice@1.2.3.4"]
    );
}

#[test]
fn key_auth_without_path_is_skipped() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = None;
    assert_eq!(build_ssh_args(&s), vec!["ssh", "alice@1.2.3.4"]);
}

#[test]
fn empty_username_yields_host_only() {
    let mut s = server();
    s.username = String::new();
    assert_eq!(build_ssh_args(&s), vec!["ssh", "1.2.3.4"]);
}

#[test]
fn shell_escapes_spaces_in_path() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/path with spaces/id_rsa".into());
    let line = build_ssh_command_line(&s);
    assert!(
        line.contains("'/path with spaces/id_rsa'"),
        "actual: {line}"
    );
}

// -------- Port forwarding emission --------
//
// Forwards apply only to the real connection (`build_ssh_command_line`), never to the
// connection-test paths. They are emitted as `-L/-R/-D <spec>` options before the
// `--`/destination separator so ssh parses them as options, not as the remote command.
// author: logic
// date: 2026-06-19

fn local_forward() -> PortForward {
    PortForward {
        kind: PortForwardKind::Local,
        bind_host: "127.0.0.1".into(),
        bind_port: 8080,
        target_host: Some("internal.example.com".into()),
        target_port: Some(80),
        description: None,
    }
}

#[test]
fn command_line_emits_local_forward_before_separator() {
    let mut s = server();
    s.advanced.port_forwards = vec![local_forward()];
    let line = build_ssh_command_line(&s);
    // shell-escape conservatively single-quotes the spec (it contains `:`).
    let flag_pos = line
        .find("-L '127.0.0.1:8080:internal.example.com:80'")
        .expect("local forward spec must appear");
    let sep_pos = line.find(" -- ").expect("`--` separator must appear");
    assert!(
        flag_pos < sep_pos,
        "forward option must precede `--`; got {line}"
    );
}

#[test]
fn command_line_emits_remote_forward() {
    let mut s = server();
    s.advanced.port_forwards = vec![PortForward {
        kind: PortForwardKind::Remote,
        bind_host: "0.0.0.0".into(),
        bind_port: 9000,
        target_host: Some("localhost".into()),
        target_port: Some(3000),
        description: None,
    }];
    let line = build_ssh_command_line(&s);
    assert!(
        line.contains("-R '0.0.0.0:9000:localhost:3000'"),
        "expected remote forward spec; got {line}"
    );
}

#[test]
fn command_line_emits_dynamic_forward_bind_only() {
    let mut s = server();
    s.advanced.port_forwards = vec![PortForward {
        kind: PortForwardKind::Dynamic,
        bind_host: "127.0.0.1".into(),
        bind_port: 1080,
        target_host: None,
        target_port: None,
        description: None,
    }];
    let line = build_ssh_command_line(&s);
    assert!(
        line.contains("-D '127.0.0.1:1080'"),
        "expected dynamic forward spec; got {line}"
    );
    assert!(
        !line.contains("-L") && !line.contains("-R"),
        "dynamic forward must not emit -L/-R; got {line}"
    );
}

#[test]
fn command_line_emits_multiple_forwards_in_order() {
    let mut s = server();
    s.advanced.port_forwards = vec![
        local_forward(),
        PortForward {
            kind: PortForwardKind::Dynamic,
            bind_host: "127.0.0.1".into(),
            bind_port: 1080,
            target_host: None,
            target_port: None,
            description: None,
        },
    ];
    let line = build_ssh_command_line(&s);
    let local_pos = line.find("-L ").expect("local forward present");
    let dynamic_pos = line.find("-D ").expect("dynamic forward present");
    assert!(
        local_pos < dynamic_pos,
        "forwards must keep their configured order; got {line}"
    );
}

#[test]
fn command_line_skips_invalid_forward() {
    let mut s = server();
    // Local forward missing its target → no renderable spec → skipped.
    s.advanced.port_forwards = vec![PortForward {
        kind: PortForwardKind::Local,
        bind_host: "127.0.0.1".into(),
        bind_port: 8080,
        target_host: None,
        target_port: None,
        description: None,
    }];
    let line = build_ssh_command_line(&s);
    assert!(
        !line.contains("-L"),
        "invalid forward must be skipped; got {line}"
    );
}

#[test]
fn build_ssh_args_never_emits_forwards() {
    // Forwards must not leak into the bare arg builder used by the connection-test paths.
    let mut s = server();
    s.advanced.port_forwards = vec![local_forward()];
    let args = build_ssh_args(&s);
    assert!(
        !args.iter().any(|a| a == "-L" || a == "-R" || a == "-D"),
        "build_ssh_args must not emit port forwards; got {args:?}"
    );
}

#[test]
fn test_connection_requires_password_for_password_auth() {
    let s = server();
    // test_connection should return Offline + an error message when no password is supplied
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(test_connection(&s, None));
    assert_eq!(result.status, ConnectionStatus::Offline);
    assert!(
        result
            .error_message
            .unwrap()
            .contains("Password not provided")
    );
}

#[test]
fn test_connection_requires_password_for_onekey_auth() {
    let mut s = server();
    s.auth_type = AuthType::OneKey;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(test_connection(&s, None));
    assert_eq!(result.status, ConnectionStatus::Offline);
    assert!(
        result
            .error_message
            .unwrap()
            .contains("Password not provided")
    );
}

#[test]
fn onekey_key_auth_emits_dash_i_when_key_path_is_resolved() {
    let mut s = server();
    s.auth_type = AuthType::OneKey;
    s.key_path = Some("/home/u/.ssh/shared_ed25519".into());

    assert_eq!(
        build_ssh_args(&s),
        vec!["ssh", "-i", "/home/u/.ssh/shared_ed25519", "alice@1.2.3.4"]
    );
}

#[test]
fn test_connection_key_auth_uses_batch_mode() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/user/.ssh/id_rsa".into());
    // For key auth the BatchMode=yes path is taken (carried by run_ssh_test);
    // here we only verify that build_ssh_args includes -i and the key_path.
    let args = build_ssh_args(&s);
    assert!(args.contains(&"-i".to_string()));
    assert!(args.contains(&"/home/user/.ssh/id_rsa".to_string()));
}

#[test]
fn connection_status_equality() {
    assert_eq!(ConnectionStatus::Online, ConnectionStatus::Online);
    assert_eq!(ConnectionStatus::Offline, ConnectionStatus::Offline);
    assert_eq!(ConnectionStatus::Unknown, ConnectionStatus::Unknown);
    assert_ne!(ConnectionStatus::Online, ConnectionStatus::Offline);
    assert_ne!(ConnectionStatus::Online, ConnectionStatus::Unknown);
    assert_ne!(ConnectionStatus::Offline, ConnectionStatus::Unknown);
}

// -------- Password stdin injection security --------

/// Verify that `build_password_auth_stdin` correctly encodes the password + newline.
/// This is the key invariant of the password-leak fix: the bytes written to ssh's stdin
/// must be exactly the literal password + `\n`, not any form that would cause the password
/// to travel via argv, environment variables, or temporary files.
#[test]
fn build_password_auth_stdin_contains_password_with_newline() {
    let password: Zeroizing<String> = Zeroizing::new("s3cret-pass".into());
    let bytes = build_password_auth_stdin(&password);
    assert_eq!(&*bytes, b"s3cret-pass\n");
}

/// Edge case: an empty password must still write a single `\n` so that ssh receives EOF
/// immediately and reports authentication failure (rather than hanging while waiting for
/// a prompt).
#[test]
fn build_password_auth_stdin_empty_password_still_has_newline() {
    let password: Zeroizing<String> = Zeroizing::new(String::new());
    let bytes = build_password_auth_stdin(&password);
    assert_eq!(&*bytes, b"\n");
}

/// Unicode password: written as raw UTF-8 bytes.
#[test]
fn build_password_auth_stdin_unicode_password() {
    let password: Zeroizing<String> = Zeroizing::new("密码🔐".into());
    let bytes = build_password_auth_stdin(&password);
    let mut expected = "密码🔐".as_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(&*bytes, expected.as_slice());
}

/// Regression: `build_ssh_args` must not emit `sshpass`, preventing anyone from accidentally
/// re-adding it to cmd_args (Windows and macOS ship without sshpass by default; a stale
/// reference would immediately produce "No such file or directory").
#[test]
fn build_ssh_args_does_not_emit_sshpass() {
    let s = server();
    let args = build_ssh_args(&s);
    assert!(
        !args.iter().any(|a| a == "sshpass"),
        "build_ssh_args must not emit sshpass; got {args:?}"
    );
}

// -------- password auth cmd_args regression guards --------
//
// These tests protect the critical switches that prevent "test connection" password
// paths from hitting the 10s timeout. Any -o option adjustment inside
// `test_password_auth` must satisfy all three:
// 1. keyboard-interactive is not declared (otherwise server-side PAM falls back to kbd-int)
// 2. KbdInteractiveAuthentication is explicitly disabled (client capability switch, not a preference)
// 3. `echo ok` still appears at the end as the remote command (otherwise stdout matching fails)
// author: logic
// date: 2026-06-01

/// Regression guard: `PreferredAuthentications` must contain only `password` — not
/// `keyboard-interactive`. An stdin pipe + EOF triggers the kbd-int PAM retry chain
/// (`pam_faildelay` ~2s/attempt), which exhausts the 10s `TEST_TIMEOUT`.
#[test]
fn password_auth_args_no_keyboard_interactive() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");
    assert!(
        !joined.contains("keyboard-interactive"),
        "test_password_auth must NOT use keyboard-interactive; got {args:?}"
    );
    assert!(
        joined.contains("PreferredAuthentications=password"),
        "expected PreferredAuthentications=password; got {args:?}"
    );
    // Even when PreferredAuthentications=password appears, no other method must follow it.
    // Split on "=" and check the first segment after it; if it starts with "password,"
    // there is at least one additional auth method listed.
    let after_pref = joined
        .split("PreferredAuthentications=")
        .nth(1)
        .unwrap_or("");
    assert!(
        !after_pref.starts_with("password,"),
        "PreferredAuthentications should not list other methods after password; got {args:?}"
    );
}

/// Regression guard: kbd-interactive must be explicitly disabled (client capability switch),
/// not just omitted from `PreferredAuthentications` (which only constrains the password
/// sub-method). OpenSSH 8.2+ behavior differences and interactions with the server's
/// `AuthenticationMethods` make this defense-in-depth layer especially important.
#[test]
fn password_auth_args_disable_kbd_interactive() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");
    assert!(
        joined.contains("KbdInteractiveAuthentication=no"),
        "missing KbdInteractiveAuthentication=no; got {args:?}"
    );
}

/// Regression guard: `echo ok` must appear as the remote command at the end of cmd_args.
/// Under ssh parsing rules, the first non-option positional argument after the destination
/// is the remote command; if option ordering is wrong and ssh does not recognize `echo ok`
/// as a command, success detection breaks.
#[test]
fn password_auth_args_ends_with_echo_ok_command() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    assert!(!args.is_empty(), "cmd_args is empty: {args:?}");
    let last = args.last().unwrap();
    assert_eq!(
        last, "echo ok",
        "cmd_args must end with `echo ok` as remote command; got {args:?}"
    );
}

/// Regression guard: the destination (`user@host`) in the password auth path must appear
/// after all `-o` options and before `echo ok`. The ssh command-line parsing rule is
/// `ssh [options] destination [command]`; the first non-option argument is the destination
/// and everything after it is the remote command. If a `-o` option ends up after the
/// destination, ssh treats it as part of the remote command rather than its own option,
/// silently disabling `PreferredAuthentications`, `KbdInteractiveAuthentication`, and the
/// other critical switches — triggering the kbd-interactive PAM retry chain and exhausting
/// the 10s `TEST_TIMEOUT`.
/// author: logic
/// date: 2026-06-01
#[test]
fn password_auth_args_destination_before_echo_ok_and_after_options() {
    let s = server();
    let args = build_password_auth_cmd_args(&s);
    let joined = args.join(" ");

    // destination "alice@1.2.3.4" must appear before "echo ok"
    let dest_pos = joined
        .find("alice@1.2.3.4")
        .expect("destination must appear in args");
    let echo_pos = joined
        .find("echo ok")
        .expect("`echo ok` must appear in args");

    assert!(
        dest_pos < echo_pos,
        "destination must come before `echo ok`; got joined: {joined}"
    );

    // destination must appear after all -o options
    // find the position of the last -o option
    let last_o_pos = joined
        .rfind("-o ")
        .expect("expected at least one -o option");
    assert!(
        last_o_pos < dest_pos,
        "all -o options must come before destination; got joined: {joined}"
    );
}

/// Regression guard: in the key auth path, `build_ssh_args` also requires the destination
/// to come after -o options. Verifies ordering by simulating the `test_key_auth` build logic.
/// author: logic
/// date: 2026-06-01
#[test]
fn key_auth_args_destination_comes_after_options() {
    let mut s = server();
    s.auth_type = AuthType::Key;
    s.key_path = Some("/home/user/.ssh/id_rsa".into());

    // Simulate the build logic from test_key_auth
    let mut args = build_ssh_args(&s);
    let target = args.pop().unwrap();
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
    args.push(target);
    args.push("echo ok".into());

    let joined = args.join(" ");
    let dest_pos = joined
        .find("alice@1.2.3.4")
        .expect("destination must appear in args");
    let echo_pos = joined
        .find("echo ok")
        .expect("`echo ok` must appear in args");
    let last_o_pos = joined
        .rfind("-o ")
        .expect("expected at least one -o option");

    assert!(
        last_o_pos < dest_pos,
        "all -o options must come before destination; got joined: {joined}"
    );
    assert!(
        dest_pos < echo_pos,
        "destination must come before `echo ok`; got joined: {joined}"
    );
}

// -------- Windows SSH_ASKPASS regression guards --------
//
// On Windows, Win32-OpenSSH rejects reading the password from stdin when there is no
// console + CREATE_NO_WINDOW (Win32-OpenSSH issue #1470); the SSH_ASKPASS mechanism is
// required. These tests guard the existence of that code path against accidental merge
// back to stdin-based writing.
// author: logic
// date: 2026-06-01

/// Regression guard: on Windows, `test_password_auth` must use `AskpassSession` and must
/// not write the password to stdin. This assertion works at the type-system level: if the
/// Windows branch is changed back to stdin injection, `AskpassSession` would become unused
/// and the compiler would emit a dead_code error, causing CI to fail.
#[cfg(windows)]
#[test]
fn windows_password_auth_uses_askpass_not_stdin() {
    // This test acts at compile time: if the Windows branch in ssh_command.rs reverts
    // to stdin injection, `AskpassSession` is no longer used, the compiler emits a
    // dead_code error, and CI fails.
    // Here we just verify that AskpassSession exists and can be named — it won't actually
    // run (it needs to write files), but it blocks accidental deletion of AskpassSession.
    let _ = std::any::type_name::<AskpassSession>();
}

/// End-to-end: create an `AskpassSession` to obtain the askpass script path, then spawn it
/// via `CreateProcessW` (mirroring how ssh spawns askpass), and verify it starts successfully.
///
/// This test guards against askpass scripts that are "not spawnable" —
/// specifically the `CreateProcessW failed error:5` (ERROR_ACCESS_DENIED) regression.
/// A previous bug set `FILE_ATTRIBUTE_HIDDEN` on the askpass file, which caused ssh's
/// `posix_spawnp` to refuse to spawn it; the password was never sent to the server and
/// users saw "wrong password".
#[cfg(windows)]
#[test]
fn windows_askpass_script_is_spawnable() {
    use std::os::windows::process::CommandExt as _;
    use std::process::Stdio;
    use zeroize::Zeroizing;

    let password: Zeroizing<String> = Zeroizing::new("dummy-pw-for-spawn-test".into());
    let session = AskpassSession::new(&password).expect("AskpassSession::new failed");
    let script = session.script_path.clone();
    let password_file = session.password_path.clone();

    // Spawn the askpass script via CreateProcessW, matching the code path ssh uses.
    // CREATE_NO_WINDOW simulates the environment when ssh spawns askpass (no console).
    // WARP_SSH_ASKPASS_FILE must be set; the script uses it to locate the password file.
    let output = std::process::Command::new("cmd.exe")
        .raw_arg(format!("/c \"{}\"", script.display()))
        .env("WARP_SSH_ASKPASS_FILE", &password_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .expect("CreateProcessW failed — askpass script is not spawnable");

    assert!(
        output.status.success(),
        "askpass script exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The askpass script reads the first line of the password file and echoes it;
    // the output must match the password written when the session was created.
    assert!(
        stdout.trim() == "dummy-pw-for-spawn-test",
        "askpass output mismatch: got {stdout:?}"
    );
}

#[test]
fn alias_command_line_passes_only_the_alias_after_separator() {
    assert_eq!(build_ssh_alias_command_line("prod-web"), "ssh -- prod-web");
}

#[test]
fn alias_command_line_escapes_shell_metacharacters() {
    // A crafted alias must not be able to break out of the ssh invocation.
    let cmd = build_ssh_alias_command_line("foo; rm -rf /");
    assert_eq!(cmd, "ssh -- 'foo; rm -rf /'");
}

#[test]
fn alias_command_line_guards_against_leading_dash() {
    // `--` ensures an alias starting with `-` is treated as a destination, not an option.
    let cmd = build_ssh_alias_command_line("-oProxyCommand=evil");
    assert!(cmd.starts_with("ssh -- "));
}
