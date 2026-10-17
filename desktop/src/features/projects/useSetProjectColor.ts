import { useMutation, useQueryClient } from "@tanstack/react-query";

import { type Project, projectsQueryKey } from "@/features/projects/hooks";
import { publishOwnedAgentProjectAnnouncements } from "@/features/projects/projectOwnerControl";
import { buildProjectColorPatchTemplate } from "@/features/projects/projectRepositoryCreation";
import { markProjectDataAuthoritative } from "@/features/projects/projectSnapshot";
import { publishProjectOwnerAnnouncement } from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";

export type SetProjectColorInput = {
  color: string;
  ownerControlAgentPubkey?: string;
  project: Project;
};

/** Republishes the live project head with a new `color` tag. */
export async function setProjectColor(
  { color, ownerControlAgentPubkey, project }: SetProjectColorInput,
  {
    fetchEvents = relayClient.fetchEvents.bind(relayClient),
    publishOwnedAgentAnnouncements = publishOwnedAgentProjectAnnouncements,
    publishOwnerAnnouncement = publishProjectOwnerAnnouncement,
  }: {
    fetchEvents?: typeof relayClient.fetchEvents;
    publishOwnedAgentAnnouncements?: typeof publishOwnedAgentProjectAnnouncements;
    publishOwnerAnnouncement?: typeof publishProjectOwnerAnnouncement;
  } = {},
): Promise<Project> {
  const targetOwner = project.owner.toLowerCase();
  // The agent signs whatever it is sent: only the project's own owner may.
  if (
    ownerControlAgentPubkey &&
    ownerControlAgentPubkey.toLowerCase() !== targetOwner
  ) {
    throw new Error(
      "This project belongs to another owner. Refresh and try again.",
    );
  }
  const [liveHead] = await fetchEvents({
    kinds: [KIND_PROJECT_ANNOUNCEMENT],
    authors: [targetOwner],
    "#d": [project.dtag],
    limit: 1,
  });
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }
  if (liveHead.created_at > project.createdAt) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }
  const template = buildProjectColorPatchTemplate({ color, liveHead });
  const createdAt = Math.max(
    Math.floor(Date.now() / 1_000),
    liveHead.created_at + 1,
  );
  if (ownerControlAgentPubkey) {
    const [projectEvent] = await publishOwnedAgentAnnouncements(
      ownerControlAgentPubkey,
      [{ ...template, createdAt }],
    );
    if (!projectEvent || projectEvent.kind !== KIND_PROJECT_ANNOUNCEMENT) {
      throw new Error(
        "The owner agent updated the project but its response was incomplete. Refresh and try again.",
      );
    }
    if (
      projectEvent.pubkey.toLowerCase() !== targetOwner ||
      projectEvent.tags.find((tag) => tag[0] === "d")?.[1] !== project.dtag
    ) {
      throw new Error(
        "The owner agent published a different project. Refresh and try again.",
      );
    }
    return { ...project, color, createdAt: projectEvent.created_at };
  }
  const publication = await publishOwnerAnnouncement({
    ...template,
    createdAt,
    targetOwner,
  });
  if (publication.publicationError) {
    throw new Error(publication.publicationError);
  }
  return { ...project, color, createdAt: publication.event.created_at };
}

export function useSetProjectColorMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: SetProjectColorInput) => setProjectColor(input),
    onSuccess: (project) => {
      markProjectDataAuthoritative(project, "local-write");
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.map((candidate) =>
          candidate.id === project.id ? project : candidate,
        ),
      );
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
    },
  });
}
