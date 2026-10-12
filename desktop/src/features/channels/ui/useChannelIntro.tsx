import * as React from "react";
import { Bot, FolderPlus, Plus, Sparkles, UserPlus } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  canBuildCommunity,
  selectWelcomeChannelGuide,
} from "@/features/channels/lib/welcomeChannelGuide";
import {
  mergeOpenChannelDirectory,
  useOpenChannelDirectoryQuery,
} from "@/features/channels/openChannelDirectory";
import {
  getChannelIntroDescription,
  getChannelIntroKind,
} from "@/features/channels/ui/ChannelPane.helpers";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import type { ChannelIntroChannel } from "@/features/messages/ui/ChannelIntroBlock";
import {
  isStarterWelcomeChannel,
  isWelcomeChannel,
  isWelcomeExperienceChannel,
} from "@/features/onboarding/welcome";
import { useIsProjectHomeChannel } from "@/features/projects/lib/projectHomeChannel";
import { ProjectChannelIcon } from "@/features/projects/ui/ProjectChannelIcon";
import type { Channel } from "@/shared/api/types";
import { HashSearch } from "@/shared/ui/icons";

const NO_CHANNELS: Channel[] = [];

type ChannelIntroAction = {
  description?: string;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
};

/**
 * Builds the empty-channel intro block (heading, description, action cards)
 * for the channel timeline. The Welcome channel gets its onboarding trio
 * (browse / create channel / create agent); other channels get contextual
 * member actions. #welcome-everyone also lists the community's channels
 * with what each is for, and shows the create cards only to the people who
 * build the community, so guests see where to go rather than how to build.
 */
export function useChannelIntro({
  activeChannel,
  onAddAgent,
  onAddFiles,
  onBrowseChannels,
  onCreateChannel,
  onOpenMembers,
  onWelcomeAddAgent,
}: {
  activeChannel: Channel | null;
  onAddAgent?: (options?: { beforeSend?: () => void }) => void;
  onAddFiles?: () => void;
  onBrowseChannels?: () => void;
  onCreateChannel?: () => void;
  onOpenMembers?: () => void;
  onWelcomeAddAgent?: () => void;
}) {
  const projectHome = useIsProjectHomeChannel(activeChannel?.id);
  const { goChannel } = useAppNavigation();
  const isStarterWelcome = isStarterWelcomeChannel(activeChannel);
  const canBuild = canBuildCommunity(useMyRelayMembershipLookupQuery().data);
  const memberChannels = useChannelsQuery().data ?? NO_CHANNELS;
  const directoryChannels = useOpenChannelDirectoryQuery({
    enabled: isStarterWelcome,
  }).data;
  const guideChannels = React.useMemo<ChannelIntroChannel[]>(() => {
    if (!isStarterWelcome || !activeChannel) {
      return [];
    }

    return selectWelcomeChannelGuide(
      mergeOpenChannelDirectory(memberChannels, directoryChannels),
      activeChannel.id,
    ).map((entry) => ({
      ...entry,
      onOpen: () => void goChannel(entry.id),
    }));
  }, [
    activeChannel,
    directoryChannels,
    goChannel,
    isStarterWelcome,
    memberChannels,
  ]);

  return React.useMemo(() => {
    if (!activeChannel || activeChannel.channelType === "dm") {
      return null;
    }

    const actions: ChannelIntroAction[] = [];
    if (isWelcomeExperienceChannel(activeChannel)) {
      const showsCreateActions = !isStarterWelcome || canBuild;
      if (onBrowseChannels) {
        actions.push({
          icon: <HashSearch aria-hidden className="h-6 w-6" />,
          label: "Browse channels",
          onClick: onBrowseChannels,
          testId: "welcome-intro-action-browse-channels",
        });
      }

      if (onCreateChannel && showsCreateActions) {
        actions.push({
          icon: <Plus aria-hidden className="h-6 w-6" />,
          label: "Create a channel",
          onClick: onCreateChannel,
          testId: "welcome-intro-action-create-channel",
        });
      }

      if (onWelcomeAddAgent && showsCreateActions) {
        actions.push({
          icon: <Bot aria-hidden className="h-6 w-6" />,
          label: "Create an agent",
          onClick: onWelcomeAddAgent,
          testId: "welcome-intro-action-create-agent",
        });
      }

      return {
        actions,
        channelKindLabel: isWelcomeChannel(activeChannel)
          ? "private welcome channel"
          : getChannelIntroKind(activeChannel, projectHome),
        channelName: activeChannel.name,
        channels: guideChannels,
        description: isWelcomeChannel(activeChannel)
          ? null
          : getChannelIntroDescription(activeChannel),
        icon: <Sparkles aria-hidden className="h-7 w-7" />,
      };
    }

    if (!activeChannel.archivedAt && activeChannel.isMember) {
      if (onAddFiles) {
        actions.push({
          description: "Add a repo.",
          icon: <FolderPlus aria-hidden className="h-5 w-5" />,
          label: "Add files",
          onClick: onAddFiles,
          testId: "channel-intro-action-add-files",
        });
      }

      if (onAddAgent) {
        actions.push({
          description: "Add an agent here.",
          icon: <Bot aria-hidden className="h-5 w-5" />,
          label: "Add agent",
          onClick: onAddAgent,
          testId: "channel-intro-action-create-agent",
        });
      }

      if (onOpenMembers) {
        actions.push({
          description: "Invite members.",
          icon: <UserPlus aria-hidden className="h-5 w-5" />,
          label: "Add people",
          onClick: onOpenMembers,
          testId: "channel-intro-action-add-people",
        });
      }
    }

    return {
      actions,
      channelKindLabel: getChannelIntroKind(activeChannel, projectHome),
      channelName: activeChannel.name,
      description: getChannelIntroDescription(activeChannel),
      hideBeginning: projectHome,
      icon: projectHome ? (
        <ProjectChannelIcon className="h-7 w-7" />
      ) : undefined,
    };
  }, [
    activeChannel,
    canBuild,
    guideChannels,
    isStarterWelcome,
    onAddAgent,
    onAddFiles,
    onBrowseChannels,
    onCreateChannel,
    onOpenMembers,
    onWelcomeAddAgent,
    projectHome,
  ]);
}
