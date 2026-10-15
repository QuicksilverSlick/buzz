import { Zap } from "lucide-react";

import { cn } from "@/shared/lib/cn";

/**
 * Stands in for the author avatar on a relay workflow post, with the same Zap
 * icon the sidebar uses for Workflows. The author label next to it names it.
 */
export function WorkflowAvatar({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex h-9 w-9 shrink-0 items-center justify-center rounded-[30%] bg-muted text-muted-foreground",
        className,
      )}
      data-testid="workflow-avatar"
    >
      <Zap className="h-4 w-4" />
    </span>
  );
}
