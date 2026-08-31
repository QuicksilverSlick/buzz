//! Walk the ticket fold through the situations that actually happen, and print
//! what it decides.
//!
//! ```text
//! cargo run -p buzz-core --example ticket_scenarios
//! ```
//!
//! This exists so the fold's behaviour can be reviewed without reading Rust.
//! Every scenario is a judgement call someone has to agree with.

use buzz_core::ticket::{
    fold, Authority, Confirmation, Outcome, Participation, RowKind, StatusRow,
};

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;

const CRAIG: &str = "craig (asked for it)";
const OWNER: &str = "reset-agent (owns the project)";
const WORKER: &str = "claude-session";

fn authority() -> Authority {
    Authority {
        requester: CRAIG.to_string(),
        owner: Some(OWNER.to_string()),
    }
}

fn part(actor: &str, p: Participation, at: i64, deadline: Option<i64>) -> StatusRow {
    StatusRow {
        actor: actor.to_string(),
        kind: RowKind::Participation(p),
        created_at: at,
        deadline,
        reason: None,
    }
}

fn gave_up(actor: &str, at: i64, why: &str) -> StatusRow {
    StatusRow {
        actor: actor.to_string(),
        kind: RowKind::Participation(Participation::Abandoned),
        created_at: at,
        deadline: None,
        reason: Some(why.to_string()),
    }
}

fn closed(actor: &str, o: Outcome, at: i64, why: Option<&str>) -> StatusRow {
    StatusRow {
        actor: actor.to_string(),
        kind: RowKind::Outcome(o),
        created_at: at,
        deadline: None,
        reason: why.map(str::to_string),
    }
}

fn show(title: &str, story: &str, rows: &[StatusRow], open_deadline: Option<i64>, now: i64) {
    let v = fold(rows, open_deadline, &authority());

    println!("\n\x1b[1m{title}\x1b[0m");
    println!("  {story}");
    println!("  -- decides --");
    println!("     state       {}", v.state.as_tag());
    match v.deadline {
        Some(d) if d > now => println!("     deadline    in {}", human(d - now)),
        Some(d) => println!(
            "     deadline    \x1b[33m{} ago - OVERDUE\x1b[0m",
            human(now - d)
        ),
        None => println!("     deadline    none (nothing is waiting on it)"),
    }
    if let Some(reason) = &v.reason {
        println!("     reason      {reason}");
    }
    if let Some(c) = &v.confirmation {
        let how = match c {
            Confirmation::ByRequester => "the person who asked confirmed it".to_string(),
            Confirmation::ByOwner => "the project owner confirmed it".to_string(),
            Confirmation::Unopposed { delivered_by, .. } => {
                format!("nobody objected after {delivered_by} delivered")
            }
        };
        println!("     closed how  {how}");
    }
    for (who, why) in &v.abandoned {
        println!("     gave up     {who} - {why}");
    }
    if !v.unauthorized_close.is_empty() {
        println!(
            "     \x1b[33mrefused\x1b[0m     {} tried to close it without authority",
            v.unauthorized_close.join(", ")
        );
    }
    if !v.interrupted.is_empty() {
        println!(
            "     interrupted {} was still working when it closed",
            v.interrupted.join(", ")
        );
    }
    println!(
        "     watchdog    {}",
        if v.is_overdue(now) {
            "\x1b[33mwould escalate this now\x1b[0m"
        } else if v.state.is_terminal() {
            "ignores it - finished"
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
    let now = 10 * HOUR;
    println!("\x1b[1mTicket fold - what it decides, and why\x1b[0m");
    println!("Each scenario is a judgement call. Disagree with any of them and say so.");

    show(
        "1. Nobody picked it up",
        "Craig reported a bug 45 seconds ago. No agent has said anything.",
        &[],
        Some(now - 15),
        now,
    );

    show(
        "2. Being worked on, healthy",
        "An agent is working and posting progress. Next update due in 10 minutes.",
        &[part(
            WORKER,
            Participation::Working,
            now - 5 * MIN,
            Some(now + 10 * MIN),
        )],
        None,
        now,
    );

    show(
        "3. Went quiet mid-work",
        "The agent said it was working, then stopped renewing.",
        &[part(
            WORKER,
            Participation::Working,
            now - 27 * MIN,
            Some(now - 12 * MIN),
        )],
        None,
        now,
    );

    show(
        "4. One worker gives up, another is still on it",
        "THE RULE THAT CHANGED. One worker quitting no longer ends the ticket -\n  \
         it records who gave up and why, and the ticket stays live for everyone else.",
        &[
            gave_up(
                WORKER,
                now - 20 * MIN,
                "the branch is protected, I cannot push",
            ),
            part(
                "reviewer",
                Participation::Working,
                now - 5 * MIN,
                Some(now + 20 * MIN),
            ),
        ],
        None,
        now,
    );

    show(
        "5. Everyone gave up",
        "Both workers left. Nobody closed it, so it belongs to no one - and is still watched.",
        &[
            gave_up(WORKER, now - 40 * MIN, "outside what I can do"),
            gave_up("reviewer", now - 30 * MIN, "no capacity this week"),
        ],
        Some(now - 5 * MIN),
        now,
    );

    show(
        "6. A worker tries to close it",
        "The agent marked the whole ticket done. It has no authority to, so the\n  \
         claim is demoted to delivered-awaiting-confirmation and surfaced.",
        &[closed(
            WORKER,
            Outcome::Done(Confirmation::ByRequester),
            now - 10 * MIN,
            None,
        )],
        Some(now + 2 * HOUR),
        now,
    );

    show(
        "7. Delivered, waiting on Craig",
        "The work is finished and waiting to be confirmed. Still swept, so an\n  \
         unconfirmed delivery cannot sit unnoticed forever.",
        &[part(
            WORKER,
            Participation::Delivered,
            now - 30 * MIN,
            Some(now + 90 * MIN),
        )],
        None,
        now,
    );

    show(
        "8. Closed because nobody objected",
        "Craig never replied within the window, so it closed - but visibly as a\n  \
         second-class close that names who delivered.",
        &[closed(
            CRAIG,
            Outcome::Done(Confirmation::Unopposed {
                delivered_by: WORKER.to_string(),
                window_secs: 2 * HOUR,
            }),
            now - MIN,
            None,
        )],
        None,
        now,
    );

    show(
        "9. Craig calls it off while work is in flight",
        "A close is not blocked by live work - but whoever was interrupted is named.",
        &[
            part(
                WORKER,
                Participation::Working,
                now - 10 * MIN,
                Some(now + MIN),
            ),
            closed(
                CRAIG,
                Outcome::Archived,
                now - MIN,
                Some("we shipped it another way"),
            ),
        ],
        None,
        now,
    );

    println!("\n\x1b[1mThe judgement calls worth disagreeing with\x1b[0m");
    println!("  - Only the person who asked, or the project owner, can end a ticket.");
    println!("  - A worker giving up is recorded with its reason and never ends the ticket.");
    println!("  - Finishing is not closing: delivered work waits to be confirmed.");
    println!("  - A ticket everyone walked away from is unowned, not finished.");
    println!("  - Nobody objecting closes it, but that never reads as a person agreeing.\n");
}
