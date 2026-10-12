import { BookOpenText, LogIn, SquareTerminal } from "lucide-react";
import type * as React from "react";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import type { ActiveDmHeaderParticipant } from "@/features/channels/useActiveChannelHeader";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import {
  getChannelDescription,
  getChannelDetail,
  getChannelHeaderLine,
} from "@/features/channels/lib/channelDescription";
import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { ChannelGlyph } from "@/features/channels/ui/ChannelGlyph";
import { ChannelHeaderStatusBadge } from "@/features/channels/ui/ChannelHeaderStatusBadge";
import { useChannelModerationCapabilities } from "@/features/channels/ui/ChannelManagementModerationActions";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { ownsAnyAgent } from "@/features/agents/knownAgentPubkeys";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { WELCOME_TEAM_ID } from "@/features/onboarding/welcomeGuide";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { UserNameIndicators } from "@/features/user-status/ui/UserNameIndicators";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  toggleTerminalPanel,
  useTerminalPanel,
} from "@/features/terminal/terminalPanelStore";
import { TERMINAL_LABEL } from "@/shared/constants/brand";
import { isMacPlatform } from "@/shared/lib/platform";
import { CHANNEL_ABOUT_TITLE } from "./ChannelAboutPanel";

const DM_HEADER_AVATAR_SIZE = 32;
const DM_HEADER_AVATAR_STATUS_GEOMETRY = scaleProfileAvatarStatusGeometry(
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  DM_HEADER_AVATAR_SIZE,
);

type ChannelScreenHeaderProps = {
  activeChannel: Channel | null;
  activeChannelEphemeralDisplay: EphemeralChannelDisplay | null;
  activeChannelTitle: string;
  actionsVariant?: "inline" | "compact";
  activeDmAvatarUrl: string | null;
  activeDmHeaderParticipants: ActiveDmHeaderParticipant[];
  activeDmPresenceStatus: PresenceStatus | null;
  /** The side panel is showing About this channel. */
  channelAboutOpen?: boolean;
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  currentPubkey?: string;
  /** The channel's canvas has content to show in About this channel. */
  hasChannelAbout?: boolean;
  headerEndActions?: React.ReactNode;
  isAddBotOpen?: boolean;
  isJoining?: boolean;
  showHeaderContent?: boolean;
  transparentChrome?: boolean;
  onAddBotOpenChange?: (open: boolean) => void;
  /** Opens channel settings in edit mode for an undescribed channel. */
  onAddDescription?: () => void;
  onJoinChannel?: () => Promise<void>;
  onManageChannel: () => void;
  onToggleChannelAbout?: () => void;
  onToggleMembers: () => void;
};

