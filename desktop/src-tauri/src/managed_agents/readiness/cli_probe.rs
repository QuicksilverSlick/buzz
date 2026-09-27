use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::managed_agents::{resolve_command, runtime::build_augmented_path};

/// Build the augmented PATH for CLI probes and other native child processes
/// (auth commands, `buzz-acp models` discovery), including nvm's default
/// Node.js bin directory so `#!/usr/bin/env node` shims (e.g. codex-acp)
/// resolve.
pub(crate) fn augmented_path() -> Option<String> {
    let home = dirs::home_dir();
    let nvm_bin = home
        .as_deref()
        .and_then(crate::managed_agents::find_nvm_default_bin);
    build_augmented_path(
        home,
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        crate::managed_agents::login_shell_path(),
        nvm_bin,
    )
}

/// Outcome of a CLI login-status probe.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProbeOutcome {
    /// The CLI reported a successful login (exit 0).
    LoggedIn,
    /// The CLI exited non-zero without a config-parse signal — treat as
    /// "not authenticated."
    LoggedOut,
    /// The CLI exited non-zero and its stderr contains a config-parse error
    /// (e.g. from `~/.codex/config.toml`). The user needs to fix their
    /// config, not re-run login.
    ConfigInvalid {
        /// A trimmed excerpt of the stderr message to surface in the nudge.
        stderr_excerpt: String,
    },
}

/// Signals emitted to stderr by codex (and related CLI tools) when they
/// fail to parse their config file. We check these to distinguish a
/// config-parse failure from a genuine "not authenticated" exit.
///
/// The real codex error reads:
///   `Error loading configuration: .../.codex/config.toml:... unknown variant ...`
/// So we require BOTH "error loading configuration" AND "unknown variant" to be
/// present, avoiding false positives from unrelated errors that mention only
/// one term.
const CONFIG_PARSE_SIGNALS: &[&str] = &["error loading configuration", "unknown variant"];

/// `CODEX_PATH` as a spawned codex-acp sees it: the agent's env over the app
/// env the spawn inherits. Empty counts as unset, as it does in codex-acp.
pub(crate) fn codex_path_env(agent_env: Option<&BTreeMap<String, String>>) -> Option<String> {
    agent_env
        .and_then(|env| env.get("CODEX_PATH").cloned())
        .or_else(|| std::env::var("CODEX_PATH").ok())
        .filter(|path| !path.is_empty())
}

/// The program and argv (`argv[0]` is only a label) a login probe runs.
///
/// Codex is probed with the engine the agent will run, not whichever `codex`
/// is first on PATH: codex-acp spawns `CODEX_PATH` when set, else the
/// `@openai/codex` it bundles. An older global CLI can reject config values
/// the bundled engine accepts, which parks a working agent in setup mode.
pub(crate) fn probe_command(
    probe_args: &[&str],
    adapter_path: Option<&Path>,
    codex_path: Option<&str>,
) -> Option<(PathBuf, Vec<String>)> {
    let owned = |args: &[&str]| args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    if probe_args[0] == "codex" {
        if let Some(path) = codex_path.and_then(resolve_command) {
            return Some((path, owned(probe_args)));
        }
        if let Some(codex_js) = adapter_path.and_then(bundled_codex_js) {
            let mut argv = vec!["node".to_string(), codex_js.to_string_lossy().into_owned()];
            argv.extend(owned(&probe_args[1..]));
            // Bare `node`, resolved through the probe's augmented PATH exactly
            // as the adapter's own shim resolves it.
            return Some((PathBuf::from("node"), argv));
        }
    }
    Some((resolve_command(probe_args[0])?, owned(probe_args)))
}

