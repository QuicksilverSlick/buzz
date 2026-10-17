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
  type BenchPlainChip,
  type BenchPlainView,
  benchPlainView,
} from "@/features/bench/lib/benchCards";
import { slugWorkbench } from "@/features/bench/lib/workbenches";

export type BenchField = { label: string; text: string };

export type BenchCardBlock =
  | { kind: "text"; text: string }
  /** One field, or the risk line's: Risk, Undo and Reach on one line. */
  | { kind: "fields"; fields: BenchField[] }
  | { kind: "chips"; chips: BenchPlainChip[] };

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
 * the lines under it join it until a blank line; Details fields go apart.
 */
export function benchBodyBlocks(body: string): {
  blocks: BenchCardBlock[];
  details: string[];
} {
  const blocks: BenchCardBlock[] = [];
  const details: BenchField[] = [];
  let open: { text: string } | null = null;
  for (const raw of body.split("\n")) {
    const line = raw.trim();
    if (!line) {
      open = null;
      continue;
    }
    const risk = labelAt(line, RISK_LABELS);
    const field = risk ? null : labelAt(line, FIELD_LABELS);
    if (risk) {
      const fields = riskFields(risk);
      blocks.push({ kind: "fields", fields });
      open = fields[fields.length - 1];
    } else if (field) {
      if (field.label === "Details") details.push(field);
      else blocks.push({ kind: "fields", fields: [field] });
      open = field;
    } else if (open) {
      open.text += `\n${line}`;
    } else {
      const text = { kind: "text" as const, text: line };
      blocks.push(text);
      open = text;
    }
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
      ...(plain.chips.length > 0
        ? [{ kind: "chips" as const, chips: plain.chips }]
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
 * The project (else area) a card's header names: a workbench's name for its
 * slugs ("28-day-reset" is "28 Day Reset"), else the slug as it is.
 */
export function benchCardPlace(
  card: Pick<BenchCard, "area" | "project">,
): string {
  const slug = card.project ?? card.area;
  return slugWorkbench(slug)?.label ?? slug;
}
