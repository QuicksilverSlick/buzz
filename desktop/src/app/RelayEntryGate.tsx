import { useState, useSyncExternalStore } from "react";
import { relayClient } from "@/shared/api/relayClient";
import { PRODUCT_NAME } from "@/shared/constants/brand";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";

export function RelayEntryGate({
  relayUrl,
  communities,
  onSwitch,
  onRetry,
  onAddressChanged,
}: {
  relayUrl: string;
  communities: { id: string; name: string; relayUrl: string }[];
  onSwitch: (id: string) => void;
  onRetry: () => void;
  onAddressChanged: () => void;
}) {
  const entry = relayClient.entryRequirements;
  const state = useSyncExternalStore(entry.subscribe, entry.getSnapshot);
  const [editing, setEditing] = useState(false);
  const visible = state.relayUrl === relayUrl && state.verdict === "required";
  return (
    <>
      <Dialog open={visible && !editing}>
        <DialogContent
          showCloseButton={false}
          onEscapeKeyDown={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
          data-testid="relay-entry-guidance"
        >
          {/* Dreamforge: Block's copy sent people to download Block's app. */}
          <DialogTitle>
            {PRODUCT_NAME} can't join this community yet
          </DialogTitle>
          <DialogDescription>
            This community requires a sign-in method {PRODUCT_NAME} doesn't
            support yet. Your keys and saved communities are still here.
          </DialogDescription>
          <p className="break-all text-sm text-muted-foreground">{relayUrl}</p>
          <p className="text-sm">Ask your {PRODUCT_NAME} admin, then retry.</p>
          <Button variant="outline" disabled={state.pending} onClick={onRetry}>
            {state.pending ? "Checking community…" : "Retry community check"}
          </Button>
          {communities
            .filter((community) => community.relayUrl !== relayUrl)
            .map((community) => (
              <Button
                variant="outline"
                key={community.id}
                onClick={() => onSwitch(community.id)}
              >
                Open {community.name}
              </Button>
            ))}
          <Button variant="ghost" onClick={() => setEditing(true)}>
            Change community address
          </Button>
        </DialogContent>
      </Dialog>
      {editing ? (
        <CommunityChangeOverlay
          onClose={() => setEditing(false)}
          onUpdated={(_name, updatedRelayUrl) => {
            if (updatedRelayUrl !== relayUrl) onAddressChanged();
            setEditing(false);
          }}
        />
      ) : null}
    </>
  );
}
