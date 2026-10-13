import { createFileRoute } from "@tanstack/react-router";

import { BenchScreen } from "@/features/bench/ui/BenchScreen";

// Not lazy: BenchCard already ships in the main chunk with the timeline.
export const Route = createFileRoute("/bench")({
  component: BenchScreen,
});
