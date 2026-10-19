//! Texting the owner when a Critical opens (factory resilience spec, M2
//! item 4).
//!
//! M1 raised a Critical card in the Bench inbox and posted a STATE record to
//! #factory. Both of those are inside the Dreamforge app, so when Reset
//! Worker lost its Claude sign-in on 18 Oct nothing reached the owner at
//! all: a switch went off and nobody knew, and the outage was found three
//! hours later by a real request failing. A text does reach them.
//!
//! So every Critical also queues one short message, which the harness hands
//! to a command the owner configures — the same `send_sms.py` skill their
//! agents already use. One text when an incident opens and one when it
//! closes, and never a token, a path, an email or anything a member wrote.

use std::time::Duration;

use super::{truncate, Factory, TEXT_MAX};

/// The command line the harness runs to text the owner; the alert text is
/// its last argument. Unset: no texts are sent.
pub(crate) const ALERT_CMD_ENV: &str = "BUZZ_FACTORY_ALERT_CMD";

/// Who the text goes to, placed in the alert command's own environment as
/// [`SMS_TO_ENV`] and nowhere else.
pub(crate) const ALERT_SMS_TO_ENV: &str = "BUZZ_FACTORY_ALERT_SMS_TO";

/// The variable the owner's `send_sms.py` reads its recipient from.
const SMS_TO_ENV: &str = "SMS_TO";

/// A send that has not finished in this long is abandoned.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

impl Factory {
    /// Queue the owner's text for a Critical that just opened.
    ///
    /// At most one per incident, whatever else that incident goes on to
    /// raise: two Criticals in one outage are still one outage, and an agent
    /// that probes every 30 minutes must not text every 30 minutes. `text`
    /// is the card's own wording, which is already free of tokens, paths,
    /// emails and member text.
    pub(super) fn alert_critical(&mut self, text: &str) {
        let incident = self.incident.clone().unwrap_or_default();
        if self.alerted.as_ref() == Some(&incident) {
            return;
        }
        self.alerted = Some(incident);
        self.queue_alert(&format!("Buzz: {text}"));
    }

    /// Queue a line about something the owner will want to act on that is
    /// not an incident: a spare login of the pool that needs signing in
    /// again while the agent itself keeps working.
    ///
    /// Outside [`alert_critical`]'s dedupe on purpose. That one keys on the
    /// incident, so borrowing it here would both swallow the next real
    /// Critical's text and leave an all-clear owed for an incident that
    /// never opened. The caller's own state is the dedupe instead — a
    /// login's `signed_out` flag, which the 15-minute check sets once and
    /// only a passing check or a real answer clears — so this is at most one
    /// text per episode either way.
    pub(super) fn alert_aside(&mut self, text: &str) {
        self.queue_alert(&format!("Buzz: {text}"));
    }

    /// Queue the all-clear when the incident the owner was texted about
    /// closes. Never without an opening text, and never twice: an owner who
    /// was not told it broke does not need to be told it is fine.
    pub(super) fn alert_all_clear(&mut self) {
        if self.alerted.take().is_none() {
            return;
        }
        let who = self.who().to_string();
        self.queue_alert(&format!(
            "Buzz: all clear — {who} is working normally again."
        ));
    }

    /// One message, so it is cut to [`TEXT_MAX`] characters including the
    /// marker [`truncate`] leaves behind.
    fn queue_alert(&mut self, text: &str) {
        self.alerts.push(truncate(text, TEXT_MAX - 1));
    }

    /// Take the texts waiting to go out. The harness sends them off the hot
    /// path, the same way it posts the STATE record.
    pub(crate) fn take_alerts(&mut self) -> Vec<String> {
        std::mem::take(&mut self.alerts)
    }
}

