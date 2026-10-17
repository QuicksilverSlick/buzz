import * as React from "react";

import type { BenchCard } from "@/features/bench/lib/benchCards";
import {
  type BenchCardBlock,
  benchCardLead,
} from "@/features/bench/lib/benchCardLayout";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

/**
 * A card's blocks with a gap between each: a field's label bold with its
 * colon, the value as it was written. Agent text renders as plain React
 * text, never markdown or HTML; only the labels are ours.
 */
export function BenchCardBlocks({
  blocks,
  className,
  clamp = false,
}: {
  blocks: BenchCardBlock[];
  className?: string;
  /** The Inbox preview: each block at most 3 lines. */
  clamp?: boolean;
}) {
  if (blocks.length === 0) return null;
  return (
    <div className={cn("space-y-2", className)}>
      {blocks.map((block, index) => (
        <BenchCardBlockView
          block={block}
          className={clamp ? "line-clamp-3" : undefined}
          // biome-ignore lint/suspicious/noArrayIndexKey: blocks are positional; two paragraphs may say the same
          key={index}
        />
      ))}
    </div>
  );
}

function BenchCardBlockView({
  block,
  className,
}: {
  block: BenchCardBlock;
  className?: string;
}) {
  if (block.kind === "chips") {
    return (
      <div className="flex flex-wrap gap-1.5">
        {block.chips.map((chip) => (
          <Badge
            className="max-w-full [overflow-wrap:anywhere]"
            key={chip.label}
            variant={chip.variant}
          >
            {chip.label}
          </Badge>
        ))}
      </div>
    );
  }
  return (
    <p
      className={cn("whitespace-pre-wrap [overflow-wrap:anywhere]", className)}
    >
      {block.kind === "text"
        ? block.text
        : block.fields.map((field, index) => (
            <React.Fragment key={field.label}>
              {index > 0 ? " · " : null}
              <span className="font-semibold">{field.label}:</span>
              {field.text ? ` ${field.text}` : null}
            </React.Fragment>
          ))}
    </p>
  );
}

/** Long enough to fold: about four lines of the card's width. */
const DETAILS_FOLD_CHARS = 280;
const DETAILS_FOLD_LINES = 4;

export function benchDetailsFolds(text: string): boolean {
  return (
    text.length > DETAILS_FOLD_CHARS ||
    text.split("\n").length > DETAILS_FOLD_LINES
  );
}

/**
 * Details apart from the decision: a rule, the bold label, the text smaller
 * and muted. Long details fold to 4 lines behind Show more; short ones never
 * fold, so nothing is hidden without a way to show it. `toggle={false}` (the
 * read-only Inbox preview) always clamps and has no button.
 */
export function BenchCardDetails({
  text,
  toggle = true,
}: {
  text: string;
  toggle?: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const id = React.useId();
  const folds = toggle && benchDetailsFolds(text);
  return (
    <div
      className="mt-2.5 border-t border-border/70 pt-2"
      data-testid="bench-card-details"
    >
      <p className="text-xs font-semibold text-foreground">Details</p>
      <p
        className={cn(
          "mt-0.5 whitespace-pre-wrap text-xs text-muted-foreground [overflow-wrap:anywhere]",
          (!toggle || (folds && !open)) && "line-clamp-4",
        )}
        id={id}
      >
        {text}
      </p>
      {folds ? (
        <button
          aria-controls={id}
          aria-expanded={open}
          className="mt-1 rounded-sm text-xs font-medium text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => setOpen((shown) => !shown)}
          type="button"
        >
          {open ? "Show less" : "Show more"}
        </button>
      ) : null}
    </div>
  );
}

/**
 * The Inbox row for a card's own post: its title, then its first field (else
 * its first paragraph), never the header line its relay text opens with.
 */
export function BenchCardSnippet({ card }: { card: BenchCard }) {
  const lead = benchCardLead(card);
  return (
    <div data-testid="inbox-bench-snippet">
      <p className="line-clamp-2 font-semibold [overflow-wrap:anywhere]">
        {card.title}
      </p>
      {lead ? (
        <BenchCardBlockView
          block={lead}
          className="mt-0.5 line-clamp-2 font-normal"
        />
      ) : null}
    </div>
  );
}
