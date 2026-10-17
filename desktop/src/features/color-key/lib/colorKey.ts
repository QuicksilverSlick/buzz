import type {
  BenchChipVariant,
  BenchPlainChip,
} from "@/features/bench/lib/benchCards";
import type { InboxItemType } from "@/features/home/lib/inboxClass";
import { INBOX_TYPE_META } from "@/features/home/ui/InboxItemPreview";
import { projectColorOf } from "@/features/projects/lib/projectColors";
import { listSidebarProjects } from "@/features/sidebar/ui/listSidebarProjects";
import type { ManagedAgent } from "@/shared/api/types";

/** What puts a row in each type: classifyInboxItem's rules, first match wins. */
const TYPE_MEANINGS: Record<InboxItemType, string> = {
  "needs-you":
    "An approval or reminder addressed to you, or a Bench question still waiting for an answer.",
  bench: "A Bench card, or its thread, that is not waiting on you.",
  code: "Activity on a Buzz Git repository.",
  dm: "A direct message.",
  mention: "A message that @mentions you.",
  thread: "Any other channel or thread activity.",
};

/** Every Inbox type in INBOX_TYPE_META's order, with what it means. */
export const COLOR_KEY_TYPES = (
  Object.keys(INBOX_TYPE_META) as InboxItemType[]
).map((type) => ({
  type,
  stripe: INBOX_TYPE_META[type].stripe,
  meaning: TYPE_MEANINGS[type],
}));

/** The sidebar's Projects list, in its order, each with its color. */
export function colorKeyProjects(
  input: Parameters<typeof listSidebarProjects>[0],
) {
  return listSidebarProjects(input).map((project) => ({
    address: project.projectAddress,
    name: project.name,
    color: projectColorOf(project),
  }));
}

export const NO_AGENT_DESCRIPTION = "No description yet";

/**
 * Only an agent's public face: name, avatar and its profile About. Prompts,
 * env, commands and args never leave the managed-agent record.
 */
export function colorKeyAgents(
  agents: readonly ManagedAgent[],
  aboutOf: (pubkey: string) => string | null | undefined,
) {
  return agents.map((agent) => ({
    pubkey: agent.pubkey,
    name: agent.name,
    avatarUrl: agent.avatarUrl,
    job: aboutOf(agent.pubkey)?.trim() || NO_AGENT_DESCRIPTION,
  }));
}

/**
 * The labels a Bench card really shows (benchCards.ts, BenchCard.tsx), each
 * with its chip look and one line on what it means.
 */
export const BENCH_KEY_LABELS: ReadonlyArray<{
  chips: ReadonlyArray<{
    label: string;
    /** The Badge it shows as; null for plain text on the card. */
    variant: BenchChipVariant | BenchPlainChip["variant"] | null;
  }>;
  meaning: string;
}> = [
  {
    chips: [{ label: "Your decision", variant: "destructive" }],
    meaning: "Top priority: an agent escalated this to you.",
  },
  {
    chips: [{ label: "Attention", variant: "warning" }],
    meaning: "Needs a look soon.",
  },
  {
    chips: [
      { label: "Info", variant: "info" },
      { label: "Good", variant: "success" },
    ],
    meaning: "Less urgent: worth knowing, or good news.",
  },
  {
    chips: [{ label: "Pending", variant: null }],
    meaning: "A question with options, waiting for an answer.",
  },
  {
    chips: [{ label: "Decision", variant: null }],
    meaning: "A call an agent already made, kept on record.",
  },
  {
    chips: [{ label: "Status", variant: null }],
    meaning:
      "Where a piece of work stands, with a state such as Done or Blocked.",
  },
  {
    chips: [{ label: "Back to you", variant: "info" }],
    meaning:
      "Asked again after an earlier answer; the Earlier line shows that answer.",
  },
  {
    chips: [{ label: "For … · Decider", variant: "outline" }],
    meaning: "Who may answer. The owner can still answer, as owner.",
  },
  {
    chips: [
      { label: "May be stale", variant: "outline" },
      { label: "Superseded", variant: "outline" },
    ],
    meaning: "The card may be out of date, or a newer card replaced it.",
  },
  {
    chips: [
      { label: "High risk", variant: "destructive" },
      { label: "Medium risk", variant: "warning" },
      { label: "Low risk", variant: "secondary" },
    ],
    meaning: "How much could go wrong.",
  },
  {
    chips: [
      { label: "Easy to undo", variant: "outline" },
      { label: "Hard to undo", variant: "outline" },
      { label: "Can't be undone", variant: "outline" },
    ],
    meaning: "How hard the change is to reverse.",
  },
  {
    chips: [{ label: "Reach: …", variant: "outline" }],
    meaning: "Who or what the change affects.",
  },
  {
    chips: [{ label: "Recommended", variant: "secondary" }],
    meaning:
      "The option the agent suggests; the Recommended line says how sure it is and why.",
  },
  {
    chips: [{ label: "Recorded: …", variant: null }],
    meaning: "The answer the Bench saved. Saving... shows while it saves.",
  },
  {
    chips: [
      { label: "Answered: …", variant: null },
      { label: "Outcome · …", variant: null },
    ],
    meaning: "A closed card: what was answered, by whom, and how it ended.",
  },
];
