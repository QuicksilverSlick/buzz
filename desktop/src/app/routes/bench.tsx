import { createFileRoute } from "@tanstack/react-router";

import { BenchScreen } from "@/features/bench/ui/BenchScreen";

// Not lazy: BenchCard already ships in the main chunk with the timeline.
export const Route = createFileRoute("/bench")({
  // `card`: a card to scroll to and focus, from global search.
  validateSearch: (search: Record<string, unknown>): { card?: string } => ({
    card:
      typeof search.card === "string" && search.card ? search.card : undefined,
  }),
  component: BenchRoute,
});

function BenchRoute() {
  return <BenchScreen focusCardId={Route.useSearch().card} />;
}
