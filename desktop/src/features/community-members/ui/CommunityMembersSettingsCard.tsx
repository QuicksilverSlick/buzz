import {
  AlertCircle,
  Crown,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Search,
  Shield,
} from "lucide-react";
import { nip19 } from "nostr-tools";
import * as React from "react";
import { toast } from "sonner";

import {
  useChangeRelayMemberRoleMutation,
  useMyRelayMembershipLookupQuery,
  useRelayMembersQuery,
} from "@/features/community-members/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import {
  shouldShowCommunityMembersSettings,
  shouldWarnMissingMembershipSnapshot,
} from "@/shared/api/relayMembers";
import type {
  RelayMember,
  RelayMemberRole,
  UserProfileSummary,
} from "@/shared/api/types";
import {
  normalizePubkey,
  truncateNpub,
  UNAVAILABLE_KEY_LABEL,
} from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import { AddMemberDialog } from "./AddMemberDialog";
import { CommunityInviteDialog } from "./CommunityInviteDialog";
import { ConfirmRemoveDialog } from "./ConfirmRemoveDialog";

const ROLE_ORDER: Record<RelayMemberRole, number> = {
  owner: 0,
  admin: 1,
  member: 2,
};

function formatDisplayName(member: RelayMember, displayName?: string | null) {
  const trimmedDisplayName = displayName?.trim();
  if (
    trimmedDisplayName &&
    !trimmedDisplayName.toLowerCase().startsWith("npub1")
  ) {
    return trimmedDisplayName;
  }
  return member.role === "owner" ? "Community owner" : "Unnamed member";
}

function npubFromPubkey(pubkey: string): string | null {
  try {
    return nip19.npubEncode(pubkey);
  } catch {
    return null;
  }
}

