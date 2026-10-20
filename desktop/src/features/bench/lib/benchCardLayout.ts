/**
 * One decision-card layout for the Bench screen, a card's message and the
 * Inbox, in the order the phone's card text has it (bench/plain.rs): the
 * body's paragraphs, where a label a writer typed at the start of a line
 * starts a field, then the plain fields and the recommendation; the details
 * apart. Labels are ours to show bold; every value stays the writer's plain
 * text.
 */

import {
  type BenchCard,
  type BenchPlainView,
  benchKindLabel,
  benchPlainView,
} from "@/features/bench/lib/benchCards";
import { slugWorkbench } from "@/features/bench/lib/workbenches";

export type BenchField = { label: string; text: string };

export type BenchCardBlock =
  | { kind: "text"; text: string }
  /** One field, or the risk line's: Risk, Undo and Reach on one line. */
  | { kind: "fields"; fields: BenchField[] };

export type BenchCardLayout = {
  blocks: BenchCardBlock[];
  details: string | null;
  action: BenchPlainView["action"];
};

/** plain.rs FIELDS and RISK: only these, and only at a line's start. */
const FIELD_LABELS = [
  "What changes",
  "Asked by",
  "If you wait",
  "The choice",
  "Recommended",
  "Details",
] as const;
const RISK_LABELS = ["Risk", "Undo", "Reach"] as const;

function labelAt(line: string, labels: readonly string[]): BenchField | null {
  const label = labels.find((known) => line.startsWith(`${known}:`));
  return label ? { label, text: line.slice(label.length + 1).trim() } : null;
}

/** "Risk: Low; Undo: Easy": a risk label after "; " starts the next field. */
function riskFields(field: BenchField): BenchField[] {
  for (
    let at = field.text.indexOf("; ");
    at !== -1;
    at = field.text.indexOf("; ", at + 2)
  ) {
    const next = labelAt(field.text.slice(at + 2), RISK_LABELS);
    if (next) {
      return [
        { label: field.label, text: field.text.slice(0, at).trim() },
        ...riskFields(next),
      ];
    }
  }
  return [field];
}

/**
 * The body's paragraphs. A line opening with a label starts a field, and
 * the lines under it join it until a blank line. Details go apart, and keep
 * every line and paragraph after them until another label opens a line, so
 * nothing of theirs lands among the decision fields.
 */
export function benchBodyBlocks(body: string): {
  blocks: BenchCardBlock[];
  details: string[];
} {
  const blocks: BenchCardBlock[] = [];
  const details: BenchField[] = [];
  let open: { text: string } | null = null;
  let detail: BenchField | null = null;
  let blank = false;
  for (const raw of body.split("\n")) {
    const line = raw.trim();
    if (!line) {
      if (open !== detail) open = null;
      blank = true;
      continue;
    }
    const risk = labelAt(line, RISK_LABELS);
    const field = risk ? null : labelAt(line, FIELD_LABELS);
    if (risk) {
      const fields = riskFields(risk);
      blocks.push({ kind: "fields", fields });
      open = fields[fields.length - 1];
      detail = null;
    } else if (field) {
      if (field.label === "Details") details.push(field);
      else blocks.push({ kind: "fields", fields: [field] });
      open = field;
      detail = field.label === "Details" ? field : null;
    } else if (open) {
      open.text += open.text ? `${blank ? "\n\n" : "\n"}${line}` : line;
    } else {
      const text = { kind: "text" as const, text: line };
      blocks.push(text);
      open = text;
    }
    blank = false;
  }
  return { blocks, details: details.map((d) => d.text) };
}

function fieldBlock(
  label: string,
  text: string | null | undefined,
): BenchCardBlock[] {
  return text ? [{ kind: "fields", fields: [{ label, text }] }] : [];
}

/** The card's blocks top to bottom, its details, and its action. */
export function benchCardLayout(card: BenchCard): BenchCardLayout {
  const body = benchBodyBlocks(card.body);
  const plain = benchPlainView(card);
  const line = (label: string) =>
    fieldBlock(label, plain.lines.find((l) => l.label === label)?.text);
  const details = [...body.details, plain.details].filter(Boolean);
  return {
    blocks: [
      ...body.blocks,
      ...line("What changes"),
      ...line("Asked by"),
      ...(plain.risk.length > 0
        ? [{ kind: "fields" as const, fields: plain.risk }]
        : []),
      ...line("If you wait"),
      ...fieldBlock("Recommended", plain.recommended),
    ],
    details: details.length > 0 ? details.join("\n\n") : null,
    action: plain.action,
  };
}

/**
 * Where a post's text leaves the request's paragraphs (item.rs card_head):
 * its options, due line, links, `as of`, a receipt's answer, For or Closed.
 * prose() puts a U+FEFF after every colon and before a list mark in agent
 * text, so none of these can open a line of a paragraph.
 */
