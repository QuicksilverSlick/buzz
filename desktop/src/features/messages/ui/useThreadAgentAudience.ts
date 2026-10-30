import * as React from "react";

import { useKeepMentionedAgentsPinned } from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import {
  selectOneOnOneThreadAgent,
  syncOneOnOneThreadAgent,
  type ThreadParticipantSource,
} from "@/features/messages/lib/oneOnOneThreadAgent";
import {
  initializePersistentAgentAudience,
  usePersistentAgentAudience,
} from "@/features/messages/lib/persistentAgentAudience";

export function useThreadAgentAudience({
  currentPubkey,
  isAgentPubkey,
  onSharedThreadAgent,
  participants,
  replyToPubkeys,
  rootTags,
  scope,
}: {
  currentPubkey: string | null;
  isAgentPubkey: (pubkey: string) => boolean;
  /** Drops the one-on-one chip (and its prefix) once the thread is shared. */
  onSharedThreadAgent: (pubkey: string) => void;
  participants: readonly ThreadParticipantSource[] | null;
  replyToPubkeys: readonly (string | null | undefined)[];
  rootTags: readonly string[][];
  scope: string | null;
}) {
  const audience = usePersistentAgentAudience(scope);
  const keepMentionedAgentsPinned = useKeepMentionedAgentsPinned();

  const rootAgentPubkeys = React.useMemo(
    () =>
      rootTags.flatMap((tag) => {
        const pubkey = tag[0] === "p" ? tag[1] : null;
        return pubkey && isAgentPubkey(pubkey) ? [pubkey] : [];
      }),
    [isAgentPubkey, rootTags],
  );

  React.useEffect(() => {
    if (!scope || !keepMentionedAgentsPinned) return;
    initializePersistentAgentAudience(scope, rootAgentPubkeys);
  }, [keepMentionedAgentsPinned, rootAgentPubkeys, scope]);

  const oneOnOneAgent = selectOneOnOneThreadAgent({
    currentPubkey,
    isAgentPubkey,
    participants,
    replyToPubkeys,
  });
  const onSharedThreadAgentRef = React.useRef(onSharedThreadAgent);
  onSharedThreadAgentRef.current = onSharedThreadAgent;
  // Audience changes re-run this so a store reset (switch off) re-adds the chip.
  React.useEffect(() => {
    if (!scope) return;
    const added = syncOneOnOneThreadAgent(scope, oneOnOneAgent);
    if (
      added &&
      audience.pubkeys.includes(added) &&
      !(keepMentionedAgentsPinned && rootAgentPubkeys.includes(added))
    ) {
      onSharedThreadAgentRef.current(added);
    }
  }, [
    audience.pubkeys,
    keepMentionedAgentsPinned,
    oneOnOneAgent,
    rootAgentPubkeys,
    scope,
  ]);

  return { audience, keepMentionedAgentsPinned };
}
