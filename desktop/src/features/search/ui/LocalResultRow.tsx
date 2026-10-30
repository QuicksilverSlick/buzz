import { useViewerAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { benchCardMeta } from "@/features/bench/lib/benchCardLayout";
import {
  type BenchCard,
  benchSeverityChip,
  benchStateLabel,
} from "@/features/bench/lib/benchCards";
import { benchCardTier } from "@/features/bench/lib/benchFind";
import {
  WORKBENCHES,
  cardWorkbench,
  workbenchColor,
  workbenchLabel,
} from "@/features/bench/lib/workbenches";
import { INBOX_TYPE_META } from "@/features/home/ui/InboxItemPreview";
import { useProjectsQuery } from "@/features/projects/hooks";
import { HighlightedSearchText } from "@/features/search/ui/HighlightedSearchText";
import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

/**
 * A Bench card in global search, in the Inbox's colors (Needs you amber,
 * Bench card violet), with its tier, severity, state and project. Spans
 * only: the row is a button.
 */
export function BenchResultRow({
  card,
  query,
}: {
  card: BenchCard;
  query: string;
}) {
  const viewer = useIdentityQuery().data?.pubkey;
  const projects = useProjectsQuery(false).data ?? [];
  const viewerAgents = useViewerAgentPubkeys();
  const needsYou = card.kind === "pending" && !card.answer;
  const meta = INBOX_TYPE_META[needsYou ? "needs-you" : "bench"];
  const severity = benchSeverityChip(card.severity);
  const state = card.kind === "status" ? benchStateLabel(card.state) : null;
  const where = cardWorkbench(card);
  const workbench = WORKBENCHES.find((w) => w.id === where);
  const dot = workbench
    ? workbenchColor(workbench, projects, viewer, viewerAgents).dot
    : null;

  return (
    <>
      <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-background/70">
        <meta.icon aria-hidden className={cn("h-4 w-4", meta.text)} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold leading-5">
          <HighlightedSearchText query={query} text={card.title} />
        </span>
        <span className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-xs">
          <span className={cn("font-medium", meta.text)}>
            {needsYou ? "Needs you on the Bench" : "Bench card"}
          </span>
          {benchCardTier(card) ? (
            <Badge variant="destructive">Red tier</Badge>
          ) : null}
          {severity ? (
            <Badge variant={severity.variant}>{severity.label}</Badge>
          ) : null}
          {state ? <Badge variant="secondary">{state}</Badge> : null}
          {dot ? (
            <span aria-hidden className={cn("size-2 rounded-full", dot)} />
          ) : null}
          <span className="min-w-0 truncate text-muted-foreground">
            {benchCardMeta(card, workbenchLabel(where))}
          </span>
        </span>
      </span>
    </>
  );
}
