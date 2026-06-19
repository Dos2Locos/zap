//! `~/.ssh/config` → `SshConfigCandidate` parser and one-shot loader.
//!
//! Design decisions and scope are documented in
//! `specs/gh-110-ssh-config-import/{PRODUCT,TECH}.md` (GitHub issue #110):
//! only 5 fields are supported (`Host` / `HostName` / `User` / `Port` /
//! `IdentityFile`); wildcard / negation `Host` patterns are skipped; `Match`
//! blocks are ignored entirely; `Include` emits a warning and is not followed
//! recursively; an invalid `Port` returns `None` instead of silently falling
//! back to 22.
//!
//! The parser is a pure function (`parse_ssh_config(&str) -> Vec<_>`) with no
//! IO, env, or tokio dependencies — unit tests are driven by string literals.
//! `load_candidates()` is the top-level IO wrapper; its `LoadResult` keeps the
//! path and the outcome separate so the UI can show the user which path was
//! attempted even in the NotFound / Error cases.

use std::path::PathBuf;

/// A single importable candidate sourced from a valid `Host` block in
/// `~/.ssh/config`.
///
/// Fields are a subset of the OpenSSH `ssh_config` directives — the minimal
/// set selected by PRODUCT.md decisions I/J/K. `alias` is the literal alias
/// from the `Host` line; when imported into `SshServerInfo` it becomes the
/// `host` field so that OpenSSH can still apply the advanced directives
/// (`ProxyJump`, etc.) associated with that alias when Zap later launches
/// `ssh`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshConfigCandidate {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<PathBuf>,
}

/// Parse an `ssh_config` file body and return an ordered list of candidates.
///
/// Order follows the appearance of `Host` blocks in the file; a `Host a b c`
/// line is expanded into 3 candidates that share the same body. For the exact
/// boundary rules see section 4 (F-L) of `PRODUCT.md`.
pub fn parse_ssh_config(content: &str) -> Vec<SshConfigCandidate> {
    let mut out = Vec::new();
    let mut state = ParseState::Outside;

    for line in content.lines() {
        // Everything after a `#` on the same line is treated as a comment and
        // truncated. OpenSSH's actual semantics around `#` inside vs. outside
        // quotes have some edge cases, but none of the 5 fields in scope would
        // contain a `#` in reasonable input, so naive truncation matches user
        // expectations.
        let no_comment = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };
        let trimmed = no_comment.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let keyword = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("").trim();

        if keyword.eq_ignore_ascii_case("Host") {
            flush(&mut state, &mut out);
            let aliases = parse_host_aliases(value);
            state = if aliases.is_empty() {
                // The entire line consists of wildcard / negation patterns —
                // don't start a new block, but do "consume" the following field
                // lines so they don't leak into the next valid Host. The
                // InMatch state already means "discard until the next Host", so
                // it is reused here.
                ParseState::InMatch
            } else {
                ParseState::InHost {
                    aliases,
                    body: BodyFields::default(),
                }
            };
        } else if keyword.eq_ignore_ascii_case("Match") {
            // PRODUCT.md decision H: Match blocks are ignored entirely — same
            // InMatch path as an all-wildcard Host.
            flush(&mut state, &mut out);
            state = ParseState::InMatch;
        } else if keyword.eq_ignore_ascii_case("Include") {
            // PRODUCT.md decision F: MVP does not recurse into Include — only
            // emit a warning. State is unchanged so subsequent lines still
            // belong to the current Host block (if any) — consistent with
            // OpenSSH's Include semantics (Include does not close the current
            // Host context).
            log::warn!(
                "Include directive in ssh_config is not supported by importer; \
                 hosts in `{value}` will not be imported"
            );
        } else if let ParseState::InHost { body, .. } = &mut state {
            apply_body_field(body, keyword, value);
        }
        // Any other keyword in InMatch / Outside state: ignore.
    }

    flush(&mut state, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

enum ParseState {
    /// No `Host` or `Match` directive has been seen yet.
    Outside,
    /// Currently inside a valid `Host` block. `aliases` are the literal
    /// aliases remaining after wildcard patterns have been filtered out.
    InHost {
        aliases: Vec<String>,
        body: BodyFields,
    },
    /// Currently inside an ignored block (`Match` or an all-wildcard `Host`);
    /// field lines are consumed and discarded until the next `Host` or EOF.
    InMatch,
}

#[derive(Default, Clone)]
struct BodyFields {
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<PathBuf>,
}

fn flush(state: &mut ParseState, out: &mut Vec<SshConfigCandidate>) {
    let prev = std::mem::replace(state, ParseState::Outside);
    if let ParseState::InHost { aliases, body } = prev {
        for alias in aliases {
            out.push(SshConfigCandidate {
                alias,
                hostname: body.hostname.clone(),
                user: body.user.clone(),
                port: body.port,
                identity_file: body.identity_file.clone(),
            });
        }
    }
}

/// Parse a line like `Host a *.prod b !bad` into `["a", "b"]`.
fn parse_host_aliases(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|tok| !tok.contains('*') && !tok.contains('?') && !tok.contains('!'))
        .map(|s| s.to_string())
        .collect()
}

