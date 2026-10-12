import assert from "node:assert/strict";
import test from "node:test";

import { isChannelEditRequested } from "./channelEditRequest.ts";

test("isChannelEditRequested matches only a fresh request for the same channel", () => {
  const request = { channelId: "chan-1", requestedAt: 1_000 };

  assert.equal(isChannelEditRequested(request, "chan-1", 1_000), true);
  assert.equal(isChannelEditRequested(request, "chan-1", 10_999), true);
  assert.equal(isChannelEditRequested(request, "chan-1", 11_000), false);
  assert.equal(isChannelEditRequested(request, "chan-2", 1_000), false);
  assert.equal(isChannelEditRequested(request, null, 1_000), false);
  assert.equal(isChannelEditRequested(null, "chan-1", 1_000), false);
});
