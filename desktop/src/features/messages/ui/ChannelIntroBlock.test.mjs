import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ChannelIntroBlock } from "./ChannelIntroBlock.tsx";

test("ChannelIntroBlock preserves multi-paragraph description whitespace", () => {
  const description = "First paragraph.\n\nSecond paragraph.\nThird line.";
  const html = renderToStaticMarkup(
    React.createElement(ChannelIntroBlock, {
      intro: {
        channelKindLabel: "regular channel",
        channelName: "test",
        description,
      },
    }),
  );

  assert.match(html, /whitespace-pre-line/);
  assert.match(html, /First paragraph\.\n\nSecond paragraph\.\nThird line\./);
});

test("ChannelIntroBlock lists other channels with what each is for", () => {
  const html = renderToStaticMarkup(
    React.createElement(ChannelIntroBlock, {
      intro: {
        channelKindLabel: "regular channel",
        channelName: "welcome-everyone",
        channels: [
          {
            description: "Your Reset room. Type @Reset Worker to ask.",
            id: "reset",
            name: "craig-ask-here",
            onOpen: () => {},
          },
          {
            description: null,
            id: "general",
            name: "general",
            onOpen: () => {},
          },
        ],
      },
    }),
  );

  assert.match(html, /Channels in this community/);
  assert.match(html, /data-testid="channel-intro-channel-craig-ask-here"/);
  assert.match(html, /#craig-ask-here/);
  assert.match(html, /Your Reset room\. Type @Reset Worker to ask\./);
  assert.match(html, /data-testid="channel-intro-channel-general"/);
});

test("ChannelIntroBlock omits the channel list when there is nothing to list", () => {
  const html = renderToStaticMarkup(
    React.createElement(ChannelIntroBlock, {
      intro: {
        channelKindLabel: "regular channel",
        channelName: "welcome-everyone",
        channels: [],
      },
    }),
  );

  assert.doesNotMatch(html, /Channels in this community/);
});