/// Apply one field to the current Host block body. **First occurrence wins**
/// (matching OpenSSH semantics).
fn apply_body_field(body: &mut BodyFields, keyword: &str, value: &str) {
    if keyword.eq_ignore_ascii_case("HostName") {
        if body.hostname.is_none() {
            body.hostname = Some(value.to_string());
        }
    } else if keyword.eq_ignore_ascii_case("User") {
        if body.user.is_none() {
            body.user = Some(value.to_string());
        }
    } else if keyword.eq_ignore_ascii_case("Port") {
        // Note: the first *declaration* wins, not the first *valid* one —
        // but because a failed Port parse yields None (PRODUCT.md decision K),
        // "already declared" and "value is Some" are equivalent here. Using
        // is_none as the guard keeps things simple.
        if body.port.is_none() {
            body.port = value.parse::<u16>().ok();
        }
    } else if keyword.eq_ignore_ascii_case("IdentityFile") && body.identity_file.is_none() {
        let unquoted = strip_surrounding_quotes(value);
        body.identity_file = Some(expand_tilde(unquoted));
    }
    // All other keywords: ignored (MVP supports only 5 fields).
}

fn strip_surrounding_quotes(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(s)
}

/// The default `~/.ssh/config` path for the current user, cross-platform.
///
/// Returns `None` in the rare case where the home directory cannot be
/// determined.
pub fn default_ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("config"))
}

/// The parsed result together with the path that was attempted, for use by
/// the UI when displaying error / empty states.
#[derive(Debug)]
pub struct LoadResult {
    /// The path that was actually attempted. `None` means the home directory
    /// itself could not be determined.
    pub path: Option<PathBuf>,
    pub outcome: LoadOutcome,
}

#[derive(Debug)]
pub enum LoadOutcome {
    /// The file was read and parsed successfully (the list may be empty).
    Loaded(Vec<SshConfigCandidate>),
    /// The path does not exist — a clean state; the UI should show a
    /// "not found" hint rather than an error.
    NotFound,
    /// An IO error occurred (permissions, encoding, disk, etc.). The `String`
    /// is a human-readable message for display.
    Error(String),
}

/// One-shot loader for the default `~/.ssh/config` path; returns the path and
/// the outcome.
///
/// Designed to be synchronous and panic-free: the UI calls this once when the
/// panel first opens; a typical config file is <10 KB, so synchronous IO is
/// fast enough. Missing-file and permission errors are mapped to `NotFound` /
/// `Error` respectively — nothing is propagated as a panic.
pub fn load_candidates() -> LoadResult {
    match default_ssh_config_path() {
        Some(p) => load_candidates_from(&p),
        None => LoadResult {
            path: None,
            outcome: LoadOutcome::Error("Could not determine home directory".into()),
        },
    }
}

