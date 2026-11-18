import assert from "node:assert/strict";
import test from "node:test";

const SELF = "aa".repeat(32);
const CHRIS = "bb".repeat(32);
const TALOS = "cc".repeat(32);
const AGENT = "dd".repeat(32);
const RELAY = "wss://relay.example";

const calls = [];
let openDmError = null;
const rawAgent = {
  pubkey: AGENT,
  name: "Scout",
  persona_id: null,
  relay_url: RELAY,
  acp_command: "acp",
  agent_command: "agent",
  agent_args: [],
  mcp_command: "mcp",
  turn_timeout_seconds: 60,
  idle_timeout_seconds: 60,
  max_turn_duration_seconds: 60,
  parallelism: 1,
  system_prompt: null,
  model: null,
  status: "running",
  pid: 1,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  last_started_at: null,
  last_stopped_at: null,
  last_exit_code: null,
  last_error: null,
  log_path: "/tmp/log",
  start_on_app_launch: false,
  backend: { type: "local" },
  backend_agent_id: null,
};
const tauriMock = {
  invoke(command, args) {
    calls.push({ command, args });
    if (command === "open_dm") {
      if (openDmError) return Promise.reject(new Error(openDmError));
      return Promise.resolve({
        id: "new-dm",
        name: "",
        channel_type: "dm",
        visibility: "private",
        description: "",
        topic: null,
        purpose: null,
        member_count: args.pubkeys.length + 1,
        member_pubkeys: [SELF, ...args.pubkeys],
        last_message_at: null,
        archived_at: null,
        participants: [],
        participant_pubkeys: [SELF, ...args.pubkeys],
        is_member: true,
        ttl_seconds: null,
        ttl_deadline: null,
      });
    }
    if (command === "add_channel_members") {
      return Promise.resolve({ added: [AGENT], errors: [] });
    }
    if (command === "start_managed_agent") return Promise.resolve(rawAgent);
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback() {
    return Math.random();
  },
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

const React = (await import("react")).default;
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { toast } = await import("sonner");
const { useCreatedAgentChannelAttachment } = await import(
  "./useCreatedAgentChannelAttachment.ts"
);
const { CommunitiesProvider } = await import(
  "../communities/useCommunities.tsx"
);
const { saveActiveCommunityId, saveCommunities } = await import(
  "../communities/communityStorage.ts"
);

saveCommunities([{ id: "a", name: "A", relayUrl: RELAY }]);
saveActiveCommunityId("a");

// sonner's `toast` is one shared object, so these replacements see the hook's calls.
const toasts = [];
for (const kind of ["success", "warning", "loading"]) {
  toast[kind] = (title, options) => {
    toasts.push({ kind, title, options });
    return options?.id ?? toasts.length;
  };
}

function channel(id, channelType, participantPubkeys) {
  return {
    id,
    name: id,
    channelType,
    visibility: channelType === "dm" ? "private" : "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: participantPubkeys.length,
    memberPubkeys: participantPubkeys,
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys,
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

function created() {
  return {
    agent: {
      pubkey: AGENT,
      name: "Scout",
      status: "stopped",
      backend: { type: "local" },
    },
    privateKeyNsec: "",
    profileSyncError: null,
    spawnError: null,
  };
}

async function present(targetChannel, channels) {
  calls.length = 0;
  toasts.length = 0;
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.setQueryData(["identity"], { pubkey: SELF });
  queryClient.setQueryData(["channels"], channels);
  const root = createRoot(document.createElement("div"));
  const latest = {};
  function Harness() {
    Object.assign(latest, useCreatedAgentChannelAttachment());
    return null;
  }
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(Harness),
        ),
      ),
    );
  });
  await act(async () => {
    await latest.presentCreatedAgent(created(), {
      id: targetChannel.id,
      name: targetChannel.name,
    });
  });
  return { unmount: () => act(async () => root.unmount()) };
}

const commands = (name) => calls.filter((call) => call.command === name);

test("an agent made in a DM joins a new DM of the same people and starts", async () => {
  const dm = channel("dm-1", "dm", [SELF, CHRIS, TALOS]);
  const { unmount } = await present(dm, [dm]);
  try {
    assert.equal(commands("open_dm").length, 1);
    assert.deepEqual(commands("open_dm")[0].args.pubkeys, [
      CHRIS,
      TALOS,
      AGENT,
    ]);
    // Tenant scope rides along, so a switch mid-flight fails closed.
    assert.equal(commands("open_dm")[0].args.expectedRelayUrl, RELAY);
    assert.equal(commands("open_dm")[0].args.expectedSignerPubkey, SELF);
    assert.equal(commands("add_channel_members").length, 0);
    assert.equal(commands("start_managed_agent").length, 1);
    assert.equal(toasts.length, 1);
    assert.equal(toasts[0].kind, "success");
    assert.equal(
      toasts[0].options?.description,
      "Scout is in a new DM with the same people.",
    );
  } finally {
    await unmount();
  }
});

test("an agent made in a stream is added to it as a bot", async () => {
  const stream = channel("general", "stream", [SELF, CHRIS]);
  const { unmount } = await present(stream, [stream]);
  try {
    assert.equal(commands("add_channel_members").length, 1);
    assert.deepEqual(commands("add_channel_members")[0].args, {
      channelId: "general",
      pubkeys: [AGENT],
      role: "bot",
    });
    assert.equal(commands("open_dm").length, 0);
  } finally {
    await unmount();
  }
});

test("a failed DM open keeps the agent created, starts nothing, and offers Try again", async () => {
  openDmError = "relay offline";
  const dm = channel("dm-1", "dm", [SELF, CHRIS]);
  const { unmount } = await present(dm, [dm]);
  try {
    assert.equal(commands("open_dm").length, 1);
    assert.equal(commands("start_managed_agent").length, 0);
    assert.equal(toasts.length, 1);
    assert.equal(toasts[0].kind, "warning");
    assert.equal(
      toasts[0].options.description,
      "Scout couldn’t be added to a new DM. relay offline",
    );
    assert.equal(toasts[0].options.action?.label, "Try again");

    openDmError = null;
    await act(async () => {
      toasts[0].options.action.onClick({ preventDefault() {} });
      for (let i = 0; i < 20 && toasts.at(-1).kind !== "success"; i += 1) {
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    });
    assert.equal(commands("open_dm").length, 2);
    assert.deepEqual(commands("open_dm")[1].args.pubkeys, [CHRIS, AGENT]);
    assert.equal(commands("start_managed_agent").length, 1);
    assert.equal(
      toasts.at(-1).options.description,
      "Scout is in a new DM with the same people.",
    );
  } finally {
    openDmError = null;
    await unmount();
  }
});

test("a full 9-person DM says so and offers no Try again", async () => {
  const people = [
    SELF,
    ...Array.from({ length: 8 }, (_, i) => `${i}`.repeat(64)),
  ];
  const dm = channel("dm-full", "dm", people);
  const { unmount } = await present(dm, [dm]);
  try {
    assert.equal(commands("open_dm").length, 0);
    assert.equal(commands("start_managed_agent").length, 0);
    assert.equal(toasts.length, 1);
    assert.equal(toasts[0].kind, "warning");
    assert.equal(
      toasts[0].options.description,
      "Scout couldn’t be added to a new DM. This DM already has 9 people, the most a DM can hold. Add Scout to a channel instead.",
    );
    assert.equal(toasts[0].options.action, undefined);
  } finally {
    await unmount();
  }
});
