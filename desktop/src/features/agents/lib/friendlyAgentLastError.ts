/**
 * Promote certain machine-readable `lastError` strings to user-facing copy.
 *
 * The error classification seam flows like this:
 *   buzz-agent — classifies LLM failures into `AgentError` variants with
 *                  JSON-RPC codes (`-32001` auth, `-32002` model-not-found,
 *                  `-32000` generic), defined in `crates/buzz-agent/src/types.rs`.
 *   buzz-acp   — preserves the code structurally in
 *                  `AcpError::AgentError { code, message }`, whose Display is
 *                  `"Agent reported error (code N): message"`, and includes
 *                  `code` in `turn_error` observer events.
 *   desktop supervisor — on nonzero exit, recovers `{ message, code }` from
 *                  the log tail (`managed_agents/storage.rs`) into
 *                  `ManagedAgent.lastError` / `lastErrorCode`.
 *
 * This function dispatches on the numeric code first (works for any harness),
 * then recovers a code embedded in the message string (handles records where
 * the `lastErrorCode` field was lost, e.g. downgrade or pre-code records with
 * new-format strings), then falls back to legacy string prefixes for records
 * written before structured codes existed.
 *
 * Returns:
 *  - null when there's nothing to show (null/empty lastError).
 *  - A `{ severity: "denied"; copy: string }` object for the auth-failure
 *    and model-not-found cases, so the UI can render with the right visual
 *    weight (destructive).
 *  - A `{ severity: "generic"; copy: string }` pass-through for any other
 *    lastError, so generic harness exits still surface their text instead of
 *    being swallowed.
 */
export type FriendlyAgentLastError =
  | { severity: "denied"; copy: string }
  | { severity: "generic"; copy: string };

/**
 * The exact copy for the relay-mesh denial. Centralized as a constant so the
 * test asserts the user-facing string verbatim rather than a fuzzy pattern.
 */
export const RELAY_MESH_DENIED_COPY =
  "Community access denied this agent — check its community membership.";

export const MODEL_NOT_FOUND_COPY =
  "The configured model is not available — open agent settings and select a different one from the dropdown.";

/**
 * buzz-acp now folds the adapter's detail (codex-acp's `data.message`) into
 * the message, so a bare "Internal error" really carried no detail; it is
 * not a hint that a Codex model is unsupported.
 */
export const CLI_ACP_INTERNAL_ERROR_COPY =
  "The agent's adapter reported an internal error and gave no detail. The agent's log may say more.";

const EMBEDDED_CODE_RE = /^Agent reported error \(code (-?\d+)\): /;
/** Bare form of the standard JSON-RPC -32603 message (after stripping the ACP wrapper prefix). */
const BARE_INTERNAL_ERROR = "Internal error";

/**
 * The Claude SDK's own usage-limit prefixes (`USAGE_LIMIT_ERROR_PREFIXES`),
 * as `crates/buzz-acp/src/factory.rs` reads them: text that means a limit
 * was reached, with no secrets in it, so it is shown word for word.
 */
const LIMIT_PREFIXES = [
  "You've hit your",
  "You've reached your",
  "You're out of usage credits",
  "Your org is out of usage · add funds to continue",
  "Your org is out of usage · contact your admin",
  "Your seat type doesn't include usage credits",
  "Your seat type doesn't include usage",
  "Your usage allocation has been disabled by your admin",
  "Your group's usage limit is set to $0",
  "Fable 5 requires usage credits",
  "You're out of extra usage",
  "Your seat type doesn't include extra usage",
];
/** buzz-acp's ` (errorKind: …)` / ` (codexErrorInfo: …)` markers at the end of a message. */
const ADAPTER_MARKER_RE = / \((errorKind|codexErrorInfo): ([^)]*)\)$/;

/**
 * "Paused: Claude weekly limit · resets Oct 18, 7pm (America/Chicago)" for a
 * usage or spend limit the harness parks work behind, in the provider's own
 * words; null for any other error.
 */