const POST_TAIL =
  /^(?:\d+\. |due \d{4}-\d\d-\d\dT\d\d:\d|as of \d{1,2}:\d\d$|Answered: |For: |Closed: )|: (?:https|buzz):\/\//;
/** render_card's lines on how to answer, which a read-only card never says. */
const POST_HOW_TO =
  / ?Tap one number below to answer\.$|^Or reply in this thread\.$|^as of \d{1,2}:\d\d$/;

/** A line of a post as plain text: our `**` marks and prose()'s U+FEFFs gone. */
function postLine(line: string): string {
  return line
    .replace(/^\*\*([^*]+)\*\*$/, "$1")
    .replace(/^\*\*([^*]+):\*\*/, "$1:")
    .replace(/ · \*\*(Risk|Undo|Reach):\*\*/g, "; $1:")
    .replaceAll("\uFEFF", "");
}

export type BenchPost = {
  /** The card as its post's own text has it; only identity is the item's. */
  card: BenchCard;
  /** The header after its marker: place and writer, as posted. */
  meta: string;
  /** Its other lines, as posted: For, Recorded, a receipt's answer, Closed. */
  notes: string[];
  /** A receipt's Outcome (item.rs render_receipt), laid out as its own. */
  outcome: BenchPost | null;
};

/**
 * A card from its post's own text (item.rs render_card; render_closed on one
 * a newer card replaced): the wording the owner read and replied to, never
 * the live item's. Its paragraphs lay out as a body does.
 */
export function benchPostCard(item: BenchCard, text: string): BenchPost {
  const lines = text.split("\n");
  if (lines[0] === "Back to you") lines.shift();
  const meta = (lines.shift() ?? "").split(" · ").slice(1).join(" · ");
  const title = /^\*\*([^*]+)\*\*$/.exec(lines[0] ?? "")?.[1];
  if (title !== undefined) lines.shift();
  const end = lines.findIndex((line) => POST_TAIL.test(line));
  const tail = end === -1 ? [] : lines.splice(end);
  // A receipt's Outcome follows its answer: a header, title, body and links.
  const answered = tail.findIndex((line) => line.startsWith("Answered: "));
  const at = tail.findIndex(
    (line, index) =>
      answered !== -1 && index > answered && line.startsWith("Outcome · "),
  );
  const outcome =
    at === -1 ? null : benchPostCard(item, tail.splice(at).join("\n"));
  const options: string[] = [];
  const links: BenchCard["links"] = [];
  const notes: string[] = [];
  let deadline: string | null = null;
  for (const line of tail) {
    const option = /^\d+\. (.+)$/.exec(line);
    const link = /^(.+): ((?:https|buzz):\/\/\S+)$/.exec(line);
    if (option) options.push(option[1]);
    else if (/^due \d{4}-\d\d-\d\dT\S+$/.test(line)) deadline = line.slice(4);
    else if (link) {
      // Its own thread line (item.rs thread_line): its row opens its thread.
      if (link[1] !== "Open thread")
        links.push({ label: link[1], url: link[2] });
    } else {
      const note = postLine(line.replace(POST_HOW_TO, ""));
      if (note) notes.push(note);
    }
  }
  return {
    card: {
      ...item,
      title: title ?? "",
      body: lines.map(postLine).join("\n").trim(),
      options,
      deadline,
      links,
      // The live item's answer is not this post's.
      answer: null,
      recorded: null,
      decidedBy: null,
      recommend: null,
      plain: null,
      action: null,
      request: null,
    },
    meta,
    notes,
    outcome,
  };
}

/** The Inbox row's second line: the card's first field, else its first paragraph. */
export function benchCardLead(card: BenchCard): BenchCardBlock | null {
  const { blocks } = benchCardLayout(card);
  return (
    blocks.find((block) => block.kind === "fields") ??
    blocks.find((block) => block.kind === "text") ??
    null
  );
}

/**
 * Where a card's header says it is, as bench/plain.rs `place` prints it:
 * its project (else area) by its workbench's name ("28-day-reset" is "28 Day
 * Reset"), else the slug; then the area when it says more ("reset-billing"),
 * never a slug of the workbench already named.
 */
export function benchCardPlace(
  card: Pick<BenchCard, "area" | "project">,
): string[] {
  const slug = card.project ?? card.area;
  const workbench = slugWorkbench(slug);
  const name = workbench?.label ?? slug;
  return card.area === name || workbench?.slugs.includes(card.area)
    ? [name]
    : [name, card.area];
}

/**
 * The header's muted line: the kind, then where the card is, once by name
 * (never "28 Day Reset" beside "28-day-reset"), then who it is from.
 */
export function benchCardMeta(card: BenchCard, projectLabel?: string): string {
  return [
    ...new Set([
      benchKindLabel(card.kind),
      projectLabel,
      ...benchCardPlace(card),
      card.writer,
    ]),
  ]
    .filter(Boolean)
    .join(" · ");
}
