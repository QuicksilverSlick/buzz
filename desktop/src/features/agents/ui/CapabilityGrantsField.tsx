import type { AgentCapability } from "@/shared/api/types";
import { Checkbox } from "@/shared/ui/checkbox";
import { PERSONA_LABEL_OPTIONAL_CLASS } from "./agentConfigOptions";

/**
 * Owner-granted capabilities for an agent.
 *
 * Deny by default: nothing is granted until the owner ticks it, and an
 * untouched agent carries an empty grant list.
 *
 * The copy below deliberately says what a grant *authorises* rather than what
 * it prevents. An agent already runs with the owner's filesystem access and an
 * unrestricted shell, so an unticked box does not mean "cannot" — it means the
 * owner has not authorised it. Wording these as guarantees would be the more
 * reassuring choice and the wrong one.
 */

type Grant = {
  value: AgentCapability;
  label: string;
  detail: string;
};

/**
 * Ordered from narrowest reach to broadest, so the list reads as an escalation
 * and the heaviest grant is not the first thing a hurried owner ticks.
 */
const GRANTS: Grant[] = [
  {
    value: "cross-session-note",
    label: "Leave notes for other sessions",
    detail:
      "Adds a message another Claude Code session sees next time it runs. Nothing starts on its own.",
  },
  {
    value: "cross-session-read",
    label: "Read other sessions",
    detail:
      "Reads transcripts from your other projects, including anything discussed in them.",
  },
  {
    value: "cross-session-activate",
    label: "Start work in other sessions",
    detail:
      "Wakes another session and runs a request in it. That session cannot tell the request from one you typed.",
  },
  {
    value: "desktop-control",
    label: "Control the Claude desktop app",
    detail: "Operates the Claude app on this machine.",
  },
  {
    value: "computer-control",
    label: "Control this computer",
    detail:
      "Takes screenshots and moves the mouse and keyboard. Not limited to Claude.",
  },
];

export function CapabilityGrantsField({
  value,
  onChange,
  disabled,
}: {
  value: AgentCapability[];
  onChange: (next: AgentCapability[]) => void;
  disabled?: boolean;
}) {
  const granted = new Set(value);

  const toggle = (capability: AgentCapability, checked: boolean) => {
    const next = new Set(granted);
    if (checked) {
      next.add(capability);
    } else {
      next.delete(capability);
    }
    // Emit in the order declared above rather than insertion order, so the
    // submitted list does not depend on which box was ticked first.
    onChange(GRANTS.map((g) => g.value).filter((v) => next.has(v)));
  };

  return (
    <fieldset className="space-y-2" disabled={disabled}>
      <legend className="text-sm font-medium text-foreground">
        What this agent may reach
        <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
      </legend>
      <p className="text-sm text-muted-foreground">
        Nothing is granted unless you tick it. Each grant applies to everyone
        this agent answers, not only to you.
      </p>
      <div className="space-y-2.5 pt-1">
        {GRANTS.map((grant) => {
          const id = `agent-capability-${grant.value}`;
          return (
            <div className="flex items-start gap-2.5" key={grant.value}>
              <Checkbox
                checked={granted.has(grant.value)}
                className="mt-0.5"
                data-testid={id}
                id={id}
                onCheckedChange={(checked) =>
                  toggle(grant.value, checked === true)
                }
              />
              <label className="min-w-0 flex-1 cursor-pointer" htmlFor={id}>
                <span className="block text-sm text-foreground">
                  {grant.label}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {grant.detail}
                </span>
              </label>
            </div>
          );
        })}
      </div>
    </fieldset>
  );
}
