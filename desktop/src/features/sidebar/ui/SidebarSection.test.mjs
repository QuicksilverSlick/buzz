import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  formatWorkingTooltip,
  getChannelRowTooltipDetail,
} from "./SidebarSection.tsx";

function summary(agentNames, agentCount = agentNames.length) {
  return {
    channelId: "chan-1",
    anchorAt: 0,
    agentCount,
    agentPubkeys: Array.from(
      { length: agentCount },
      (_, index) => `agent-${index}-pubkey`,
    ),
    agentNames,
  };
}

describe("formatWorkingTooltip", () => {
  it("names one known agent", () => {
    assert.equal(formatWorkingTooltip(summary(["Ned"])), "Ned working");
  });

  it("names one known agent and counts one additional agent", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart"])),
      "Ned and 1 agent working",
    );
  });

  it("names one known agent and counts multiple additional agents", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl"])),
      "Ned and 2 agents working",
    );
  });

  it("uses a singular count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 1)), "1 agent working");
  });

  it("uses a plural count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 3)), "3 agents working");
  });

  it("counts unknown agents with the named lead", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned"], 3)),
      "Ned and 2 agents working",
    );
  });
});

describe("getChannelRowTooltipDetail", () => {
  function channel(overrides = {}) {
    return {
      archivedAt: null,
      channelType: "stream",
      description: "",
      isMember: true,
      purpose: null,
      topic: null,
      ...overrides,
    };
  }

  it("shows a stream's or forum's description", () => {
    assert.equal(
      getChannelRowTooltipDetail(channel({ description: "Ask here." }), false),
      "Ask here.",
    );
    assert.equal(
      getChannelRowTooltipDetail(
        channel({ channelType: "forum", description: "Long posts." }),
        false,
      ),
      "Long posts.",
    );
  });

  it("falls back to topic, then purpose, like the header", () => {
    assert.equal(
      getChannelRowTooltipDetail(
        channel({ topic: "Topic", purpose: "P" }),
        false,
      ),
      "Topic",
    );
    assert.equal(
      getChannelRowTooltipDetail(channel({ purpose: "Purpose" }), false),
      "Purpose",
    );
  });

  it("has nothing for undescribed channels and direct messages", () => {
    assert.equal(getChannelRowTooltipDetail(channel(), false), null);
    assert.equal(
      getChannelRowTooltipDetail(
        channel({ channelType: "dm", description: "Hi" }),
        false,
      ),
      null,
    );
  });

  it("yields to the activity preview on busy rows", () => {
    assert.equal(
      getChannelRowTooltipDetail(channel({ description: "Ask here." }), true),
      null,
    );
  });
});
