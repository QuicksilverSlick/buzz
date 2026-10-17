import { useObserverEvents } from "@/features/agents/ui/useObserverEvents";
import type { ManagedAgent } from "@/shared/api/types";
import {
  friendlyAgentLastError,
  latestTurnError,
} from "./friendlyAgentLastError";

/**
 * The agent's last error for its panel. While it runs, the newest turn's
 * error: a harness paused on a usage limit never exits, so the `lastError`
 * recorded at exit says nothing then. Otherwise the error it exited with.
 */
export function useAgentLastError(
  agent:
    | Pick<ManagedAgent, "pubkey" | "status" | "lastError" | "lastErrorCode">
    | undefined,
) {
  const running = agent?.status === "running";
  const { events } = useObserverEvents(running, agent?.pubkey);
  const turn = running ? latestTurnError(events) : null;
  return turn
    ? friendlyAgentLastError(turn.error, turn.code)
    : friendlyAgentLastError(agent?.lastError ?? null, agent?.lastErrorCode);
}