export function ChannelScreenHeader({
  activeChannel,
  activeChannelEphemeralDisplay,
  activeChannelTitle,
  actionsVariant = "inline",
  activeDmAvatarUrl,
  activeDmHeaderParticipants,
  activeDmPresenceStatus,
  channelAboutOpen = false,
  chromeWrapperRef,
  currentPubkey,
  hasChannelAbout = false,
  headerEndActions,
  isAddBotOpen,
  isJoining = false,
  onAddBotOpenChange,
  onAddDescription,
  showHeaderContent = true,
  transparentChrome = false,
  onJoinChannel,
  onManageChannel,
  onToggleChannelAbout,
  onToggleMembers,
}: ChannelScreenHeaderProps) {
  const isGroupDm =
    activeChannel?.channelType === "dm" &&
    activeDmHeaderParticipants.length > 1;
  const activeDmParticipant = activeDmHeaderParticipants[0] ?? null;
  const showJoinButton =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt &&
    onJoinChannel;

  // Only an undescribed channel needs the roster, to decide whether this
  // viewer gets "Add a description". The channel screen already loads it.
  const needsManageCheck =
    onAddDescription !== undefined &&
    activeChannel !== null &&
    activeChannel.channelType !== "dm" &&
    !activeChannel.archivedAt &&
    getChannelDetail(activeChannel) === null;
  const membersQuery = useChannelMembersQuery(
    activeChannel?.id ?? null,
    needsManageCheck,
  );
  const { canManageChannel } = useChannelModerationCapabilities(
    needsManageCheck ? membersQuery.data : undefined,
    currentPubkey,
    needsManageCheck,
  );
  const headerLine = getChannelHeaderLine(activeChannel, canManageChannel);
  const subtitle =
    headerLine.kind === "description" ? (
      <p
        className="min-w-0 truncate pl-5 text-xs leading-4 text-muted-foreground"
        data-testid="chat-description"
        title={headerLine.title}
      >
        {headerLine.text}
      </p>
    ) : headerLine.kind === "add-description" && onAddDescription ? (
      <button
        className="ml-5 min-w-0 truncate rounded-sm text-xs leading-4 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        data-testid="chat-add-description"
        onClick={onAddDescription}
        type="button"
      >
        Add a description
      </button>
    ) : null;

  const terminalPanel = useTerminalPanel();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  // The terminal is agent tooling. Hide it from people who own no agents
  // beyond the Welcome Team that onboarding gives every new member, unless
  // it is already open, so the button can still close it.
  const showTerminalButton =
    activeChannel !== null &&
    (terminalPanel.mode !== "closed" ||
      ownsAnyAgent(
        managedAgentsQuery.data,
        relayAgentsQuery.data,
        currentPubkey,
        WELCOME_TEAM_ID,
      ));
  const terminalButton = showTerminalButton ? (
    <Button
      aria-label={
        terminalPanel.mode === "closed"
          ? `Open ${TERMINAL_LABEL}`
          : `Hide ${TERMINAL_LABEL}`
      }
      onClick={toggleTerminalPanel}
      size="icon"
      title={`${TERMINAL_LABEL} (${isMacPlatform() ? "⌘J" : "Ctrl+J"})`}
      type="button"
      variant={terminalPanel.mode === "closed" ? "outline" : "secondary"}
    >
      <SquareTerminal />
    </Button>
  ) : null;
  // Shown only when the canvas has something to read; it stays visible while
  // the panel is open so the same button can close it.
  const aboutButton =
    activeChannel !== null &&
    onToggleChannelAbout &&
    (hasChannelAbout || channelAboutOpen) ? (
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <Button
            aria-label={CHANNEL_ABOUT_TITLE}
            aria-pressed={channelAboutOpen}
            data-testid="channel-about-trigger"
            onClick={onToggleChannelAbout}
            size="icon"
            type="button"
            variant={channelAboutOpen ? "secondary" : "outline"}
          >
            <BookOpenText />
          </Button>
        </TooltipTrigger>
        <TooltipContent>{CHANNEL_ABOUT_TITLE}</TooltipContent>
      </Tooltip>
    ) : null;
  const channelActions = activeChannel ? (
    showJoinButton ? (
      <div className="flex items-center gap-1">
        <Button
          disabled={isJoining}
          onClick={() => void onJoinChannel()}
          size="sm"
          variant="default"
        >
          <LogIn className="mr-1.5 h-4 w-4" />
          {isJoining ? "Joining…" : "Join"}
        </Button>
        {headerEndActions}
      </div>
    ) : (
      <ChannelMembersBar
        channel={activeChannel}
        currentPubkey={currentPubkey}
        endActions={headerEndActions}
        isAddBotOpen={isAddBotOpen}
        onAddBotOpenChange={onAddBotOpenChange}
        onManageChannel={onManageChannel}
        onToggleMembers={onToggleMembers}
        variant={actionsVariant}
      />
    )
  ) : (
    headerEndActions
  );
  const actions =
    aboutButton || terminalButton || channelActions ? (
      <div className="flex items-center gap-1">
        {aboutButton}
        {terminalButton}
        {channelActions}
      </div>
    ) : null;

  if (!showHeaderContent) {
    return null;
  }

  return (
    <ChatHeader
      belowSystemChrome
      chromeWrapperRef={chromeWrapperRef}
      actions={actions}
      channelType={activeChannel?.channelType}
      description={getChannelDescription(activeChannel)}
      leadingContent={
        activeChannel?.channelType === "dm" ? (
          isGroupDm ? (
            <DmHeaderParticipantStack
              participants={activeDmHeaderParticipants}
            />
          ) : activeDmParticipant ? (
            <UserProfilePopover
              pubkey={activeDmParticipant.pubkey}
              role={activeDmParticipant.isAgent ? "bot" : undefined}
              triggerAriaLabel={`Open profile for ${activeChannelTitle}`}
              triggerElement="span"
            >
              <ProfileAvatarWithStatus
                avatarClassName="text-xs"
                avatarUrl={activeDmAvatarUrl}
                className="mr-1.5 h-8 w-8"
                geometry={DM_HEADER_AVATAR_STATUS_GEOMETRY}
                iconClassName="h-4 w-4"
                label={activeChannelTitle}
                shape={activeDmParticipant.isAgent ? "squircle" : "circle"}
                size={DM_HEADER_AVATAR_SIZE}
                status={activeDmPresenceStatus ?? "offline"}
                statusTestId="chat-presence-badge"
                testId="chat-header-dm-avatar"
              />
            </UserProfilePopover>
          ) : (
            <ProfileAvatarWithStatus
              avatarClassName="text-xs"
              avatarUrl={activeDmAvatarUrl}
              className="mr-1.5 h-8 w-8"
              geometry={DM_HEADER_AVATAR_STATUS_GEOMETRY}
              iconClassName="h-4 w-4"
              label={activeChannelTitle}
              shape="circle"
              size={DM_HEADER_AVATAR_SIZE}
              status={activeDmPresenceStatus ?? "offline"}
              statusTestId="chat-presence-badge"
              testId="chat-header-dm-avatar"
            />
          )
        ) : activeChannel ? (
          <ChannelGlyph
            channel={activeChannel}
            className="h-4 w-4 translate-y-px text-muted-foreground"
          />
        ) : undefined
      }
      statusBadge={
        <>
          <ChannelHeaderStatusBadge
            ephemeralDisplay={activeChannelEphemeralDisplay}
          />
          {!isGroupDm && activeDmParticipant ? (
            <AgentManagementMarker
              pubkey={activeDmParticipant.pubkey}
              testId="chat-header-agent-provenance"
            />
          ) : null}
        </>
      }
      subtitle={subtitle}
      title={activeChannelTitle}
      titleAdornment={
        activeChannel?.channelType === "dm" && !isGroupDm ? (
          <UserNameIndicators
            className="ml-1"
            pubkey={activeDmParticipant?.pubkey}
            size="dm"
          />
        ) : null
      }
      transparentChrome={transparentChrome}
      visibility={activeChannel?.visibility}
    />
  );
}

