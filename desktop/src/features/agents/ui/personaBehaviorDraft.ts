import type {
  AcpSessionPolicy,
  AgentCapability,
  AgentVoiceProfile,
  PersonaBehaviorInput,
  RespondToMode,
} from "@/shared/api/types";

/**
 * Dialog-side draft of a definition's NIP-AP behavioral group.
 *
 * `respondTo: null` means "unset" — the definition carries no mode and the
 * harness default (owner-only) applies at mint. The distinction matters for
 * wire bytes, not semantics: a definition without behavioral fields must
 * stay without behavioral fields
 * through unrelated edits so its published content (and content hash) does
 * not move.
 */
export type PersonaBehaviorDraft = {
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  /** Raw text; only `parseInt > 0` submits (legacy dialog parity). */
  parallelism: string;
  /**
   * Capabilities the owner has granted. Empty is the default and means the
   * agent is granted nothing — the deny-by-default value.
   */
  capabilities: AgentCapability[];
  sessionPolicy: AcpSessionPolicy;
  /** Huddle voice; `aliases` is the raw comma-separated text. */
  voice: VoiceDraft;
};

export type VoiceDraft = {
  enabled: boolean;
  voice: string;
  persona: string;
  aliases: string;
  voiceWithoutOwner: boolean;
};

export const emptyVoiceDraft: VoiceDraft = voiceDraftFrom(undefined);

export const emptyPersonaBehaviorDraft: PersonaBehaviorDraft = {
  respondTo: null,
  respondToAllowlist: [],
  parallelism: "",
  capabilities: [],
  sessionPolicy: "channel",
  voice: emptyVoiceDraft,
};

function voiceDraftFrom(voice: AgentVoiceProfile | undefined): VoiceDraft {
  return {
    enabled: voice?.enabled ?? false,
    voice: voice?.voice ?? "",
    persona: voice?.persona ?? "",
    aliases: (voice?.aliases ?? []).join(", "),
    voiceWithoutOwner: voice?.voiceWithoutOwner ?? false,
  };
}

/** Absent when nothing is set, so a voiceless definition submits no voice. */
function voiceFromDraft(draft: VoiceDraft): AgentVoiceProfile | undefined {
  const aliases = draft.aliases
    .split(",")
    .map((alias) => alias.trim())
    .filter(Boolean);
  const voice: AgentVoiceProfile = {
    enabled: draft.enabled,
    voice: draft.voice.trim() || undefined,
    persona: draft.persona.trim() || undefined,
    aliases: aliases.length > 0 ? aliases : undefined,
    voiceWithoutOwner: draft.voiceWithoutOwner,
  };
  return Object.values(voice).some(Boolean) ? voice : undefined;
}

/** Seed the draft from a dialog-state behavior group (edit/duplicate). */
export function draftFromBehavior(
  behavior: PersonaBehaviorInput | undefined,
): PersonaBehaviorDraft {
  return {
    respondTo: behavior?.respondTo ?? null,
    respondToAllowlist: [...(behavior?.respondToAllowlist ?? [])],
    parallelism:
      behavior?.parallelism != null ? String(behavior.parallelism) : "",
    capabilities: [...(behavior?.capabilities ?? [])],
    sessionPolicy: behavior?.sessionPolicy ?? "channel",
    voice: voiceDraftFrom(behavior?.voice),
  };
}

/**
 * Allowlist-mode crash-loop guard (re-homed from the legacy create dialog):
 * an empty allowlist would crash every instance minted from the definition
 * at startup, so submit is blocked in create AND edit mode. The server-side
 * chokepoint (`apply_persona_behavior`) enforces the same rule.
 */
export function personaBehaviorDraftValid(draft: PersonaBehaviorDraft) {
  return draft.respondTo !== "allowlist" || draft.respondToAllowlist.length > 0;
}

function behaviorFromDraft(draft: PersonaBehaviorDraft): PersonaBehaviorInput {
  const parallelism = Number.parseInt(draft.parallelism, 10);
  const voice = voiceFromDraft(draft.voice);
  const group: PersonaBehaviorInput = {
    respondTo: draft.respondTo ?? undefined,
    // Mode and list travel as a unit; a list without allowlist mode is
    // stale data the author didn't choose (legacy dialog parity).
    respondToAllowlist:
      draft.respondTo === "allowlist" ? draft.respondToAllowlist : undefined,
    parallelism: parallelism > 0 ? parallelism : undefined,
    // Sorted so two owners ticking the same boxes in a different order produce
    // identical published bytes and the definition's content hash holds still.
    capabilities:
      draft.capabilities.length > 0
        ? [...draft.capabilities].sort()
        : undefined,
    sessionPolicy: draft.sessionPolicy,
    // Omitted, not undefined, so a voiceless group keeps its exact shape.
    ...(voice ? { voice } : {}),
  };
  return group;
}

/**
 * Resolve the behavior group a persona submit should carry.
 *
 * Absent (`undefined`) means "don't touch the stored behavior group"
 * server-side, so:
 * - a behavior group that is untouched relative to its seed submits nothing — an
 *   unrelated edit (rename, prompt tweak) must not rewrite the published
 *   definition's behavior bytes or flip its content hash;
 * - creates always submit the selected session policy; the channel default is
 *   omitted from durable/public JSON by the backend for wire compatibility;
 * - any real change submits the full group (replace-as-a-unit semantics);
 * - clearing the optional fields on edit still submits the channel default,
 *   because "submit nothing" would silently no-op the clear.
 *
 * Duplicate flows pass the source persona's behavior group as `seed` but with
 * `isEdit: false`: a duplicate is a CREATE, so a non-empty inherited
 * behavior group
 * must be submitted even though it equals the seed.
 */
export function behaviorForSubmit(
  draft: PersonaBehaviorDraft,
  seed: PersonaBehaviorDraft,
  isEdit: boolean,
): PersonaBehaviorInput | undefined {
  const group = behaviorFromDraft(draft);
  if (!isEdit) {
    return group;
  }
  const seedGroup = behaviorFromDraft(seed);
  if (JSON.stringify(group) === JSON.stringify(seedGroup)) {
    return undefined;
  }
  return group;
}
