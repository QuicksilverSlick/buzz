import { Link2 } from "lucide-react";
import type * as React from "react";

import { Button } from "@/shared/ui/button";

const NAMED_LIMIT = 3;

/** "Ana, Bo, Cy and 4 others": a long roster never fills the composer. */
export function formatShareNames(names: readonly string[]): string {
  const rest = names.length - NAMED_LIMIT;
  if (rest <= 0) return names.join(", ");
  return `${names.slice(0, NAMED_LIMIT).join(", ")} and ${rest} ${rest === 1 ? "other" : "others"}`;
}

/**
 * Inline "share with agent" prompt above the composer (owner-approved 16
 * Oct). It shows the quote it would send, so nothing goes out unseen;
 * Escape dismisses it and the send stays unsent.
 */
export function ComposerShareLinkPrompt({
  onDismiss,
  onIncludeQuote,
  onSendLinkOnly,
  prompt,
}: {
  onDismiss: () => void;
  onIncludeQuote: () => void;
  onSendLinkOnly: () => void;
  prompt: { names: string[]; openChannel: string | null; quote: string } | null;
}) {
  if (!prompt) return null;
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    onDismiss();
  };
  return (
    <div
      className="relative z-0 -mb-4 flex flex-wrap items-center gap-2 rounded-t-2xl border border-b-0 border-border/60 bg-muted/55 px-4 pb-6 pt-2.5 text-sm leading-5 text-muted-foreground backdrop-blur-sm"
      data-testid="share-link-prompt"
    >
      <Link2 aria-hidden className="h-4 w-4 shrink-0" />
      <p className="min-w-0 flex-1 text-foreground" role="alert">
        {formatShareNames(prompt.names)} can't open that link. Include the
        message text as a quote?
        {prompt.openChannel
          ? ` Everyone who can read #${prompt.openChannel} will see it.`
          : null}
      </p>
      <Button
        onClick={onIncludeQuote}
        onKeyDown={onKeyDown}
        size="xs"
        type="button"
      >
        Include quote
      </Button>
      <Button
        onClick={onSendLinkOnly}
        onKeyDown={onKeyDown}
        size="xs"
        type="button"
        variant="outline"
      >
        Send link only
      </Button>
      <blockquote
        className="max-h-24 w-full overflow-y-auto whitespace-pre-wrap break-words border-l-2 border-border pl-2 text-xs"
        data-testid="share-link-quote"
      >
        {prompt.quote.replace(/^> ?/gm, "")}
      </blockquote>
    </div>
  );
}
