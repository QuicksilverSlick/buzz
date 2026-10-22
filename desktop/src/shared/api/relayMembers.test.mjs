import assert from "node:assert/strict";
import test from "node:test";

import {
  canManageCommunityMembers,
  loadRelayMembershipLookup,
  shouldShowCommunityMembersSettings,
  shouldWarnMissingMembershipSnapshot,
} from "./relayMembers.ts";

test("open relays skip the membership snapshot request", async () => {
  const lookup = await loadRelayMembershipLookup("a".repeat(64), false, () =>
    assert.fail("open relays must not request a membership snapshot"),
  );

  assert.deepEqual(lookup, {
    snapshotFound: false,
    membershipRequired: false,
    membership: null,
    roleSource: null,
  });
});

test("membership relays request their membership snapshot", async () => {
  const pubkey = "a".repeat(64);
  let requestCount = 0;
  const lookup = await loadRelayMembershipLookup(pubkey, true, async () => {
    requestCount += 1;
    return {
      created_at: 1,
      tags: [["member", pubkey, "admin"]],
    };
  });

  assert.equal(requestCount, 1);
  assert.equal(lookup.snapshotFound, true);
  assert.equal(lookup.membershipRequired, true);
  assert.equal(lookup.membership?.role, "admin");
  assert.equal(lookup.roleSource, "snapshot");
});

test("community member management is hidden on open relays", () => {
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: false,
      membershipRequired: false,
      membership: null,
    }),
    false,
  );
});

test("community member management is visible to closed-relay admins and owners", () => {
  for (const role of ["admin", "owner"]) {
    assert.equal(
      canManageCommunityMembers({
        snapshotFound: true,
        membershipRequired: true,
        membership: { pubkey: "a".repeat(64), role },
      }),
      true,
    );
  }
});

test("community member management stays hidden while loading and from closed-relay non-admins", () => {
  assert.equal(canManageCommunityMembers(undefined), false);
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: true,
      membershipRequired: true,
      membership: { pubkey: "a".repeat(64), role: "member" },
    }),
    false,
  );
  assert.equal(
    canManageCommunityMembers({
      snapshotFound: true,
      membershipRequired: true,
      membership: null,
    }),
    false,
  );
});

test("missing snapshot warns when the relay requires membership", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: false,
      membershipRequired: true,
      membership: null,
    }),
    true,
  );
});

test("missing snapshot is normal on an open relay", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: false,
      membershipRequired: false,
      membership: null,
    }),
    false,
  );
});

test("an available snapshot never warns", () => {
  assert.equal(
    shouldWarnMissingMembershipSnapshot({
      snapshotFound: true,
      membershipRequired: true,
      membership: null,
    }),
    false,
  );
});

const VIEWER = "a".repeat(64);
const OTHER = "b".repeat(64);
const RELAY_URL = "wss://greymatter.communities.example";
const missingSnapshot = async () => null;
const hostedOwnedBy =
  (owner, normalizedHost = "greymatter.communities.example") =>
  async () => ({
    relayUrl: RELAY_URL,
    communities: [{ normalized_host: normalizedHost, owner_pubkey: owner }],
  });

test("hosted-owner fallback grants owner when the snapshot is missing and host and key match", async () => {
  const lookup = await loadRelayMembershipLookup(
    VIEWER,
    true,
    missingSnapshot,
    hostedOwnedBy(VIEWER.toUpperCase()),
  );

  assert.equal(lookup.snapshotFound, false);
  assert.equal(lookup.membership?.role, "owner");
  assert.equal(lookup.membership?.pubkey, VIEWER);
  assert.equal(lookup.roleSource, "hosted-owner");
  assert.equal(canManageCommunityMembers(lookup), true);
});

test("hosted-owner fallback is skipped without a Builderlab session", async () => {
  const lookup = await loadRelayMembershipLookup(
    VIEWER,
    true,
    missingSnapshot,
    async () => {
      throw new Error("Sign in to Builderlab first");
    },
  );

  assert.equal(lookup.membership, null);
  assert.equal(lookup.roleSource, null);
  assert.equal(shouldWarnMissingMembershipSnapshot(lookup), true);
});

test("hosted-owner fallback needs the same relay host and the viewer's key", async () => {
  for (const loadHosted of [
    hostedOwnedBy(OTHER),
    hostedOwnedBy(VIEWER, "someone-else.communities.example"),
    async () => ({
      relayUrl: RELAY_URL,
      communities: [
        {
          normalized_host: "greymatter.communities.example",
          owner_pubkey: VIEWER,
          archived_at: "2026-10-01T00:00:00Z",
        },
      ],
    }),
  ]) {
    const lookup = await loadRelayMembershipLookup(
      VIEWER,
      true,
      missingSnapshot,
      loadHosted,
    );
    assert.equal(lookup.membership, null);
    assert.equal(lookup.roleSource, null);
  }
});

test("a present snapshot always wins over the hosted-owner record", async () => {
  let hostedCalls = 0;
  const lookup = await loadRelayMembershipLookup(
    VIEWER,
    true,
    async () => ({ created_at: 1, tags: [["member", VIEWER, "member"]] }),
    async () => {
      hostedCalls += 1;
      return hostedOwnedBy(VIEWER)();
    },
  );

  assert.equal(hostedCalls, 0);
  assert.equal(lookup.membership?.role, "member");
  assert.equal(lookup.roleSource, "snapshot");
});

test("open relays never consult the hosted-owner record", async () => {
  const lookup = await loadRelayMembershipLookup(
    VIEWER,
    false,
    () => assert.fail("open relays must not request a membership snapshot"),
    () => assert.fail("open relays must not consult Builderlab"),
  );
  assert.equal(lookup.membership, null);
});

test("Members settings entry: hidden on open relays and from members, shown when degraded", () => {
  const openRelay = {
    snapshotFound: false,
    membershipRequired: false,
    membership: null,
    roleSource: null,
  };
  const missing = { ...openRelay, membershipRequired: true };
  const memberWithSnapshot = {
    snapshotFound: true,
    membershipRequired: true,
    membership: { pubkey: VIEWER, role: "member" },
    roleSource: "snapshot",
  };
  const adminWithSnapshot = {
    ...memberWithSnapshot,
    membership: { pubkey: VIEWER, role: "admin" },
  };

  assert.equal(shouldShowCommunityMembersSettings(openRelay, false), false);
  assert.equal(shouldShowCommunityMembersSettings(undefined, false), false);
  assert.equal(
    shouldShowCommunityMembersSettings(memberWithSnapshot, false),
    false,
  );
  assert.equal(
    shouldShowCommunityMembersSettings(adminWithSnapshot, false),
    true,
  );
  assert.equal(shouldShowCommunityMembersSettings(missing, false), true);
  assert.equal(shouldShowCommunityMembersSettings(undefined, true), true);
});
