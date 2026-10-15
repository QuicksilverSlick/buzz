import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Author label for a message a relay workflow posted. The relay's workflow
 * sink tags the post with its owner only (`buzz:workflow-owner`), never the
 * workflow's id or name, so there is no name to add after "Workflow".
 */
export const WORKFLOW_POST_LABEL = "Workflow";

/**
 * True for a message a relay workflow posted: signed by the active relay
 * (NIP-11 `self`) and carrying the relay's `buzz:workflow` marker. The tag
 * alone is not proof, since anyone can put custom tags on their own event;
 * a missing relay identity fails closed.
 */
export function isRelayWorkflowPost(
  event: { pubkey: string; tags: readonly string[][] },
  relaySelfPubkey: string | null | undefined,
): boolean {
  return (
    Boolean(relaySelfPubkey) &&
    normalizePubkey(event.pubkey) === normalizePubkey(relaySelfPubkey ?? "") &&
    event.tags.some((tag) => tag[0] === "buzz:workflow" && tag[1] === "true")
  );
}

/**
 * True for a post by one of the viewer's own workflows that does not name the
 * viewer in its template. The engine p-tags the owner on every post, which
 * would put each internal nudge in the owner's Inbox as a mention. A template
 * that does `@`-name its owner still carries `buzz:workflow-mention` for them,
 * so those stay.
 */
export function isOwnWorkflowPost(
  event: { pubkey: string; tags: readonly string[][] },
  currentPubkey: string | null | undefined,
  relaySelfPubkey: string | null | undefined,
): boolean {
  if (!currentPubkey || !isRelayWorkflowPost(event, relaySelfPubkey)) {
    return false;
  }
  const self = normalizePubkey(currentPubkey);
  const tagNames = (name: string) =>
    event.tags.some(
      (tag) => tag[0] === name && normalizePubkey(tag[1] ?? "") === self,
    );
  return tagNames("buzz:workflow-owner") && !tagNames("buzz:workflow-mention");
}
