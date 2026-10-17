import { Link2 } from "lucide-react";

import { Button } from "@/shared/ui/button";

/** Inline "share with agent" prompt above the composer (owner-approved 16 Oct). */
export function ComposerShareLinkPrompt({
  names,
  onIncludeQuote,
  onSendLinkOnly,
}: {
  names: string[] | null;
  onIncludeQuote: () => void;
  onSendLinkOnly: () => void;
}) {
  if (!names) return null;
  return (
    <div
      className="relative z-0 -mb-4 flex flex-wrap items-center gap-2 rounded-t-2xl border border-b-0 border-border/60 bg-muted/55 px-4 pb-6 pt-2.5 text-sm leading-5 text-muted-foreground backdrop-blur-sm"
      data-testid="share-link-prompt"
    >
      <Link2 aria-hidden className="h-4 w-4 shrink-0" />
      <p className="min-w-0 flex-1 text-foreground" role="alert">
        {names.join(", ")} can't open that link. Include the message text as a
        quote?
      </p>
      <Button onClick={onIncludeQuote} size="xs" type="button">
        Include quote
      </Button>
      <Button
        onClick={onSendLinkOnly}
        size="xs"
        type="button"
        variant="outline"
      >
        Send link only
      </Button>
    </div>
  );
}
