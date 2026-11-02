import { AudioLines, LoaderCircle } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { useTalkToTalos } from "./useTalkToTalos";

// The same fixed 28px row as TOP_CHROME_ICON_BUTTON_CLASS in AppTopChrome.
const CHROME_CLASS =
  "h-[28px] shrink-0 gap-1.5 rounded-[4px] px-2 text-xs text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground [&_svg]:size-[16px]";

/** The top bar and Settings header button, or (given `participants`) the room's quick-add. */
export function TalkToTalosButton({
  className,
  participants,
}: {
  className?: string;
  participants?: string[];
}) {
  const { pubkey, pending, talk } = useTalkToTalos();
  if (!pubkey || participants?.some((p) => normalizePubkey(p) === pubkey)) {
    return null;
  }
  const room = participants !== undefined;
  return (
    <Button
      aria-busy={pending}
      className={cn(!room && CHROME_CLASS, className)}
      data-testid={room ? "talos-quick-add" : "talk-to-talos"}
      disabled={pending}
      onClick={talk}
      size="sm"
      title={
        pending
          ? "Waiting for Talos. This stops by itself if Talos doesn't answer within 15 seconds."
          : undefined
      }
      variant={room ? "outline" : "ghost"}
    >
      {pending ? <LoaderCircle className="animate-spin" /> : <AudioLines />}
      {room ? "Add Talos" : "Talk to Talos"}
    </Button>
  );
}
