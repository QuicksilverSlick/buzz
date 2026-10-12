import * as React from "react";

/**
 * A one-shot request to open a channel's edit dialog. The channel header's
 * "Add a description" action sets it; the channel settings panel takes it once
 * it is open and knows the viewer may edit. Requests expire, so settings
 * opened much later never jump straight into editing.
 */
export type ChannelEditRequest = {
  channelId: string;
  requestedAt: number;
};

const CHANNEL_EDIT_REQUEST_TTL_MS = 10_000;

let pendingRequest: ChannelEditRequest | null = null;
const listeners = new Set<() => void>();

function publish(next: ChannelEditRequest | null) {
  pendingRequest = next;
  for (const listener of listeners) listener();
}

export function requestChannelEdit(channelId: string, now = Date.now()) {
  publish({ channelId, requestedAt: now });
}

export function clearChannelEditRequest(channelId: string) {
  if (pendingRequest?.channelId === channelId) publish(null);
}

/** Whether `request` asks to edit `channelId` and has not expired. */
export function isChannelEditRequested(
  request: ChannelEditRequest | null,
  channelId: string | null,
  now = Date.now(),
): boolean {
  return (
    request !== null &&
    channelId !== null &&
    request.channelId === channelId &&
    now - request.requestedAt < CHANNEL_EDIT_REQUEST_TTL_MS
  );
}

export function useChannelEditRequest() {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => pendingRequest,
  );
}
