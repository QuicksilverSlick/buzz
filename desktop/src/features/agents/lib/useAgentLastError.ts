import { useObserverEvents } from "@/features/agents/ui/useObserverEvents";
import type { ManagedAgent } from "@/shared/api/types";
import {
  friendlyAgentLastError,
  latestTurnError,
  pausedStateCopy,
} from "./friendlyAgentLastError";

/**
 * The agent's last error for its panel, newest signal first.
 *
 * While it runs, the newest turn's error: a harness paused on a usage limit
 * never exits, so the `lastError` recorded at exit says nothing then. Next
 * the harness's own durable paused state, which is what the panel reads for
 * most of a multi-day outage — the observer event window the turn error
 * comes from is capped, trimmed once it fills, and empty after a restart,
 * and `lastError` stays null for exactly this case. Otherwise the error it
 * exited with.
 */
export function useAgentLastError(
  agent:
    | Pick<
        ManagedAgent,
        "pubkey" | "status" | "lastError" | "lastErrorCode" | "paused"
      >
    | undefined,
) {
  const running = agent?.status === "running";
  const { events } = useObserverEvents(running, agent?.pubkey);
  const turn = running ? latestTurnError(events) : null;
  if (turn) return friendlyAgentLastError(turn.error, turn.code);
  const paused = agent?.paused ? pausedStateCopy(agent.paused) : null;
  if (paused) return paused;
  return friendlyAgentLastError(agent?.lastError ?? null, agent?.lastErrorCode);
}