function formatDate(dateString: string): string {
  return new Date(dateString).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function HoverMemberIdentity({
  displayName,
  pubkey,
}: {
  displayName: string;
  pubkey: string;
}) {
  const npub = npubFromPubkey(pubkey) ?? UNAVAILABLE_KEY_LABEL;
  return (
    <span className="inline-grid min-w-0 max-w-full grid-cols-1" title={npub}>
      <span
        className="col-start-1 row-start-1 max-w-40 truncate opacity-100 blur-0 transition-[max-width,opacity,filter] duration-[250ms] ease-in-out group-hover/member:max-w-0 group-hover/member:opacity-0 group-hover/member:blur-[2px] group-focus-within/member:max-w-0 group-focus-within/member:opacity-0 group-focus-within/member:blur-[2px] motion-reduce:transition-none"
        data-testid={`relay-member-name-${pubkey}`}
      >
        {displayName}
      </span>
      <span
        className="col-start-1 row-start-1 max-w-0 truncate font-mono text-2xs opacity-0 blur-0 transition-[max-width,opacity,filter] duration-[250ms] ease-in-out group-hover/member:max-w-40 group-hover/member:opacity-100 group-hover/member:blur-0 group-focus-within/member:max-w-40 group-focus-within/member:opacity-100 group-focus-within/member:blur-0 motion-reduce:transition-none"
        data-testid={`relay-member-npub-${pubkey}`}
      >
        {truncateNpub(npub)}
      </span>
    </span>
  );
}

function RelayMemberRow({
  currentRole,
  currentPubkey,
  onRemove,
  profile,
  member,
}: {
  /** Null when the role check failed; the relay still authorizes removals. */
  currentRole: RelayMemberRole | null;
  currentPubkey?: string;
  onRemove: (member: RelayMember) => void;
  profile?: UserProfileSummary;
  member: RelayMember;
}) {
  const changeRoleMutation = useChangeRelayMemberRoleMutation();
  const isSelf = currentPubkey
    ? normalizePubkey(currentPubkey) === normalizePubkey(member.pubkey)
    : false;
  const isBusy = changeRoleMutation.isPending;
  const canRemove =
    !isSelf &&
    member.role !== "owner" &&
    (currentRole === "owner" || member.role === "member");
  const canPromote = currentRole === "owner" && member.role === "member";
  const canDemote = currentRole === "owner" && member.role === "admin";
  const hasActions = canRemove || canPromote || canDemote;
  const displayName = formatDisplayName(member, profile?.displayName);

  async function mutateWithToast(
    action: () => Promise<unknown>,
    success: string,
  ) {
    try {
      await action();
      toast.success(success);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Couldn’t update this community member.",
      );
    }
  }

  return (
    <div
      className="group/member flex min-h-14 items-center gap-3 px-1 py-2.5"
      data-testid={`relay-member-row-${member.pubkey}`}
    >
      <UserProfilePopover
        pubkey={member.pubkey}
        triggerAriaLabel={`Open profile for ${displayName}`}
        triggerElement="span"
      >
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-9 w-9 text-xs shadow-none"
          label={displayName}
          shape={profile?.isAgent === true ? "squircle" : "circle"}
        />
      </UserProfilePopover>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5 text-sm font-medium">
          <HoverMemberIdentity
            displayName={displayName}
            pubkey={member.pubkey}
          />
          {member.role === "owner" ? (
            <Crown className="h-4 w-4 text-amber-500" />
          ) : null}
          {member.role === "admin" ? (
            <Shield className="h-4 w-4 text-blue-500" />
          ) : null}
        </div>
        <div
          className="flex items-center gap-1.5 text-xs text-muted-foreground/70"
          data-settings-subcopy
        >
          <span className="shrink-0 capitalize">{member.role}</span>
          <span aria-hidden="true" className="shrink-0">
            ·
          </span>
          <span className="shrink-0">Added {formatDate(member.createdAt)}</span>
          {isSelf ? (
            <>
              <span aria-hidden="true" className="shrink-0">
                ·
              </span>
              <span className="shrink-0">You</span>
            </>
          ) : null}
        </div>
      </div>

      {hasActions ? (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={`Actions for ${displayName}`}
              data-testid={`relay-member-actions-${member.pubkey}`}
              disabled={isBusy}
              size="icon"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {canPromote ? (
              <DropdownMenuItem
                onClick={() =>
                  void mutateWithToast(
                    () =>
                      changeRoleMutation.mutateAsync({
                        pubkey: member.pubkey,
                        role: "admin",
                      }),
                    "Made community admin",
                  )
                }
              >
                Make admin
              </DropdownMenuItem>
            ) : null}
            {canDemote ? (
              <DropdownMenuItem
                onClick={() =>
                  void mutateWithToast(
                    () =>
                      changeRoleMutation.mutateAsync({
                        pubkey: member.pubkey,
                        role: "member",
                      }),
                    "Made community member",
                  )
                }
              >
                Make member
              </DropdownMenuItem>
            ) : null}
            {canRemove && (canPromote || canDemote) ? (
              <DropdownMenuSeparator />
            ) : null}
            {canRemove ? (
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onClick={() => onRemove(member)}
              >
                Remove from community
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}

export function CommunityMembersSettingsCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const myMembershipQuery = useMyRelayMembershipLookupQuery();
  const lookup = myMembershipQuery.data;
  const currentRole = lookup?.membership?.role ?? null;
  const showCard = shouldShowCommunityMembersSettings(
    lookup,
    myMembershipQuery.isError,
  );
  const degradedMessage = myMembershipQuery.isError
    ? "Couldn’t check your role on this community."
    : shouldWarnMissingMembershipSnapshot(lookup)
      ? lookup?.roleSource === "hosted-owner"
        ? "This relay didn’t return its member list. Your hosted community account shows you as its owner."
        : "This relay didn’t return its member list."
      : null;
  const membersQuery = useRelayMembersQuery(showCard);
  const members = React.useMemo(
    () =>
      [...(membersQuery.data ?? [])].sort(
        (a, b) => ROLE_ORDER[a.role] - ROLE_ORDER[b.role],
      ),
    [membersQuery.data],
  );
  const profilesQuery = useUsersBatchQuery(
    members.map((member) => member.pubkey),
    {
      enabled: showCard && members.length > 0,
    },
  );
  const profiles = profilesQuery.data?.profiles;
  const [inviteDialogOpen, setInviteDialogOpen] = React.useState(false);
  const [addDialogOpen, setAddDialogOpen] = React.useState(false);
  const [removeTarget, setRemoveTarget] = React.useState<RelayMember | null>(
    null,
  );
  const [search, setSearch] = React.useState("");

  const filteredMembers = React.useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return members;
    return members.filter((member) => {
      const normalizedPubkey = normalizePubkey(member.pubkey);
      const profile = profiles?.[normalizedPubkey];
      const displayName = profile?.displayName?.toLowerCase() ?? "";
      const nip05 = profile?.nip05Handle?.toLowerCase() ?? "";
      const npub = npubFromPubkey(member.pubkey)?.toLowerCase() ?? "";
      return (
        displayName.includes(q) ||
        nip05.includes(q) ||
        npub.includes(q) ||
        member.pubkey.toLowerCase().includes(q) ||
        member.role.includes(q)
      );
    });
  }, [members, profiles, search]);

  if (myMembershipQuery.isLoading) {
    return (
      <section className="min-w-0" data-testid="settings-community-members">
        <p className="text-sm text-muted-foreground">
          Checking member permissions…
        </p>
      </section>
    );
  }

  if (!showCard) {
    return null;
  }

  return (
    <section className="min-w-0" data-testid="settings-community-members">
      <SettingsSectionHeader
        action={
          <div className="flex gap-2">
            <Button
              data-testid="community-invite-dialog-trigger"
              onClick={() => setInviteDialogOpen(true)}
              variant="outline"
            >
              Invite link
            </Button>
            <Button
              data-testid="add-relay-member"
              onClick={() => setAddDialogOpen(true)}
            >
              <Plus className="h-4 w-4" />
              Add member
            </Button>
          </div>
        }
        title="Members"
        description="Manage members and community access."
      />

      {degradedMessage ? (
        <div
          className="mb-6 flex items-start gap-2 rounded-lg border border-amber-500/40 px-3 py-2.5 text-sm"
          data-testid="community-members-degraded"
          role="status"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
          <p className="min-w-0 flex-1">{degradedMessage}</p>
          <Button
            data-testid="community-members-retry"
            onClick={() =>
              void Promise.all([
                myMembershipQuery.refetch(),
                membersQuery.refetch(),
              ])
            }
            size="sm"
            variant="outline"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Retry
          </Button>
        </div>
      ) : null}

      <SettingsOptionGroup
        title={
          <>
            Members
            {members.length > 0 ? (
              <span className="ml-1.5 font-normal">{members.length}</span>
            ) : null}
          </>
        }
      >
        <div className="space-y-3 p-4 sm:p-5">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className="w-full rounded-lg border border-border/70 bg-background py-2 pl-9 pr-3 text-sm placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="community-members-search"
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search members"
              spellCheck={false}
              type="text"
              value={search}
            />
          </div>

          {membersQuery.error instanceof Error ? (
            <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {membersQuery.error.message}
            </p>
          ) : null}

          {membersQuery.isLoading ? (
            <p className="py-3 text-sm text-muted-foreground">
              Loading community members…
            </p>
          ) : members.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border/70 px-3 py-6 text-center text-sm text-muted-foreground">
              {degradedMessage
                ? "No member list to show."
                : "No community members yet."}
            </p>
          ) : filteredMembers.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border/70 px-3 py-6 text-center text-sm text-muted-foreground">
              No members match your search.
            </p>
          ) : (
            <VirtualizedList
              className="max-h-[28rem] divide-y divide-border/60"
              estimateSize={60}
              getItemKey={(member) => member.pubkey}
              items={filteredMembers}
              renderItem={(member) => (
                <RelayMemberRow
                  currentPubkey={currentPubkey}
                  currentRole={currentRole}
                  member={member}
                  onRemove={setRemoveTarget}
                  profile={profiles?.[normalizePubkey(member.pubkey)]}
                />
              )}
            />
          )}
        </div>
      </SettingsOptionGroup>

      <AddMemberDialog
        isOwner={currentRole === "owner"}
        onOpenChange={setAddDialogOpen}
        open={addDialogOpen}
      />
      <CommunityInviteDialog
        isOwner={currentRole === "owner"}
        onOpenChange={setInviteDialogOpen}
        open={inviteDialogOpen}
      />
      <ConfirmRemoveDialog
        displayName={
          removeTarget
            ? (profiles?.[normalizePubkey(removeTarget.pubkey)]?.displayName ??
              null)
            : null
        }
        member={removeTarget}
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(null);
        }}
        open={removeTarget !== null}
      />
    </section>
  );
}