export function pausedLimitCopy(message: string): string | null {
  let text = message.trim().replace(/^Internal error: /, "");
  let provider = "Claude";
  let codexLimit = false;
  for (
    let marker = ADAPTER_MARKER_RE.exec(text);
    marker;
    marker = ADAPTER_MARKER_RE.exec(text)
  ) {
    if (marker[1] === "codexErrorInfo") {
      provider = "Codex";
      codexLimit ||= marker[2] === "usageLimitExceeded";
    }
    text = text.slice(0, marker.index);
  }
  if (!codexLimit && !LIMIT_PREFIXES.some((p) => text.startsWith(p))) {
    return null;
  }
  const hit = /^You've (?:hit|reached) your /.exec(text);
  return hit
    ? `Paused: ${provider} ${text.slice(hit[0].length)}`
    : `Paused: ${provider}: ${text}`;
}

/**
 * The error of the agent's newest finished turn, or null when that turn
 * succeeded. A harness that parks work behind a limit keeps running, so the
 * `lastError` recorded at exit stays empty; the turn frames carry the error.
 */
export function latestTurnError(
  events: readonly { kind: string; payload: unknown }[],
): { error: string; code: number | null } | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const { kind, payload } = events[i];
    if (kind === "turn_completed") return null;
    if (kind !== "turn_error") continue;
    const { error, code } = (payload ?? {}) as {
      error?: unknown;
      code?: unknown;
    };
    if (typeof error !== "string") return null;
    const numeric = code == null ? null : Number(code);
    return {
      error,
      code: numeric != null && Number.isFinite(numeric) ? numeric : null,
    };
  }
  return null;
}

function recoverEmbeddedCode(trimmed: string): {
  code: number;
  remainder: string;
} | null {
  const match = EMBEDDED_CODE_RE.exec(trimmed);
  if (!match) return null;
  return {
    code: Number(match[1]),
    remainder: trimmed.slice(match[0].length),
  };
}

export function friendlyAgentLastError(
  raw: string | null,
  code?: number | null,
): FriendlyAgentLastError | null {
  if (raw == null) return null;
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;

  // Structured code first; a code embedded in the message string is the
  // same signal recovered from a record that lost the field.
  const embedded = recoverEmbeddedCode(trimmed);
  const effectiveCode = Number.isFinite(code)
    ? (code as number)
    : (embedded?.code ?? null);
  if (effectiveCode != null) {
    switch (effectiveCode) {
      case -32001:
        return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
      case -32002:
        return { severity: "denied", copy: MODEL_NOT_FOUND_COPY };
      case -32603: {
        // Standard JSON-RPC "Internal error": the adapters report usage and
        // spend limits this way, and other failures with their detail after
        // it. Only the bare "Internal error" form gets the no-detail copy;
        // specific detail is preserved so actionable information is not
        // buried.
        //
        // "Bare" means the remainder after stripping the ACP wrapper prefix
        // (if present) is exactly "Internal error". This covers both the raw
        // form ("Internal error") and the ACP-wrapped form
        // ("Agent reported error (code -32603): Internal error").
        const remainder = embedded?.remainder ?? trimmed;
        if (remainder === BARE_INTERNAL_ERROR) {
          return { severity: "generic", copy: CLI_ACP_INTERNAL_ERROR_COPY };
        }
        return {
          severity: "generic",
          copy: pausedLimitCopy(remainder) ?? remainder,
        };
      }
    }
    // A structured code we don't recognize is authoritative — don't let
    // string patterns cross-classify it.
    return { severity: "generic", copy: trimmed };
  }

  // Legacy string fallback for records written before codes existed.
  // Match either the unwrapped buzz-agent prefix or the buzz-acp v0 wrap.
  if (
    trimmed.startsWith("Agent reported error: llm auth:") ||
    trimmed.startsWith("llm auth:")
  ) {
    return { severity: "denied", copy: RELAY_MESH_DENIED_COPY };
  }

  return { severity: "generic", copy: trimmed };
}

/**
 * Convenience for `turn_error` / `agent_panic` observer payloads: coerce the
 * payload's untyped `code` JSON value and return the display copy, falling
 * back to the raw error text when no classification applies.
 */
export function friendlyTurnErrorCopy(raw: string, code: unknown): string {
  const numeric = code == null ? null : Number(code);
  const safe = Number.isFinite(numeric) ? (numeric as number) : null;
  return friendlyAgentLastError(raw, safe)?.copy ?? raw;
}