function DmHeaderParticipantStack({
  participants,
}: {
  participants: ActiveDmHeaderParticipant[];
}) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const stackItemCount = visibleParticipants.length + (hiddenCount > 0 ? 1 : 0);

  return (
    <div
      className="mr-1.5 flex shrink-0 items-center"
      data-testid="chat-header-dm-avatar-stack"
    >
      {visibleParticipants.map((participant, index) => (
        <UserProfilePopover
          key={participant.pubkey}
          pubkey={participant.pubkey}
          triggerAriaLabel={`Open profile for ${participant.displayName}`}
          triggerElement="span"
          role={participant.isAgent ? "bot" : undefined}
        >
          <span
            className={index > 0 ? "-ml-2" : ""}
            data-testid="chat-header-dm-avatar-stack-participant"
            style={{ zIndex: index + 1 }}
          >
            <UserAvatar
              accent={participant.isAgent === true}
              avatarUrl={participant.avatarUrl}
              className={
                index < stackItemCount - 1
                  ? "h-8 w-8 text-xs ring-2 ring-background"
                  : "h-8 w-8 text-xs"
              }
              displayName={participant.displayName}
              shape={participant.isAgent ? "squircle" : "circle"}
              size="sm"
            />
          </span>
        </UserProfilePopover>
      ))}
      {hiddenCount > 0 ? (
        <div
          className={visibleParticipants.length > 0 ? "-ml-2" : ""}
          data-testid="chat-header-dm-avatar-stack-more"
          style={{ zIndex: stackItemCount }}
        >
          <span className="flex h-8 w-8 items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs">
            <span className="text-2xs leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
