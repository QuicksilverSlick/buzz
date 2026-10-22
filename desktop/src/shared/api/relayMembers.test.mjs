import assert from "node:assert/strict";
import test from "node:test";

import {
  canManageCommunityMembers,
  loadRelayMembershipLookup,
  relayMemberActions,
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

test("Members settings entry follows the known role, and shows when the role is unknown", () => {
  const viewer = "a".repeat(64);
  const openRelay = {
    snapshotFound: false,
    membershipRequired: false,
    membership: null,
  };
  const missingSnapshot = { ...openRelay, membershipRequired: true };
  const memberWithSnapshot = {
    snapshotFound: true,
    membershipRequired: true,
    membership: { pubkey: viewer, role: "member" },
  };
  const adminWithSnapshot = {
    ...memberWithSnapshot,
    membership: { pubkey: viewer, role: "admin" },
  };

  // Still loading: hidden.
  assert.equal(shouldShowCommunityMembersSettings(undefined, false), false);
  // First check failed with nothing cached: shown with a warning.
  assert.equal(shouldShowCommunityMembersSettings(undefined, true), true);
  assert.equal(
    shouldShowCommunityMembersSettings(missingSnapshot, false),
    true,
  );
  assert.equal(
    shouldShowCommunityMembersSettings(adminWithSnapshot, false),
    true,
  );
  assert.equal(
    shouldShowCommunityMembersSettings(adminWithSnapshot, true),
    true,
  );
  // A failed refetch never overrides what the cached data already says.
  for (const failed of [false, true]) {
    assert.equal(
      shouldShowCommunityMembersSettings(memberWithSnapshot, failed),
      false,
    );
    assert.equal(shouldShowCommunityMembersSettings(openRelay, failed), false);
  }
});

test("row actions need a known owner or admin viewer", () => {
  const none = { canRemove: false, canPromote: false, canDemote: false };
  for (const viewerRole of [null, "member"]) {
    for (const memberRole of ["owner", "admin", "member"]) {
      assert.deepEqual(relayMemberActions(viewerRole, memberRole, false), none);
    }
  }
  assert.deepEqual(relayMemberActions("admin", "member", false), {
    ...none,
    canRemove: true,
  });
  assert.deepEqual(relayMemberActions("admin", "admin", false), none);
  assert.deepEqual(relayMemberActions("owner", "member", false), {
    canRemove: true,
    canPromote: true,
    canDemote: false,
  });
  assert.deepEqual(relayMemberActions("owner", "admin", false), {
    canRemove: true,
    canPromote: false,
    canDemote: true,
  });
  assert.deepEqual(relayMemberActions("owner", "owner", true), none);
  assert.deepEqual(relayMemberActions("admin", "member", true), none);
});