/// `@openai/codex/bin/codex.js` as Node resolves it from the codex-acp
/// package: nested under the package first, then hoisted into an ancestor.
fn bundled_codex_js(adapter_path: &Path) -> Option<PathBuf> {
    // npm's Windows `.cmd` (and sh) shims sit beside `node_modules`; a unix
    // bin symlink resolves into the package's `dist/`.
    let shim_package: PathBuf = ["node_modules", "@agentclientprotocol", "codex-acp"]
        .iter()
        .collect();
    let shim_package = adapter_path.parent()?.join(shim_package);
    let start = if shim_package.is_dir() {
        shim_package
    } else {
        adapter_path.canonicalize().ok()?.parent()?.to_path_buf()
    };
    let codex_js: PathBuf = ["node_modules", "@openai", "codex", "bin", "codex.js"]
        .iter()
        .collect();
    start
        .ancestors()
        .map(|dir| dir.join(&codex_js))
        .find(|path| path.is_file())
}

/// Run the probe at the resolved absolute path so the GUI-PATH gap is
/// bypassed. Injects the same augmented PATH used for launched agents so
/// script shims with `/usr/bin/env <interpreter>` shebangs can find runtimes
/// such as node/python when the app was launched with a bare GUI PATH.
pub(crate) fn login_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> ProbeOutcome {
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);

    match command.output() {
        Ok(o) if o.status.success() => ProbeOutcome::LoggedIn,
        Ok(o) => classify_probe_output(&o.stderr, false),
        Err(_) => ProbeOutcome::LoggedOut,
    }
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared between `login_probe` (which has the full `Output`) and the
/// process-level timeout path in `probe_auth_status` (which drains stderr
/// on a background thread and collects it separately).
pub(crate) fn classify_probe_output(stderr_bytes: &[u8], exit_success: bool) -> ProbeOutcome {
    if exit_success {
        return ProbeOutcome::LoggedIn;
    }
    let stderr = String::from_utf8_lossy(stderr_bytes);
    let stderr_lower = stderr.to_lowercase();
    if CONFIG_PARSE_SIGNALS
        .iter()
        .all(|sig| stderr_lower.contains(sig))
    {
        let excerpt = stderr.trim().lines().next().unwrap_or("").to_string();
        ProbeOutcome::ConfigInvalid {
            stderr_excerpt: excerpt,
        }
    } else {
        ProbeOutcome::LoggedOut
    }
}

#[cfg(test)]
mod tests {
    use super::{ProbeOutcome, CONFIG_PARSE_SIGNALS};

    #[cfg(unix)]
    #[test]
    fn login_probe_uses_augmented_path_for_env_shebang_interpreter() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let temp = tempfile::tempdir().expect("temp dir");
        let script_dir = temp.path().join("script-bin");
        let interpreter_dir = temp.path().join("interpreter-bin");
        let empty_path_dir = temp.path().join("empty-bin");
        fs::create_dir_all(&script_dir).expect("script dir");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
        fs::create_dir_all(&empty_path_dir).expect("empty path dir");

