import assert from "node:assert/strict";
import test from "node:test";

import { deleteChannel } from "./tauriChannels.ts";

const ARCHIVED = "relay returned 400 Bad Request: invalid: channel is archived";

// Each command pops its next scripted outcome: undefined resolves, a string
// rejects the way a Rust command error does.
async function runDelete(script) {
  const calls = [];
  const previousWindow = globalThis.window;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command, args) {
        calls.push(command);
        assert.deepEqual(args, { channelId: "c1" });
        const outcome = script[command]?.shift();
        return outcome === undefined
          ? Promise.resolve(null)
          : Promise.reject(outcome);
      },
    },
  };
  try {
    const error = await deleteChannel("c1").then(
      () => null,
      (caught) => caught,
    );
    return { calls, error };
  } finally {
    globalThis.window = previousWindow;
  }
}

test("deleteChannel sends one delete for a live channel", async () => {
  const { calls, error } = await runDelete({});
  assert.equal(error, null);
  assert.deepEqual(calls, ["delete_channel"]);
});

test("deleteChannel passes an unrelated refusal through unchanged", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [
      "relay returned 403 Forbidden: only owner can delete group",
    ],
  });
  assert.equal(
    error.message,
    "relay returned 403 Forbidden: only owner can delete group",
  );
  assert.deepEqual(calls, ["delete_channel"]);
});

test("deleteChannel restores an archived channel, then deletes it", async () => {
  const { calls, error } = await runDelete({ delete_channel: [ARCHIVED] });
  assert.equal(error, null);
  assert.deepEqual(calls, [
    "delete_channel",
    "unarchive_channel",
    "delete_channel",
  ]);
});

test("deleteChannel stops when the archived channel cannot be restored", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [ARCHIVED],
    unarchive_channel: ["relay returned 403 Forbidden: not an admin"],
  });
  assert.match(
    error.message,
    /could not be restored.*not an admin.*Nothing was changed/,
  );
  assert.deepEqual(calls, ["delete_channel", "unarchive_channel"]);
});

test("deleteChannel re-archives when the unarchive outcome is unknown", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [ARCHIVED],
    unarchive_channel: ["relay unreachable: network error"],
  });
  assert.doesNotMatch(error.message, /Nothing was changed/);
  assert.match(error.message, /network error\. It is still archived\./);
  assert.deepEqual(calls, [
    "delete_channel",
    "unarchive_channel",
    "archive_channel",
  ]);
});

test("deleteChannel treats an already-archived refusal as still archived", async () => {
  const { error } = await runDelete({
    delete_channel: [ARCHIVED],
    unarchive_channel: ["relay returned 504 Gateway Timeout"],
    archive_channel: [
      "relay returned 400 Bad Request: invalid: channel is already archived",
    ],
  });
  assert.match(error.message, /504 Gateway Timeout\. It is still archived\./);
});

test("deleteChannel reports a likely delete when the re-archive finds no channel", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [ARCHIVED, "relay unreachable: network error"],
    archive_channel: [
      "relay returned 400 Bad Request: invalid: channel not found",
    ],
  });
  assert.doesNotMatch(error.message, /channel settings/);
  assert.match(error.message, /appears to be deleted/);
  assert.deepEqual(calls, [
    "delete_channel",
    "unarchive_channel",
    "delete_channel",
    "archive_channel",
  ]);
});

test("deleteChannel re-archives when the second delete fails", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [ARCHIVED, "relay returned 500 Internal Server Error"],
  });
  assert.match(
    error.message,
    /500 Internal Server Error\. It is still archived\./,
  );
  assert.deepEqual(calls, [
    "delete_channel",
    "unarchive_channel",
    "delete_channel",
    "archive_channel",
  ]);
});

test("deleteChannel says so when the re-archive also fails", async () => {
  const { calls, error } = await runDelete({
    delete_channel: [ARCHIVED, "relay returned 500 Internal Server Error"],
    archive_channel: ["relay returned 502 Bad Gateway"],
  });
  assert.match(
    error.message,
    /could not be archived again \(relay returned 502 Bad Gateway\)/,
  );
  assert.deepEqual(calls, [
    "delete_channel",
    "unarchive_channel",
    "delete_channel",
    "archive_channel",
  ]);
});
