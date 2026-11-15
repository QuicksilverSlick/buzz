import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  attachManagedAgentToChannel,
  ensureManagedAgentRunning,
} from "./channelAgents";
import { channelsQueryKey, useOpenDmMutation } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, CreateManagedAgentResponse } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type TargetChannel = Pick<Channel, "id" | "name">;

// The relay opens a DM with at most 8 others, 9 people in all
// (buzz-relay handlers/command_executor.rs, open-DM checks).
const MAX_DM_OTHERS = 8;

/** The DM is already full, so no retry can ever add the agent. */
class FullDmError extends Error {}

/** Keeps creation successful when its optional channel attachment fails. */
export function useCreatedAgentChannelAttachment() {
  const queryClient = useQueryClient();
  const openDmMutation = useOpenDmMutation();
  const identityQuery = useIdentityQuery();

  function cachedDm(targetChannel: TargetChannel) {
    const channel = queryClient
      .getQueryData<Channel[]>(channelsQueryKey)
      ?.find((candidate) => candidate.id === targetChannel.id);
    return channel?.channelType === "dm" ? channel : null;
  }

  async function attach(
    created: CreateManagedAgentResponse,
    targetChannel: TargetChannel,
    dm: Channel | null,
  ) {
    if (!dm) {
      const attached = await attachManagedAgentToChannel(targetChannel.id, {
        agent: created.agent,
        role: "bot",
        ensureRunning: true,
      });
      created.agent = attached.agent;
      return;
    }

    // A DM's people are fixed when it opens, so the agent joins a new DM of
    // the same people plus itself (as the relay's own add-to-DM does).
    const self = identityQuery.data
      ? normalizePubkey(identityQuery.data.pubkey)
      : null;
    const others = dm.participantPubkeys.filter(
      (pubkey) => normalizePubkey(pubkey) !== self,
    );
    if (others.length + 1 > MAX_DM_OTHERS) {
      throw new FullDmError(
        "A DM holds at most 9 people; add it to a channel instead.",
      );
    }
    await openDmMutation.mutateAsync({
      pubkeys: [...others, created.agent.pubkey],
    });
    created.agent = (await ensureManagedAgentRunning(created.agent)).agent;
  }

  function addedText(
    created: CreateManagedAgentResponse,
    targetChannel: TargetChannel,
    dm: Channel | null,
  ) {
    return dm
      ? `${created.agent.name} is in a new DM with the same people.`
      : `Added ${created.agent.name} to #${targetChannel.name}`;
  }

  function showAttachmentFailure(
    created: CreateManagedAgentResponse,
    targetChannel: TargetChannel,
    dm: Channel | null,
    cause: unknown,
    toastId?: string | number,
  ) {
    const error =
      cause instanceof Error ? cause.message : "Failed to add agent.";
    const place = dm ? "a new DM" : `#${targetChannel.name}`;
    const id = toast.warning("Agent created", {
      description: `${created.agent.name} couldn’t be added to ${place}. ${error}`,
      id: toastId,
      action:
        cause instanceof FullDmError
          ? undefined
          : {
              label: "Try again",
              onClick: (event) => {
                event.preventDefault();
                toast.loading("Agent created", {
                  description: `Adding ${created.agent.name} to ${place}…`,
                  id,
                });
                void attach(created, targetChannel, dm).then(
                  () => {
                    toast.success("Agent created", {
                      description: addedText(created, targetChannel, dm),
                      id,
                    });
                  },
                  (retryCause: unknown) => {
                    showAttachmentFailure(
                      created,
                      targetChannel,
                      dm,
                      retryCause,
                      id,
                    );
                  },
                );
              },
            },
    });
  }

  async function presentCreatedAgent(
    created: CreateManagedAgentResponse,
    targetChannel?: TargetChannel | null,
  ) {
    if (created.spawnError || !targetChannel) {
      toast.success("Agent created");
      return;
    }

    const dm = cachedDm(targetChannel);
    try {
      await attach(created, targetChannel, dm);
      toast.success(
        "Agent created",
        dm ? { description: addedText(created, targetChannel, dm) } : undefined,
      );
    } catch (cause) {
      showAttachmentFailure(created, targetChannel, dm, cause);
    }
  }

  return { presentCreatedAgent };
}
