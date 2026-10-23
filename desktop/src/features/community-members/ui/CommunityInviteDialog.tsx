import * as React from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DEFAULT_INVITE_TTL_SECS,
  InviteLinkSection,
} from "./InviteLinkSection";

/** Invite link only: adding someone directly has its own Add member dialog. */
export function CommunityInviteDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [ttlSecs, setTtlSecs] = React.useState(DEFAULT_INVITE_TTL_SECS);

  React.useEffect(() => {
    // Reset after the link section has unmounted so reopening never mints an
    // invite with the previous dialog session's expiry.
    if (!open) setTtlSecs(DEFAULT_INVITE_TTL_SECS);
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-h-[85vh] max-w-xl overflow-y-auto"
        data-testid="community-invite-dialog"
      >
        <DialogHeader>
          <DialogTitle>Invite link</DialogTitle>
          <DialogDescription>
            Share a link people can use to join as members.
          </DialogDescription>
        </DialogHeader>

        <section className="mt-2 space-y-3">
          <InviteLinkSection onTtlSecsChange={setTtlSecs} ttlSecs={ttlSecs} />
        </section>
      </DialogContent>
    </Dialog>
  );
}