/// Same as [`load_candidates`] but lets the caller supply the path explicitly —
/// primarily for unit tests (tempfile), and also as a hook for a future
/// "custom config path" setting.
pub fn load_candidates_from(path: &std::path::Path) -> LoadResult {
    let outcome = match std::fs::read_to_string(path) {
        Ok(s) => LoadOutcome::Loaded(parse_ssh_config(&s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadOutcome::NotFound,
        Err(e) => LoadOutcome::Error(format!("{e}")),
    };
    LoadResult {
        path: Some(path.to_path_buf()),
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience constructor for tests: all fields default to `None`; only
    /// the fields relevant to a given test case need to be filled in.
    fn cand(alias: &str) -> SshConfigCandidate {
        SshConfigCandidate {
            alias: alias.into(),
            hostname: None,
            user: None,
            port: None,
            identity_file: None,
        }
    }

    /// The simplest happy path: one `Host` block with all 5 fields produces a
    /// single candidate. This test drives the minimal "Host block recognition +
    /// field parsing" main path; subsequent cases add state-machine branches on
    /// top of it.
    #[test]
    fn single_host_with_all_fields() {
        let input = "\
Host prodbox
    HostName prod.example.com
    User alice
    Port 2222
    IdentityFile /home/alice/.ssh/id_ed25519
";
        let got = parse_ssh_config(input);
        assert_eq!(
            got,
            vec![SshConfigCandidate {
                alias: "prodbox".into(),
                hostname: Some("prod.example.com".into()),
                user: Some("alice".into()),
                port: Some(2222),
                identity_file: Some(PathBuf::from("/home/alice/.ssh/id_ed25519")),
            }]
        );
    }

    #[test]
    fn empty_file_produces_no_candidates() {
        assert_eq!(parse_ssh_config(""), vec![]);
    }

    #[test]
    fn comments_only_produces_no_candidates() {
        assert_eq!(parse_ssh_config("# top comment\n# another\n"), vec![]);
    }

    #[test]
    fn host_with_only_alias_has_no_hostname_field() {
        // The importer layer (outside this module) uses `alias` as
        // `server.host`; here we only verify the parser does not fabricate a
        // hostname.
        assert_eq!(parse_ssh_config("Host foo\n"), vec![cand("foo")]);
    }

    #[test]
    fn multiple_hosts_in_order() {
        let input = "\
Host a
    User x
Host b
    User y
Host c
    User z
";
        let got = parse_ssh_config(input);
        let users: Vec<_> = got
            .iter()
            .map(|c| (c.alias.as_str(), c.user.as_deref()))
            .collect();
        assert_eq!(
            users,
            vec![("a", Some("x")), ("b", Some("y")), ("c", Some("z"))]
        );
    }

    #[test]
    fn wildcard_star_host_skipped() {
        // PRODUCT.md decision G: `Host *.prod` is a template, not a machine —
        // it must not appear in the candidate list.
        let input = "\
Host *.prod
    User root
Host realbox
    User me
";
        let got = parse_ssh_config(input);
        assert_eq!(
            got,
            vec![SshConfigCandidate {
                user: Some("me".into()),
                ..cand("realbox")
            }]
        );
    }

    #[test]
    fn wildcard_question_host_skipped() {
        let input = "\
Host srv?
    User x
";
        assert_eq!(parse_ssh_config(input), vec![]);
    }

    #[test]
    fn negation_host_skipped() {
        let input = "\
Host !bad
    User x
";
        assert_eq!(parse_ssh_config(input), vec![]);
    }

    #[test]
    fn host_with_multiple_aliases_expands_to_separate_candidates() {
        // PRODUCT.md decision L: `Host a b c` shares a body.
        let input = "\
Host a b c
    Port 22
    User shared
";
        let got = parse_ssh_config(input);
        assert_eq!(got.len(), 3);
        for (i, alias) in ["a", "b", "c"].iter().enumerate() {
            assert_eq!(got[i].alias, *alias);
            assert_eq!(got[i].port, Some(22));
            assert_eq!(got[i].user.as_deref(), Some("shared"));
        }
    }

    #[test]
    fn host_with_mixed_aliases_filters_wildcards_keeps_literals() {
        // `Host a *.prod b` → only a and b are exported.
        let input = "\
Host a *.prod b
    User shared
";
        let got = parse_ssh_config(input);
        let aliases: Vec<&str> = got.iter().map(|c| c.alias.as_str()).collect();
        assert_eq!(aliases, vec!["a", "b"]);
    }

    #[test]
    fn match_block_ignored_until_next_host() {
        // PRODUCT.md decision H: a `Match` block is ignored entirely — it must
        // not "pollute" the preceding Host body, and it must not become a new
        // candidate.
        let input = "\
Host a
    User u_a
Match user someone
    User SHOULD_NOT_APPEAR
    Port 9999
Host b
    User u_b
";
        let got = parse_ssh_config(input);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].alias, "a");
        assert_eq!(got[0].user.as_deref(), Some("u_a"));
        assert_eq!(got[0].port, None, "Port 9999 from Match block must not leak into a");
        assert_eq!(got[1].alias, "b");
        assert_eq!(got[1].user.as_deref(), Some("u_b"));
    }

    #[test]
    fn match_block_at_eof_does_not_panic() {
        let input = "\
Host a
    User u
Match user x
    User leak
";
        let got = parse_ssh_config(input);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].alias, "a");
        assert_eq!(got[0].user.as_deref(), Some("u"));
    }

    #[test]
    fn include_directive_logged_and_skipped_outside_host() {
        // PRODUCT.md decision F: `Include` is not followed — only a warning is
        // emitted; subsequent parsing continues normally.
        let input = "\
Include ~/.ssh/work/*.conf
Host a
    User u
";
        let got = parse_ssh_config(input);
        assert_eq!(
            got,
            vec![SshConfigCandidate {
                user: Some("u".into()),
                ..cand("a")
            }]
        );
    }

    #[test]
    fn port_invalid_string_yields_none() {
        // PRODUCT.md decision K: do not silently fall back to 22; show the
        // empty port to the user in the UI.
        let input = "Host a\n    Port not-a-number\n";
        assert_eq!(parse_ssh_config(input)[0].port, None);
    }

    #[test]
    fn port_out_of_u16_range_yields_none() {
        let input = "Host a\n    Port 70000\n";
        assert_eq!(parse_ssh_config(input)[0].port, None);
    }

    #[test]
    fn port_valid_yields_some() {
        let input = "Host a\n    Port 2222\n";
        assert_eq!(parse_ssh_config(input)[0].port, Some(2222));
    }

    #[test]
    fn quoted_identity_file_has_quotes_stripped() {
        // OpenSSH allows paths with spaces to be wrapped in quotes.
        let input = "Host a\n    IdentityFile \"C:\\Users\\Jiaqi Jiang\\.ssh\\id\"\n";
        assert_eq!(
            parse_ssh_config(input)[0].identity_file,
            Some(PathBuf::from("C:\\Users\\Jiaqi Jiang\\.ssh\\id"))
        );
    }

    #[test]
    fn tilde_in_identity_file_expanded_to_home() {
        // ~/x is expanded to $HOME/x. $HOME varies across CI environments, so
        // only the prefix is asserted.
        let input = "Host a\n    IdentityFile ~/keys/id\n";
        let got = parse_ssh_config(input);
        let path = got[0].identity_file.as_ref().expect("IdentityFile set");
        let home = dirs::home_dir().expect("test runner has home dir");
        assert!(
            path.starts_with(&home),
            "expected {path:?} to start with {home:?}"
        );
        assert!(
            path.ends_with("keys/id"),
            "expected {path:?} to end with keys/id"
        );
    }

    #[test]
    fn case_insensitive_keywords() {
        let input = "host a\n    hOsTnAmE example.com\n    user alice\n    PORT 22\n";
        let got = parse_ssh_config(input);
        assert_eq!(
            got,
            vec![SshConfigCandidate {
                alias: "a".into(),
                hostname: Some("example.com".into()),
                user: Some("alice".into()),
                port: Some(22),
                identity_file: None,
            }]
        );
    }

    #[test]
    fn repeated_field_first_wins() {
        // Matches OpenSSH semantics: within the same Host block, the first
        // occurrence of a field wins.
        let input = "Host a\n    Port 1\n    Port 2\n    User first\n    User second\n";
        let got = parse_ssh_config(input);
        assert_eq!(got[0].port, Some(1));
        assert_eq!(got[0].user.as_deref(), Some("first"));
    }

    #[test]
    fn inline_trailing_comment_dropped_from_value() {
        // OpenSSH's handling of inline `#` has some edge cases around quotes;
        // we take the conservative approach: scan the whole line and truncate
        // at the first `#`, which is correct for unquoted values.
        let input = "Host a # primary box\n    User alice # admin\n";
        let got = parse_ssh_config(input);
        assert_eq!(got[0].alias, "a");
        assert_eq!(got[0].user.as_deref(), Some("alice"));
    }

    #[test]
    fn leading_indent_tolerated() {
        // OpenSSH allows arbitrary leading whitespace.
        let input = "  Host a\n\t  Port 22\n";
        let got = parse_ssh_config(input);
        assert_eq!(got[0].alias, "a");
        assert_eq!(got[0].port, Some(22));
    }

    // -----------------------------------------------------------------
    // default_ssh_config_path / load_candidates_from / load_candidates
    // -----------------------------------------------------------------

    #[test]
    fn default_path_points_under_home_dot_ssh_config() {
        // Cross-platform: as long as dirs::home_dir() returns a value, the
        // result must be `<home>/.ssh/config`. CI runners always have HOME /
        // USERPROFILE set.
        let got = default_ssh_config_path().expect("test runner has home dir");
        let home = dirs::home_dir().expect("test runner has home dir");
        assert!(got.starts_with(&home), "{got:?} should start with {home:?}");
        assert!(got.ends_with("config"));
        assert!(
            got.to_string_lossy()
                .replace('\\', "/")
                .ends_with(".ssh/config"),
            "{got:?} should end with .ssh/config"
        );
    }

    #[test]
    fn load_candidates_from_nonexistent_path_returns_not_found() {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let path = tmp.path().join("does_not_exist");
        let res = load_candidates_from(&path);
        assert_eq!(res.path.as_deref(), Some(path.as_path()));
        assert!(
            matches!(res.outcome, LoadOutcome::NotFound),
            "got {:?}",
            res.outcome
        );
    }

    #[test]
    fn load_candidates_from_valid_file_returns_parsed_candidates() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().expect("create tempfile");
        writeln!(tmp, "Host a\n    User u\n").expect("write tempfile");
        let res = load_candidates_from(tmp.path());
        match res.outcome {
            LoadOutcome::Loaded(v) => {
                assert_eq!(v.len(), 1);
                assert_eq!(v[0].alias, "a");
                assert_eq!(v[0].user.as_deref(), Some("u"));
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn load_candidates_from_empty_file_returns_loaded_empty() {
        let tmp = tempfile::NamedTempFile::new().expect("create tempfile");
        let res = load_candidates_from(tmp.path());
        match res.outcome {
            LoadOutcome::Loaded(v) => assert!(v.is_empty()),
            other => panic!("expected Loaded(empty), got {other:?}"),
        }
    }
}
