import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { usePersonasQuery } from "@/features/agents/hooks";
import { useViewerAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { useBenchSnapshot } from "@/features/bench/useBenchCards";
import { withoutOwnWorkflowPosts } from "@/features/home/hooks";
import { buildInboxItems } from "@/features/home/lib/inbox";
import {
  classifyInboxItem,
  inboxProjects,
} from "@/features/home/lib/inboxClass";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useProjectsQuery } from "@/features/projects/hooks";
import type { Project } from "@/features/projects/projectModels";
import type { ProjectsWorkItemsResult } from "@/features/projects/projectWorkItems";
import {
  inboxHits,
  personaHits,
  projectHits,
  type SurfaceHit,
} from "@/features/search/lib/surfaceSearch";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, HomeFeedResponse } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";

/**
 * Global search hits from the other screens' warm caches. Reads only: it
 * never starts a relay fetch (the agent library is a local read), so a
 * screen not visited this session adds nothing.
 */
export function useSurfaceSearchResults({
  channels,
  enabled,
  query,
}: {
  channels: Channel[];
  enabled: boolean;
  query: string;
}): SurfaceHit[] {
  const queryClient = useQueryClient();
  const projectsOn = useFeatureEnabled("projects");
  const viewer = useIdentityQuery().data?.pubkey;
  const relaySelf = useRelaySelfQuery().data;
  const viewerAgents = useViewerAgentPubkeys();
  const benchSnapshot = useBenchSnapshot();
  const projects = useProjectsQuery(false).data;
  const personas = usePersonasQuery({ enabled }).data;

  return React.useMemo(() => {
    if (!enabled) return [];
    // Read, not subscribe: an observer would push its options onto the
    // screens' own queries.
    const cached = <T>(...queryKey: string[]) =>
      queryClient
        .getQueriesData<T>({ queryKey })
        .flatMap(([, data]) => (data ? [data] : []));
    const [feed] = cached<HomeFeedResponse>("home-feed");
    const items = buildInboxItems({
      channels,
      currentPubkey: viewer,
      feed: feed && withoutOwnWorkflowPosts(feed, viewer, relaySelf),
      relaySelfPubkey: relaySelf,
    });
    const chips = inboxProjects(projects ?? [], viewer, viewerAgents);
    return [
      ...personaHits(personas ?? [], query),
      ...inboxHits(
        items,
        (item) => classifyInboxItem(item, benchSnapshot, chips),
        query,
      ),
      ...(projectsOn
        ? projectHits(
            projects ?? [],
            cached<ProjectsWorkItemsResult<Project>>("projects", "work-items"),
            query,
          )
        : []),
    ];
  }, [
    benchSnapshot,
    channels,
    enabled,
    personas,
    projects,
    projectsOn,
    query,
    queryClient,
    relaySelf,
    viewer,
    viewerAgents,
  ]);
}
