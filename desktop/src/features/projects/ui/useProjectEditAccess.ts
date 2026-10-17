import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import type { Project } from "@/features/projects/hooks";

/**
 * Who may republish an announced project: its owner, a managed agent owner
 * on this computer, or the person who owns the owning agent (who signs
 * through that agent, `ownerControlAgentPubkey`).
 */
export function useProjectEditAccess(
  project: Project,
  identityPubkey: string | undefined,
): { canEdit: boolean; ownerControlAgentPubkey: string | undefined } {
  const ownerProfileQuery = useUsersBatchQuery([project.owner], {
    enabled: Boolean(identityPubkey),
  });
  const projectOwnerProfile =
    ownerProfileQuery.data?.profiles[project.owner.toLowerCase()];
  const projectOwnerIsManaged = useIsManagedAgent(project.owner) === true;
  const viewerIsProjectOwner =
    identityPubkey?.toLowerCase() === project.owner.toLowerCase();
  const viewerOwnsProjectAgent = ownsAuthorAgent(
    projectOwnerProfile,
    identityPubkey,
  );
  return {
    canEdit:
      !project.legacy &&
      (viewerIsProjectOwner || projectOwnerIsManaged || viewerOwnsProjectAgent),
    ownerControlAgentPubkey:
      viewerOwnsProjectAgent && !projectOwnerIsManaged && !viewerIsProjectOwner
        ? project.owner
        : undefined,
  };
}