/// Run the owner's alert command with `text` as its last argument.
///
/// Never on the hot path: the caller spawns this. A command that is missing,
/// unparseable, slow or failing is logged and nothing else — an agent must
/// not stop working because a text did not go out.
///
/// `SMS_TO` is set on the child process only. The Orchestrator texts a
/// client with this same script and reads its recipient from its own
/// environment, so setting `SMS_TO` in the harness would change who that
/// reaches.
pub(crate) async fn send(cmd: &str, sms_to: Option<&str>, text: &str) {
    let Some((program, args)) = split_command(cmd) else {
        tracing::warn!("{ALERT_CMD_ENV} names no command — the owner was not texted");
        return;
    };
    let mut command = tokio::process::Command::new(program);
    command.args(args).arg(text);
    if let Some(to) = sms_to {
        command.env(SMS_TO_ENV, to);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::acp::configure_no_window(&mut command);
    let sent = async {
        let mut child = command.spawn()?;
        child.wait().await
    };
    match tokio::time::timeout(SEND_TIMEOUT, sent).await {
        Ok(Ok(status)) if status.success() => tracing::warn!("texted the owner: {text}"),
        Ok(Ok(status)) => {
            tracing::warn!("the alert command exited with {status} — the owner may not be texted")
        }
        Ok(Err(e)) => tracing::warn!("the alert command did not run: {e}"),
        Err(_) => {
            tracing::warn!("the alert command did not finish in {SEND_TIMEOUT:?} — abandoned")
        }
    }
}

/// Split a command line into its program and arguments.
///
/// Quote-aware and nothing more: `"` and `'` group a run of characters, and
/// a backslash is an ordinary character. On Windows the backslash is the
/// path separator, so treating it as an escape would break every path the
/// owner writes. `None` when the line names no program at all.
fn split_command(line: &str) -> Option<(String, Vec<String>)> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    let mut words = words.into_iter();
    let program = words.next()?;
    Some((program, words.collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpError;
    use crate::factory::{classify, Health, Reporting};
    use crate::scope::SessionScope;
    use chrono::{DateTime, TimeZone, Utc};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn scope() -> SessionScope {
        SessionScope::Conversation {
            channel_id: uuid::Uuid::from_u128(7),
        }
    }

    /// 18 Oct 2026, 20:32 UTC — 3:32 PM CT, when Reset Worker's sign-in went.
    fn oct18() -> DateTime<Utc> {
        at(2026, 10, 18, 20, 32)
    }

    fn lost() -> crate::factory::Failure {
        classify(
            &AcpError::agent_error(
                -32000,
                "API Error: OAuth session expired and could not be refreshed",
            ),
            oct18(),
        )
    }

    fn factory() -> Factory {
        let mut f = Factory::new(None, "claude-agent-acp");
        f.reporting = Reporting {
            agent: "Reset Worker".to_string(),
            short: "ade9d0e2".to_string(),
            ..Reporting::default()
        };
        f
    }

    /// The owner's own line: `python "…\send_sms.py"`. The path keeps its
    /// backslashes and its space, and stays one argument.
    #[test]
    fn a_quoted_windows_path_stays_one_argument() {
        let (program, args) =
            split_command(r#"python "C:\Users\PC owner\.buzz\.agents\skills\sms\send_sms.py""#)
                .expect("a program");
        assert_eq!(program, "python");
        assert_eq!(
            args,
            [r"C:\Users\PC owner\.buzz\.agents\skills\sms\send_sms.py"]
        );
        // A bare path, extra spaces, single quotes and an empty argument.
        assert_eq!(
            split_command("  send-sms   --now  ''  'two words' "),
            Some((
                "send-sms".to_string(),
                vec!["--now".to_string(), String::new(), "two words".to_string()]
            ))
        );
        for empty in ["", "   ", "\t"] {
            assert_eq!(split_command(empty), None, "{empty:?}");
        }
    }

    /// One text when the incident opens, one when it closes, and no more —
    /// however many Criticals or 30-minute probes the incident goes through.
    #[test]
    fn one_text_per_incident_each_way() {
        let mut f = factory();
        let lost = lost();
        f.plan(0, &scope(), &lost, oct18());
        let opened = f.take_alerts();
        assert_eq!(opened.len(), 1, "{opened:?}");
        let text = &opened[0];
        assert_eq!(
            text,
            "Buzz: Reset Worker lost its Claude sign-in. Its requests are parked, not dropped. \
             Claude said: \"API Error: OAuth session expired and could not be refreshed\"."
        );
        assert!(f.take_alerts().is_empty(), "taken once");

        // The 30-minute probes fail again, and a spend cap joins in: still
        // one outage, so still one text.
        let spend = classify(
            &AcpError::agent_error(
                -32603,
                "Internal error: You've hit your monthly spend limit · raise it",
            ),
            oct18(),
        );
        f.plan(0, &scope(), &lost, oct18() + chrono::Duration::minutes(30));
        f.plan(0, &scope(), &spend, oct18() + chrono::Duration::minutes(60));
        assert!(f.take_alerts().is_empty(), "one text per incident");

        // The agent answers again: exactly one all-clear.
        for _ in 0..crate::factory::ANSWERS_TO_RECOVER {
            f.on_success(0, oct18() + chrono::Duration::hours(3));
        }
        assert_eq!(f.health, Health::Operational);
        assert_eq!(
            f.take_alerts(),
            ["Buzz: all clear — Reset Worker is working normally again."]
        );
        assert!(f.take_alerts().is_empty(), "one all-clear per incident");

        // A second incident texts again: the owner hears about every outage.
        f.plan(0, &scope(), &lost, at(2026, 10, 19, 9, 0));
        assert_eq!(f.take_alerts().len(), 1);
    }

    /// An agent that was never texted about an outage is never texted an
    /// all-clear either: a usage limit lifts by itself and is no Critical.
    #[test]
    fn no_all_clear_without_an_opening_text() {
        let mut f = factory();
        let weekly = classify(
            &AcpError::agent_error(-32603, "Internal error: You've hit your weekly limit"),
            oct18(),
        );
        f.plan(0, &scope(), &weekly, oct18());
        assert!(f.take_alerts().is_empty(), "no Critical, no text");
        for _ in 0..crate::factory::ANSWERS_TO_RECOVER {
            f.on_success(0, oct18());
        }
        assert!(f.take_alerts().is_empty(), "nothing to be clear about");
    }

    /// The text carries the agent's name, plain words and Claude's own
    /// error, and nothing else: no token, no path, no email, no phone
    /// number, and never more than one message. The STATE record and the
    /// Critical card beside it carry no more than the text does.
    #[test]
    fn alert_text_and_records_never_carry_a_token_or_a_path() {
        let bench = tempfile::tempdir().unwrap();
        let mut f = Factory::new(
            Some(r"token=env;fallback=C:\Users\PC owner\.buzz-agents\factory\claude"),
            "claude-agent-acp",
        );
        f.reporting = Reporting {
            agent: "Reset Worker".to_string(),
            short: "ade9d0e2".to_string(),
            bench: Some(bench.path().to_path_buf()),
            ..Reporting::default()
        };
        let leaky = classify(
            &AcpError::agent_error(
                -32000,
                "API Error: 401 reading C:\\Users\\PC owner\\.claude\\.credentials.json for \
                 russell@example.com with Bearer sk-ant-oat01-abcdefghijklmnopqrstuvwxyz012345 \
                 call 555-867-5309 if this keeps happening and then some more words to push this \
                 message past three hundred characters so the cut is exercised too",
            ),
            oct18(),
        );
        f.launch_env(0, oct18());
        f.plan(0, &scope(), &leaky, oct18());
        let texts = f.take_alerts();
        assert_eq!(texts.len(), 1);
        let text = &texts[0];
        // One message: the provider's words are what a cut takes off.
        assert!(
            text.chars().count() <= TEXT_MAX,
            "{} characters: {text}",
            text.chars().count()
        );
        assert!(
            text.starts_with("Buzz: Reset Worker lost its Claude sign-in."),
            "{text}"
        );
        // It says which login the work moved to, by label and never by path.
        assert!(text.contains("fallback"), "{text}");
        assert!(
            text.contains("[token]") && text.contains("[path]"),
            "{text}"
        );

        // The text, the STATE record and the card all say the same amount.
        let record = f.take_report(oct18()).expect("a STATE record");
        let card = std::fs::read_to_string(
            bench
                .path()
                .join("inbox")
                .join(crate::factory::CARD_WRITER)
                .join("ade9d0e2-sign-in.json"),
        )
        .expect("the Critical card");
        for (what, written) in [
            ("text", text.as_str()),
            ("record", &record),
            ("card", &card),
        ] {
            for secret in [
                "sk-ant",
                "Bearer sk",
                "oat01",
                "Users",
                "PC owner",
                ".credentials",
                ".claude",
                "russell@example.com",
                "555-867-5309",
            ] {
                assert!(
                    !written.contains(secret),
                    "{secret} in the {what}: {written}"
                );
            }
        }
    }

    /// `SMS_TO` reaches the alert command and nothing else. The Orchestrator
    /// texts a client with this same script from its own environment, so the
    /// harness's own `SMS_TO` must come out of this unchanged.
    #[tokio::test]
    async fn the_alert_command_gets_sms_to_in_its_own_env_only() {
        // A folder with a space in it, like the owner's own path.
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("send sms");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("sent.txt");
        let before = std::env::var_os(SMS_TO_ENV);

        // A fake `send_sms` that writes its recipient and its message.
        let sender = write_fake_sender(&dir, &out);
        let cmd = format!("\"{}\"", sender.display());

        send(&cmd, Some("+15551234567"), "Buzz: all clear").await;
        let written = std::fs::read_to_string(&out).expect("the command ran");
        assert!(written.contains("+15551234567"), "{written}");
        assert!(written.contains("Buzz: all clear"), "{written}");

        // The harness's own environment is untouched either way: the
        // Orchestrator's client keeps getting the Orchestrator's texts.
        assert_eq!(std::env::var_os(SMS_TO_ENV), before);
        std::fs::remove_file(&out).unwrap();
        send(&cmd, None, "Buzz: no recipient").await;
        assert_eq!(std::env::var_os(SMS_TO_ENV), before);
        let written = std::fs::read_to_string(&out).expect("the command ran");
        assert!(
            !written.contains("+1555"),
            "no recipient was set: {written}"
        );
    }

    /// Write a script that records `$SMS_TO` and its first argument in `out`.
    fn write_fake_sender(dir: &std::path::Path, out: &std::path::Path) -> std::path::PathBuf {
        #[cfg(windows)]
        {
            let path = dir.join("send_sms.cmd");
            std::fs::write(
                &path,
                format!(
                    "@echo off\r\n> \"{}\" echo [%SMS_TO%] %~1\r\n",
                    out.display()
                ),
            )
            .unwrap();
            path
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("send_sms.sh");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf '[%s] %s' \"$SMS_TO\" \"$1\" > '{}'\n",
                    out.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    /// A send that cannot work is logged and nothing more: no panic, and the
    /// harness is never held up by it.
    #[tokio::test]
    async fn a_failed_send_never_crashes_or_blocks() {
        send("", None, "Buzz: nowhere to go").await;
        send(
            "buzz-no-such-alert-command-exists",
            Some("+15550000000"),
            "Buzz: no such command",
        )
        .await;
    }
}
