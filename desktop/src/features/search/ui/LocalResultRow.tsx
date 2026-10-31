import { Activity, Bot, Zap } from "lucide-react";
import type { ComponentType } from "react";

import { useViewerAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { benchCardMeta } from "@/features/bench/lib/benchCardLayout";
import {
  type BenchCard,
  benchSeverityChip,
  benchStateLabel,
} from "@/features/bench/lib/benchCards";
import { benchCardTier, benchNeedsYou } from "@/features/bench/lib/benchFind";
import {
  WORKBENCHES,
  cardWorkbench,
  workbenchColor,
  workbenchLabel,
} from "@/features/bench/lib/workbenches";
import type { InboxItemType } from "@/features/home/lib/inboxClass";
import { INBOX_TYPE_META } from "@/features/home/ui/InboxItemPreview";
import { useProjectsQuery } from "@/features/projects/hooks";
import { PROJECT_EVENT_VISUALS } from "@/features/projects/ui/ProjectEventTypeIcon";
import type { SurfaceHit } from "@/features/search/lib/surfaceSearch";
import { HighlightedSearchText } from "@/features/search/ui/HighlightedSearchText";
import {
  TRIGGER_ACCENTS,
  TRIGGER_ICONS,
} from "@/features/workflows/ui/WorkflowCard";
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
  const needsYou = benchNeedsYou(card);
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

type Visual = {
  icon: ComponentType<{ className?: string }>;
  label: string;
  text?: string;
  tile?: string;
};

/** Each screen's own icon, colour and word for what a hit is. */
function surfaceVisual({ kind, surface }: SurfaceHit): Visual {
  if (surface === "inbox") return INBOX_TYPE_META[kind as InboxItemType];
  if (surface === "projects") {
    const pr = kind === "pull-request";
    const { icon, iconClassName } =
      PROJECT_EVENT_VISUALS[pr ? kind : "repository"];
    const label = pr
      ? "Pull request"
      : kind === "project"
        ? "Project"
        : "Repository";
    return { icon, label, text: iconClassName };
  }
  if (surface === "workflows") {
    const tile = TRIGGER_ACCENTS[kind];
    return { icon: TRIGGER_ICONS[kind] ?? Zap, label: "Workflow", tile };
  }
  return surface === "pulse"
    ? { icon: Activity, label: "Pulse note" }
    : { icon: Bot, label: "Agent library", tile: "rounded-squircle" };
}

/**
 * An Inbox row, project, workflow, note or agent-library persona in global
 * search, in its screen's icon and colours, with its project dot.
 */
export function SurfaceResultRow({
  hit,
  query,
}: {
  hit: SurfaceHit;
  query: string;
}) {
  const visual = surfaceVisual(hit);
  return (
    <>
      <span
        className={cn(
          "flex h-7 w-7 shrink-0 items-center justify-center rounded-md",
          visual.tile ? cn("border", visual.tile) : "bg-background/70",
        )}
      >
        <visual.icon className={cn("h-4 w-4", visual.text)} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold leading-5">
          <HighlightedSearchText query={query} text={hit.title} />
        </span>
        <span className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5 text-xs">
          <span
            className={cn(
              "font-medium",
              visual.text ?? "text-muted-foreground",
            )}
          >
            {visual.label}
          </span>
          {hit.chips.map((chip) => (
            <Badge key={chip} variant="secondary">
              {chip}
            </Badge>
          ))}
          {hit.color ? (
            <span
              aria-hidden
              className={cn("size-2 rounded-full", hit.color.dot)}
            />
          ) : null}
          {hit.meta ? (
            <span className="min-w-0 truncate text-muted-foreground">
              {hit.meta}
            </span>
          ) : null}
        </span>
      </span>
    </>
  );
}