        let interpreter_path = interpreter_dir.join("node");
        let marker_path = temp.path().join("fake-node-ran");
        fs::write(
            &interpreter_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nexit 0\n",
                marker_path.display()
            ),
        )
        .expect("write interpreter");
        fs::set_permissions(&interpreter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod interpreter");

        let script_path = script_dir.join("fake-codex");
        fs::write(&script_path, "#!/usr/bin/env node\n").expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let scrubbed_path = std::env::join_paths([empty_path_dir.as_path()])
            .expect("join scrubbed PATH")
            .to_string_lossy()
            .into_owned();
        let without_augmented_path = Command::new(&script_path)
            .args(["login", "status"])
            .env("PATH", &scrubbed_path)
            .output()
            .expect("run script with scrubbed PATH");
        assert!(
            !without_augmented_path.status.success(),
            "with a scrubbed PATH, /usr/bin/env should not find node"
        );

        let augmented_path =
            std::env::join_paths([interpreter_dir.as_path()]).expect("join augmented PATH");
        let augmented_path = augmented_path.to_string_lossy().into_owned();
        assert_eq!(
            super::login_probe(
                &script_path,
                &["fake-codex", "login", "status"],
                Some(&augmented_path),
            ),
            ProbeOutcome::LoggedIn,
            "the injected augmented PATH should allow /usr/bin/env to find the interpreter"
        );
        assert!(
            marker_path.exists(),
            "the fake node from the injected PATH should have run"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_config_invalid_on_stderr_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 and writes a codex-style config-parse error to stderr.
        let script_path = bin_dir.join("fake-codex-bad-config");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'Error loading configuration: /home/user/.codex/config.toml: unknown variant `ultra`, expected one of none/minimal/low/medium/high/xhigh' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-bad-config", "login", "status"],
            None,
        );
        assert!(
            matches!(outcome, ProbeOutcome::ConfigInvalid { .. }),
            "stderr with 'unknown variant' should produce ConfigInvalid; got {:?}",
            outcome
        );
        if let ProbeOutcome::ConfigInvalid { stderr_excerpt } = outcome {
            assert!(
                stderr_excerpt.contains("unknown variant")
                    || stderr_excerpt.contains("Error loading"),
                "stderr_excerpt should contain the parse error: {stderr_excerpt}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_logged_out_on_nonzero_without_config_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 with a generic "not logged in" message.
        let script_path = bin_dir.join("fake-codex-logged-out");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'not authenticated' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-logged-out", "login", "status"],
            None,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::LoggedOut,
            "non-config stderr should produce LoggedOut"
        );
    }

    /// The probe must run the Codex engine codex-acp bundles, not whichever
    /// `codex` is first on PATH: an older global CLI rejects config values the
    /// bundled engine accepts and parks a working agent in setup mode.
    #[test]
    fn codex_probe_runs_engine_bundled_with_adapter_not_path_codex() {
        use std::path::PathBuf;

        let temp = tempfile::tempdir().expect("temp dir");
        let package: PathBuf = ["node_modules", "@agentclientprotocol", "codex-acp"]
            .iter()
            .collect();
        let package = temp.path().join(package);
        let codex_bin: PathBuf = ["node_modules", "@openai", "codex", "bin"].iter().collect();
        std::fs::create_dir_all(package.join(&codex_bin)).expect("bundled codex dir");
        let codex_js = package.join(&codex_bin).join("codex.js");
        std::fs::write(&codex_js, "").expect("write codex.js");
        let probe = |adapter: &std::path::Path| {
            super::probe_command(&["codex", "login", "status"], Some(adapter), None)
                .expect("a codex probe command")
        };
        let expected = |js: &std::path::Path| {
            let js = js.to_string_lossy().into_owned();
            (
                PathBuf::from("node"),
                vec!["node".to_string(), js, "login".into(), "status".into()],
            )
        };

        // npm's Windows `.cmd` / sh shim layout: the shim sits beside node_modules.
        let shim = temp.path().join("codex-acp.cmd");
        std::fs::write(&shim, "").expect("write shim");
        assert_eq!(probe(&shim), expected(&codex_js));

        // Unix npm-global layout: bin/codex-acp links into the package's dist/.
        #[cfg(unix)]
        {
            let dist = package.join("dist");
            std::fs::create_dir_all(&dist).expect("dist dir");
            std::fs::write(dist.join("index.js"), "").expect("write index.js");
            let bin = temp.path().join("bin");
            std::fs::create_dir_all(&bin).expect("bin dir");
            std::os::unix::fs::symlink(dist.join("index.js"), bin.join("codex-acp"))
                .expect("symlink adapter");
            let codex_js = codex_js.canonicalize().expect("canonical codex.js");
            assert_eq!(probe(&bin.join("codex-acp")), expected(&codex_js));
        }
    }

    /// Verify that every string in CONFIG_PARSE_SIGNALS is lowercased so the
    /// case-insensitive match works correctly.
    #[test]
    fn config_parse_signals_are_lowercase() {
        for sig in CONFIG_PARSE_SIGNALS {
            assert_eq!(
                *sig,
                sig.to_lowercase(),
                "CONFIG_PARSE_SIGNAL must be lowercase for case-insensitive matching: {sig}"
            );
        }
    }
}
