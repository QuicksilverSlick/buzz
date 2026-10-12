import type { InboxFilter } from "./inbox";

export type InboxFilterOption = {
  label: string;
  value: InboxFilter;
};

export const INBOX_FILTER_OPTIONS: readonly InboxFilterOption[] = [
  { value: "all", label: "All" },
  { value: "project", label: "Projects" },
  { value: "mention", label: "Mentions" },
  { value: "thread", label: "Threads" },
  { value: "needs_action", label: "Needs action" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
  { value: "drafts", label: "Drafts" },
];

/**
 * The filters the inbox menu offers. Projects is a preview feature, so its
 * filter appears only while Projects is turned on.
 */
export function visibleInboxFilterOptions(
  projectsEnabled: boolean,
): readonly InboxFilterOption[] {
  return projectsEnabled
    ? INBOX_FILTER_OPTIONS
    : INBOX_FILTER_OPTIONS.filter((option) => option.value !== "project");
}
