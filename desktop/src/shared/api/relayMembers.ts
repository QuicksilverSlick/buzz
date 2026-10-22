import {
  type HostedCommunitiesResponse,
  type HostedCommunity,
  hostedCommunityRelayUrl,
} from "@/features/communities/hostedCommunityApi";
import { relayClient } from "@/shared/api/relayClient";
import { getRelayWsUrl, invokeTauri, signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type {
  RelayEvent,
  RelayMember,
  RelayMemberRole,
} from "@/shared/api/types";

const KIND_NIP43_MEMBERSHIP_LIST = 13534;
const KIND_RELAY_ADMIN_ADD_MEMBER = 9030;
const KIND_RELAY_ADMIN_REMOVE_MEMBER = 9031;
const KIND_RELAY_ADMIN_CHANGE_ROLE = 9032;

function isRelayMemberRole(
  value: string | undefined,
): value is RelayMemberRole {
  return value === "owner" || value === "admin" || value === "member";
}

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function eventCreatedAtIso(event: RelayEvent): string {
  return new Date(event.created_at * 1_000).toISOString();
}

export type RelayMembershipLookup = {
  /**
   * True when the relay returned a NIP-43 membership snapshot.
   *
   * Open relays do not publish kind:13534, so absence of this snapshot must not
   * be treated as a denial by onboarding.
   */
  snapshotFound: boolean;
  membershipRequired: boolean;
  membership: RelayMember | null;
  /**
   * Where `membership` came from. `hosted-owner` means the relay sent no
   * snapshot and the viewer's Builderlab account lists them as this hosted
   * community's owner. The relay still authorizes every admin event itself.
   */
  roleSource: "snapshot" | "hosted-owner" | null;
};

export function canManageCommunityMembers(
  lookup: RelayMembershipLookup | undefined,
): boolean {
  const role = lookup?.membership?.role;
  return role === "owner" || role === "admin";
}

export function shouldWarnMissingMembershipSnapshot(
  lookup: RelayMembershipLookup | undefined,
): boolean {
  return lookup?.membershipRequired === true && !lookup.snapshotFound;
}

/**
 * The Members settings entry shows for owners and admins, and also when the
 * role check failed or the relay sent no member list, so a degraded relay
 * never hides member management without saying why. Open relays stay hidden.
 */
export function shouldShowCommunityMembersSettings(
  lookup: RelayMembershipLookup | undefined,
  lookupFailed: boolean,
): boolean {
  return (
    lookupFailed ||
    canManageCommunityMembers(lookup) ||
    shouldWarnMissingMembershipSnapshot(lookup)
  );
}

export function relayMembersFromEvent(event: RelayEvent): RelayMember[] {
  const seen = new Set<string>();
  const members: RelayMember[] = [];
  const createdAt = eventCreatedAtIso(event);

  for (const tag of event.tags) {
    const [name, rawPubkey, maybeRoleOrRelay, maybePTagRole] = tag;
    if (name !== "member" && name !== "p") continue;
    if (!rawPubkey) continue;

    const pubkey = normalizePubkey(rawPubkey);
    if (!/^[0-9a-f]{64}$/.test(pubkey) || seen.has(pubkey)) continue;
    seen.add(pubkey);

    const rawRole = name === "member" ? maybeRoleOrRelay : maybePTagRole;
    const role = isRelayMemberRole(rawRole) ? rawRole : "member";

    members.push({
      pubkey,
      role,
      addedBy: null,
      createdAt,
    });
  }

  return members;
}

export function relayMembershipLookupFromEvent(
  event: RelayEvent | null,
  pubkey: string,
  membershipRequired = event !== null,
): RelayMembershipLookup {
  if (!event) {
    return {
      snapshotFound: false,
      membershipRequired,
      membership: null,
      roleSource: null,
    };
  }

  const normalizedPubkey = normalizePubkey(pubkey);
  const membership =
    relayMembersFromEvent(event).find(
      (member) => normalizePubkey(member.pubkey) === normalizedPubkey,
    ) ?? null;
  return {
    snapshotFound: true,
    membershipRequired,
    membership,
    roleSource: membership ? "snapshot" : null,
  };
}

async function fetchMembershipListEvent(): Promise<RelayEvent | null> {
  return relayClient.fetchFirstEvent({
    kinds: [KIND_NIP43_MEMBERSHIP_LIST],
    limit: 1,
  });
}

export type HostedCommunitiesSource = () => Promise<{
  relayUrl: string;
  communities: HostedCommunity[];
}>;

/**
 * Lists the viewer's Builderlab communities without ever prompting: with no
 * Builderlab session the native command rejects locally ("Sign in to
 * Builderlab first") before any request is sent.
 */
async function loadHostedCommunities() {
  const [relayUrl, response] = await Promise.all([
    getRelayWsUrl(),
    invokeTauri<HostedCommunitiesResponse>("list_builderlab_communities"),
  ]);
  return { relayUrl, communities: response.communities ?? [] };
}

function relayHost(url: string | null): string | null {
  if (!url) return null;
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
}

async function isHostedCommunityOwner(
  pubkey: string,
  loadHosted: HostedCommunitiesSource,
): Promise<boolean> {
  try {
    const { relayUrl, communities } = await loadHosted();
    const host = relayHost(relayUrl);
    const viewer = normalizePubkey(pubkey);
    return (
      host !== null &&
      communities.some(
        (community) =>
          !community.archived_at &&
          relayHost(hostedCommunityRelayUrl(community)) === host &&
          normalizePubkey(community.owner_pubkey ?? "") === viewer,
      )
    );
  } catch {
    // No Builderlab session, or Builderlab is unreachable: no second source.
    return false;
  }
}

/**
 * Loads the NIP-43 snapshot only when the relay advertises membership support.
 *
 * When a membership relay sends no snapshot, the viewer's Builderlab record
 * is the only other place that knows a role, and it only knows owners.
 */
export async function loadRelayMembershipLookup(
  pubkey: string,
  membershipRequired: boolean,
  fetchSnapshot: () => Promise<RelayEvent | null> = fetchMembershipListEvent,
  loadHosted: HostedCommunitiesSource = loadHostedCommunities,
): Promise<RelayMembershipLookup> {
  if (!membershipRequired) {
    return relayMembershipLookupFromEvent(null, pubkey, false);
  }
  const lookup = relayMembershipLookupFromEvent(
    await fetchSnapshot(),
    pubkey,
    true,
  );
  if (
    lookup.snapshotFound ||
    !(await isHostedCommunityOwner(pubkey, loadHosted))
  ) {
    return lookup;
  }
  return {
    ...lookup,
    membership: {
      pubkey: normalizePubkey(pubkey),
      role: "owner",
      addedBy: null,
      createdAt: new Date().toISOString(),
    },
    roleSource: "hosted-owner",
  };
}

export async function listRelayMembers(): Promise<RelayMember[]> {
  const event = await fetchMembershipListEvent();
  return event ? relayMembersFromEvent(event) : [];
}

export async function relayRequiresMembership(
  relayUrl?: string,
): Promise<boolean> {
  return invokeTauri<boolean>("relay_requires_membership", { relayUrl });
}

export async function getMyRelayMembershipLookup(): Promise<RelayMembershipLookup> {
  const [{ pubkey }, membershipRequired] = await Promise.all([
    getIdentity(),
    relayRequiresMembership(),
  ]);
  return loadRelayMembershipLookup(pubkey, membershipRequired);
}

export async function getMyRelayMembership(): Promise<RelayMember | null> {
  return (await getMyRelayMembershipLookup()).membership;
}

async function publishRelayAdminEvent(
  kind: number,
  targetPubkey: string,
  role?: string,
): Promise<void> {
  const tags = [["p", normalizePubkey(targetPubkey)]];
  if (role) {
    tags.push(["role", role]);
  }

  const event = await signRelayEvent({
    kind,
    content: "",
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out while updating relay access.",
    "Failed to update relay access.",
  );
}

export async function addRelayMember(
  targetPubkey: string,
  role: string,
): Promise<void> {
  await publishRelayAdminEvent(KIND_RELAY_ADMIN_ADD_MEMBER, targetPubkey, role);
}

export async function removeRelayMember(targetPubkey: string): Promise<void> {
  await publishRelayAdminEvent(KIND_RELAY_ADMIN_REMOVE_MEMBER, targetPubkey);
}

export async function changeRelayMemberRole(
  targetPubkey: string,
  newRole: string,
): Promise<void> {
  await publishRelayAdminEvent(
    KIND_RELAY_ADMIN_CHANGE_ROLE,
    targetPubkey,
    newRole,
  );
}
