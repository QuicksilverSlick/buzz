import * as React from "react";

import { useCanvasQuery } from "@/features/channels/hooks";
import {
  hasChannelCanvasContent,
  hasSeenChannelAbout,
  markChannelAboutSeen,
  shouldAutoOpenChannelAbout,
} from "@/features/channels/lib/channelAbout";
import { isWelcomeChannel } from "@/features/onboarding/welcome";
import type { Channel } from "@/shared/api/types";
import type { PanelSetterOptions } from "./useChannelPanelHistoryState";

type UseChannelAboutOptions = {
  activeChannel: Channel | null;
  /**
   * The window is wide enough to show the panel beside the messages. A narrow
   * window would swap the messages out, so About never opens there by itself.
   */
  canOpenBesideChannel: boolean;
  channelAboutOpen: boolean;
  /** False where the channel shows no header, such as a huddle transcript. */
  enabled: boolean;
  /** Closes the thread, profile and agent activity panels, then runs `open`. */
  openSidePanel: (open: () => void) => void;
  /** A thread, profile, agent activity or settings panel holds the side panel. */
  otherPanelOpen: boolean;
  requireThreadEditResolution: () => boolean;
  setChannelAboutOpen: (open: boolean, options?: PanelSetterOptions) => void;
};

/**
 * Drives the header's "About this channel" button: whether the channel has a
 * canvas to show, opening it once by itself on this device's first visit, and
 * toggling it from the button. Only stream channels have it.
 */
export function useChannelAbout({
  activeChannel,
  canOpenBesideChannel,
  channelAboutOpen,
  enabled,
  openSidePanel,
  otherPanelOpen,
  requireThreadEditResolution,
  setChannelAboutOpen,
}: UseChannelAboutOptions) {
  const channelId =
    enabled && activeChannel?.channelType === "stream"
      ? activeChannel.id
      : null;
  const canvasQuery = useCanvasQuery(channelId, channelId !== null);
  const hasChannelAbout =
    channelId !== null && hasChannelCanvasContent(canvasQuery.data);
  // The private Welcome channel guides new people with its own kickoff; its
  // canvas stays one click away instead of opening over that.
  const canAutoOpen = channelId !== null && !isWelcomeChannel(activeChannel);
  const canvasLoaded = canvasQuery.isSuccess;

  // Decide once per visit, when the canvas first answers. Opening or closing
  // other panels later in the same visit never brings About back by itself.
  const decidedChannelIdRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (channelId === null || !canvasLoaded) return;
    if (decidedChannelIdRef.current === channelId) return;
    decidedChannelIdRef.current = channelId;
    if (channelAboutOpen) {
      markChannelAboutSeen(channelId);
      return;
    }
    if (
      !canAutoOpen ||
      !canOpenBesideChannel ||
      !shouldAutoOpenChannelAbout({
        hasContent: hasChannelAbout,
        otherPanelOpen,
        seen: hasSeenChannelAbout(channelId),
      })
    ) {
      return;
    }
    markChannelAboutSeen(channelId);
    setChannelAboutOpen(true, { replace: true });
  }, [
    canAutoOpen,
    canOpenBesideChannel,
    canvasLoaded,
    channelAboutOpen,
    channelId,
    hasChannelAbout,
    otherPanelOpen,
    setChannelAboutOpen,
  ]);

  const toggleChannelAbout = React.useCallback(() => {
    if (!requireThreadEditResolution()) return;
    if (channelAboutOpen) {
      setChannelAboutOpen(false);
      return;
    }
    if (channelId) markChannelAboutSeen(channelId);
    openSidePanel(() => setChannelAboutOpen(true));
  }, [
    channelAboutOpen,
    channelId,
    openSidePanel,
    requireThreadEditResolution,
    setChannelAboutOpen,
  ]);

  return { hasChannelAbout, toggleChannelAbout };
}
