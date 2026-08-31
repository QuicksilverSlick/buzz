//! Walk the ticket fold through the situations that actually happen, and print
//! what it decides.
//!
//! Run with:
//!
//! ```text
//! cargo run -p buzz-core --example ticket_scenarios
//! ```
//!
//! This exists so the fold's behaviour can be reviewed without reading Rust.
//! Every scenario below is a judgement call someone has to agree with — the
//! code cannot tell you whether "one person gives up while another is still
//! working" should close the ticket. That is a decision, and it should be made
//! deliberately rather than discovered in production.

use buzz_core::ticket::{fold, StatusRow, TicketState};

/// Seconds, for readability in the scenarios below.
const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;

fn row(actor: &str, state: TicketState, at: i64, deadline: Option<i64>) -> StatusRow {
    StatusRow {
        actor: actor.to_string(),
        state,
        created_at: at,
        not_before: deadline,
        reason: None,
    }
}

fn failed(actor: &str, at: i64, why: &str) -> StatusRow {
    StatusRow {
        actor: actor.to_string(),
        state: TicketState::Failed,
        created_at: at,
        not_before: None,
        reason: Some(why.to_string()),
    }
}

/// Print one scenario: what happened, what the fold decided, and whether the
/// watchdog would act on it right now.
fn show(title: &str, story: &str, rows: &[StatusRow], open_deadline: Option<i64>, now: i64) {
    let view = fold(rows, open_deadline);

    println!("\n\x1b[1m{title}\x1b[0m");
    println!("  {story}");
    println!("  ── decides ──");
    println!("     state      {}", view.state.as_tag());
    match view.deadline {
        Some(d) if d > now => println!("     deadline   in {}", human(d - now)),
        Some(d) => println!(
            "     deadline   \x1b[33m{} ago — OVERDUE\x1b[0m",
            human(now - d)
        ),
        None => println!("     deadline   none (nothing is waiting on it)"),
    }
    if let Some(by) = &view.decided_by {
        println!("     decided by {by}");
    }
    if let Some(reason) = &view.reason {
        println!("     reason     {reason}");
    }
    println!(
        "     watchdog   {}",
        if view.is_overdue(now) {
            "\x1b[33mwould escalate this now\x1b[0m"
        } else if view.state.is_terminal() {
            "ignores it — finished"
        } else {
            "waiting, not yet due"
        }
    );
}

fn human(secs: i64) -> String {
    match secs {
        s if s < MIN => format!("{s}s"),
        s if s < HOUR => format!("{}m", s / MIN),
        s => format!("{}h", s / HOUR),
    }
}

fn main() {
    // A fixed "now" so output is stable and reviewable.
    let now = 10 * HOUR;
    println!("\x1b[1mTicket fold — what it decides, and why\x1b[0m");
    println!("Each scenario is a judgement call. Disagree with any of them and say so.");

    show(
        "1. Nobody picked it up",
        "Craig reported a bug 45 seconds ago. No agent has said anything.",
        &[],
        Some(now - 15), // ack deadline was 30s after the report, 15s ago
        now,
    );

    show(
        "2. Being worked on, healthy",
        "An agent acknowledged it and is posting progress. Next update due in 10 minutes.",
        &[row(
            "reset-agent",
            TicketState::Progress,
            now - 5 * MIN,
            Some(now + 10 * MIN),
        )],
        None,
        now,
    );

    show(
        "3. Went quiet mid-work",
        "An agent said it was working, then stopped renewing. Its progress deadline passed 12 minutes ago.",
        &[row(
            "reset-agent",
            TicketState::Progress,
            now - 27 * MIN,
            Some(now - 12 * MIN),
        )],
        None,
        now,
    );

    show(
        "4. Two workers, one is stalled",
        "A slow job is fine for hours, but a second actor's check was due 2 minutes ago.\n  \
         The fold takes the EARLIEST deadline so the stall is not masked.",
        &[
            row(
                "long-build",
                TicketState::Progress,
                now - HOUR,
                Some(now + 3 * HOUR),
            ),
            row(
                "reviewer",
                TicketState::Acknowledged,
                now - 5 * MIN,
                Some(now - 2 * MIN),
            ),
        ],
        None,
        now,
    );

    show(
        "5. Finished, while someone else was still mid-work",
        "The reviewer marked it done. A stale Progress row from the agent still exists.\n  \
         Terminal wins, so the ticket does not stay alive forever on a stale row.",
        &[
            row(
                "reset-agent",
                TicketState::Progress,
                now - 20 * MIN,
                Some(now + MIN),
            ),
            row("reviewer", TicketState::Done, now - MIN, None),
        ],
        None,
        now,
    );

    show(
        "6. Gave up, with a reason",
        "The agent could not do it and said why. The requester can read this.",
        &[failed(
            "reset-agent",
            now - 3 * MIN,
            "the repository rejected the push: branch is protected",
        )],
        None,
        now,
    );

    show(
        "7. Escalated and then ignored",
        "The watchdog raised it to a human 6.5 hours ago and nobody answered.\n  \
         Escalation is NOT terminal, so this stays overdue and escalates again.",
        &[row(
            "watchdog",
            TicketState::Escalated,
            now - 7 * HOUR,
            Some(now - 30 * MIN),
        )],
        None,
        now,
    );

    show(
        "8. Out-of-order delivery",
        "An older Acknowledged arrived after a newer Progress from the same actor.\n  \
         Only the latest row per actor counts, so the stale one cannot resurrect an old deadline.",
        &[
            row(
                "reset-agent",
                TicketState::Progress,
                now - MIN,
                Some(now + 15 * MIN),
            ),
            row(
                "reset-agent",
                TicketState::Acknowledged,
                now - 20 * MIN,
                Some(now - 10 * MIN),
            ),
        ],
        None,
        now,
    );

    println!("\n\x1b[1mThe judgement calls worth disagreeing with\x1b[0m");
    println!("  · Scenario 5: one actor finishing ends the ticket, even if another was mid-work.");
    println!(
        "  · Scenario 4: the earliest deadline wins, so a slow job cannot hide a stalled one."
    );
    println!("  · Scenario 7: escalating is not resolving — an ignored escalation keeps firing.");
    println!("  · Scenario 1: a ticket nobody accepted is still overdue, so silence is caught.\n");
}
