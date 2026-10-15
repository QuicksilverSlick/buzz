import * as React from "react";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { getAgentAddressMentionPubkeys } from "../lib/agentAddressMention.mjs";
import { getVisibleAgentAddressPubkeys } from "../lib/getVisibleAgentAddressPubkeys";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { truncateNpub } from "@/shared/lib/pubkey";
import {
  inlineChipIconClasses,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** The subset of resolved mentions that are known agents (agent chips). */
export function useAgentMentionPubkeysByName(
  mentionPubkeysByName: Record<string, string> | undefined,
  isKnownAgentPubkey: (pubkey: string) => boolean,
) {
  return React.useMemo(() => {
    if (!mentionPubkeysByName) {
      return undefined;
    }
    const values: Record<string, string> = {};
    for (const [name, pubkey] of Object.entries(mentionPubkeysByName)) {
      if (isKnownAgentPubkey(pubkey)) {
        values[name] = pubkey;
      }
    }
    return Object.keys(values).length > 0 ? values : undefined;
  }, [isKnownAgentPubkey, mentionPubkeysByName]);
}

/** Resolve all literal competitors before hiding tag-backed address chips. */
export function useMessageAgentAddressPrefix({
  profiles,
  body,
  tags,
  mentionNames,
  mentionPubkeysByName,
  isKnownAgentPubkey,
}: {
  profiles?: UserProfileLookup;
  body: string;
  tags?: string[][];
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  isKnownAgentPubkey: (pubkey: string) => boolean;
}) {
  const pubkeys = React.useMemo(
    () =>
      getVisibleAgentAddressPubkeys(
        body,
        getAgentAddressMentionPubkeys(tags).filter(isKnownAgentPubkey),
        mentionPubkeysByName,
        mentionNames,
      ),
    [body, tags, isKnownAgentPubkey, mentionPubkeysByName, mentionNames],
  );
  return pubkeys.length > 0 ? (
    <MessageAgentAddressPrefix profiles={profiles} pubkeys={pubkeys} />
  ) : undefined;
}

/**
 * Visible send-state prefix for recipients kept in the composer address tray.
 * Each chip leads with the agent's own avatar, as the tray showed it while
 * composing, so the sent message reads as the same agent. An agent with no
 * image keeps the CSS agent glyph. The avatar never shows initials: chip text
 * must stay the label, or copy drops the mention (see `InlineChip`).
 */
export function MessageAgentAddressPrefix({
  profiles,
  pubkeys,
}: {
  profiles?: UserProfileLookup;
  pubkeys: readonly string[];
}) {
  return (
    <>
      {pubkeys.map((pubkey) => {
        const profile = profiles?.[pubkey];
        const label =
          profile?.displayName?.trim() ||
          profile?.name?.trim() ||
          truncateNpub(pubkey);
        const avatarUrl = profile?.avatarUrl ?? null;
        return (
          <React.Fragment key={pubkey}>
            {/* biome-ignore lint/a11y/useValidAriaRole: UserProfilePopover uses role for agent classification, not as an ARIA attribute. */}
            <UserProfilePopover
              botIdenticonValue={label}
              pubkey={pubkey}
              role="bot"
              triggerElement="span"
            >
              <span
                className={cn(
                  MENTION_CHIP_BASE_CLASSES,
                  MENTION_CHIP_HOVER_CLASSES,
                  "agent-mention-highlight cursor-pointer",
                  avatarUrl
                    ? "gap-[var(--inline-chip-icon-gap)]"
                    : inlineChipIconClasses("agent"),
                )}
                data-mention=""
              >
                {avatarUrl ? (
                  <span aria-hidden="true" className="inline-flex select-none">
                    <UserAvatar
                      avatarUrl={avatarUrl}
                      className="h-[1.15em] w-[1.15em]"
                      displayName={label}
                      initialsLabel=""
                      shape="squircle"
                      size="xs"
                      testId="message-agent-address-avatar"
                    />
                  </span>
                ) : null}
                {label}
                <AgentManagementMarker
                  pubkey={pubkey}
                  ownerPubkey={profile?.ownerPubkey}
                />
              </span>
            </UserProfilePopover>{" "}
          </React.Fragment>
        );
      })}
    </>
  );
}
