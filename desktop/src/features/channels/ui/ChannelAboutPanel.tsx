import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import type { Channel } from "@/shared/api/types";
import { KeyedChannelCanvas } from "./KeyedChannelCanvas";

export const CHANNEL_ABOUT_TITLE = "About this channel";

type ChannelAboutPanelProps = {
  channel: Channel;
  isSinglePanelView: boolean;
  layout: "split" | "standalone";
  onClose: () => void;
  transparentChrome?: boolean;
  widthPx: number;
};

/**
 * The channel's canvas, read-only, in the channel side panel. It reuses the
 * renderer channel settings shows; editing stays in channel settings.
 */
export function ChannelAboutPanel({
  channel,
  isSinglePanelView,
  layout,
  onClose,
  transparentChrome = false,
  widthPx,
}: ChannelAboutPanelProps) {
  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="channel-about-standalone-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader inset={layout !== "split" ? "wide" : "default"}>
          <AuxiliaryPanelHeaderGroup>
            <AuxiliaryPanelTitle>{CHANNEL_ABOUT_TITLE}</AuxiliaryPanelTitle>
          </AuxiliaryPanelHeaderGroup>
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        className="overflow-y-auto overflow-x-hidden overscroll-contain px-4 pb-8"
        panelPadding
      >
        <div className="pt-3" data-testid="channel-about-panel">
          <KeyedChannelCanvas
            canEdit={false}
            channelId={channel.id}
            isArchived={channel.archivedAt !== null}
          />
        </div>
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
