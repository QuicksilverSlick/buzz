//! A login's token from a DPAPI file (`label=dpapi:<path>`): kept nowhere
//! here, and handed to the child only in its environment.

use super::signin_check::SignIn;

/// Why a file gave no token: a sign-in check's verdict and fixed words, so
/// neither a value nor anything PowerShell printed can reach a log.
pub(super) type Unread = (SignIn, &'static str);

/// A login's launch environment, or why its file gave none.
pub(super) type LoginEnv = Result<Vec<(String, String)>, Unread>;

/// What reads a token file; a test injects its own.
pub(super) type Decrypt = fn(&str) -> Result<String, Unread>;

/// The token in `file`. Blank is a failure: a failed decrypt prints nothing,
/// and the Claude CLI reads a blank token as signed out.
pub(super) fn token(file: &str, read: Option<Decrypt>) -> Result<String, Unread> {
    match read.unwrap_or(decrypt)(file) {
        Ok(token) if token.trim().is_empty() => Err((
            SignIn::SignedOut,
            "the file is missing or did not decrypt for this Windows user",
        )),
        read => read,
    }
}

/// The kit's `lib.mjs` `decrypt` call. The path travels in the child's
/// environment, so it is data to PowerShell and never command text.
#[cfg(windows)]
fn powershell(file: &str) -> tokio::process::Command {
    use std::process::Stdio;
    const DECRYPT: &str = "$ErrorActionPreference = 'Stop'; \
        $s = (Get-Content -LiteralPath $env:BUZZ_ACP_DPAPI_FILE -Raw).Trim() | ConvertTo-SecureString; \
        [Runtime.InteropServices.Marshal]::PtrToStringBSTR(\
        [Runtime.InteropServices.Marshal]::SecureStringToBSTR($s))";
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", DECRYPT])
        .env("BUZZ_ACP_DPAPI_FILE", file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::acp::configure_no_window(&mut cmd);
    cmd
}

/// ponytail: blocks the caller and the factory lock for one PowerShell start
/// (about a second, 15 s at worst) at each launch and each 15-minute check.
/// Move it off the lock if launches ever become frequent.
#[cfg(windows)]
fn decrypt(file: &str) -> Result<String, Unread> {
    use std::io::Read;
    use std::time::{Duration, Instant};
    let no_answer = (SignIn::Unknown, "PowerShell did not answer");
    let mut child = powershell(file).into_std().spawn().map_err(|_| no_answer)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !matches!(child.try_wait(), Ok(Some(_))) {
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Err(no_answer);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut token = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut token);
    }
    Ok(token.trim().to_string())
}

#[cfg(not(windows))]
fn decrypt(_file: &str) -> Result<String, Unread> {
    Err((SignIn::SignedOut, "a dpapi: login only works on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpError;
    use crate::factory::{classify, Factory, OAUTH_TOKEN_ENV};
    use crate::scope::SessionScope;
    use chrono::Utc;

    const TOKEN: &str = "fake-token-for-tests";
    const DEAD: Unread = (SignIn::SignedOut, "it did not decrypt");
    const SILENT: Unread = (SignIn::Unknown, "no answer");

    /// Stands in for DPAPI: no test here reaches PowerShell.
    fn fake(file: &str) -> Result<String, Unread> {
        match file {
            good if good.ends_with("good") => Ok(TOKEN.to_string()),
            "blank" => Ok("  ".to_string()),
            "hung" => Err(SILENT),
            _ => Err(DEAD),
        }
    }

    fn pool(entries: &str) -> Factory {
        let mut f = Factory::new(Some(entries), "claude-agent-acp");
        f.decrypt = Some(fake);
        f
    }

    fn pair(key: &str, value: &str) -> Vec<(String, String)> {
        vec![(key.to_string(), value.to_string())]
    }

    #[test]
    fn the_order_is_own_token_then_the_file_then_the_folder() {
        let now = Utc::now();
        let mut f = pool("token=env;primary=dpapi:good;fallback=C:/f");
        assert!(f.launch_env(0, now).is_empty());
        assert_eq!(f.label(0), "token");
        f.signin_checked(0, SignIn::SignedOut, now);
        assert_eq!(f.launch_env(0, now), pair(OAUTH_TOKEN_ENV, TOKEN));
        assert_eq!(f.label(0), "primary");
        assert!(!format!("{f:?}").contains(TOKEN), "the value is not kept");
        f.signin_checked(1, SignIn::SignedOut, now);
        assert_eq!(f.launch_env(0, now), pair("CLAUDE_CONFIG_DIR", "C:/f"));
        assert_eq!(f.label(0), "fallback");
    }

    /// Missing, blank or silent, a file fails its own entry and the pool
    /// moves on. With nothing left the launch fails plainly, naming the
    /// entry: never a blank token, never the agent's own environment.
    #[test]
    fn a_dead_file_moves_on_and_a_pool_of_only_dead_files_refuses() {
        let now = Utc::now();
        for file in ["missing", "blank", "hung"] {
            let mut f = pool(&format!("primary=dpapi:{file};fallback=C:/f"));
            let env = f.launch_env(0, now);
            assert_eq!(env, pair("CLAUDE_CONFIG_DIR", "C:/f"), "{file}");
            assert_eq!(f.label(0), "fallback", "{file}");
            assert!(f.accounts()[0].signed_out, "{file}");
            assert_eq!(f.take_alerts().len(), 1, "the owner hears: {file}");
        }
        let why = pool("primary=dpapi:missing").login_env(0, now);
        assert_eq!(why, Err("login primary: it did not decrypt".to_string()));
    }

    /// The check gets a dead file's verdict; a silent PowerShell is no verdict.
    #[test]
    fn the_check_gets_each_files_own_answer() {
        let f = pool("token=env;a=dpapi:missing;b=dpapi:hung;c=dpapi:good");
        let targets = f.signin_check_targets();
        assert_eq!(targets[1].1, Err(DEAD));
        assert_eq!(targets[2].1, Err(SILENT));
        assert_eq!(targets[3].1, Ok(pair(OAUTH_TOKEN_ENV, TOKEN)));
    }

    fn refuse(f: &mut Factory, now: chrono::DateTime<Utc>) {
        let channel_id = uuid::Uuid::from_u128(7);
        let scope = SessionScope::Conversation { channel_id };
        let error = AcpError::agent_error(-32603, "API Error: 401 expired");
        f.plan(0, &scope, &classify(&error, now), now);
    }

    /// A server's 401 is about the token, and `claude auth status` never asks
    /// the server, so its yes cannot bring the login back. A new file can.
    #[test]
    fn a_refused_token_stays_out_until_its_file_changes() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("good");
        std::fs::write(&file, "cipher").unwrap();
        let mut f = pool(&format!("primary=dpapi:{};fallback=C:/f", file.display()));
        f.launch_env(0, now);
        refuse(&mut f, now);
        f.launch_env(0, now);
        f.take_alerts();
        assert!(!f.signin_checked(0, SignIn::Valid, now), "nothing changed");
        assert!(f.accounts()[0].signed_out);
        assert!(!f.wants_restart(0, now), "the work stays off it");
        assert!(f.take_alerts().is_empty(), "no all-clear to take back");

        let replaced = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let handle = std::fs::File::options().write(true).open(&file).unwrap();
        handle.set_modified(replaced).unwrap();
        assert!(f.signin_checked(0, SignIn::Valid, now), "the new file");
        assert!(!f.accounts()[0].signed_out);
        assert!(f.wants_restart(0, now), "back to the first login");
    }

    /// Only a refusal waits for a new file: a real answer ends it, and a file
    /// that could not be read says nothing about its token.
    #[test]
    fn a_real_answer_ends_a_refusal_and_a_good_read_clears_an_unread_file() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        for name in ["good", "missing"] {
            std::fs::write(dir.path().join(name), "cipher").unwrap();
        }
        let good = dir.path().join("good");
        let mut f = pool(&format!("primary=dpapi:{}", good.display()));
        f.launch_env(0, now);
        refuse(&mut f, now);
        f.launch_env(0, now);
        f.on_success(0, now);
        f.signin_checked(0, SignIn::SignedOut, now);
        f.signin_checked(0, SignIn::Valid, now);
        assert!(!f.accounts()[0].signed_out, "the refusal is over");

        let missing = dir.path().join("missing");
        let mut f = pool(&format!(
            "primary=dpapi:{};fallback=C:/f",
            missing.display()
        ));
        f.launch_env(0, now);
        assert!(f.accounts()[0].signed_out);
        f.signin_checked(0, SignIn::Valid, now);
        assert!(!f.accounts()[0].signed_out, "a good read clears it");
    }

    #[cfg(not(windows))]
    #[test]
    fn off_windows_a_dpapi_entry_fails_plainly() {
        assert!(token("good", None).is_err());
    }
}
