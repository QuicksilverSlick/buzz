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
