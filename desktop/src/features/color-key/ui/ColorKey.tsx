import { useQueries } from "@tanstack/react-query";
import { Palette } from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  BENCH_KEY_LABELS,
  COLOR_KEY_TYPES,
  colorKeyAgents,
  colorKeyProjects,
} from "@/features/color-key/lib/colorKey";
import { InboxTypeTag } from "@/features/home/ui/InboxItemPreview";
import { agentAboutQueryOptions } from "@/features/messages/lib/useMentionAgentDetails";
import { useProjectsQuery } from "@/features/projects/hooks";
import { readProjectSidebarMembership } from "@/features/projects/lib/projectSidebarMembership";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import {
  readSidebarProjectsFilter,
  readSidebarProjectsSort,
} from "@/features/sidebar/ui/listSidebarProjects";
import { useIdentityQuery } from "@/shared/api/hooks";
import { FeatureGate } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** Set on this device once the first-run tip is dismissed or the key opens. */
export const COLOR_KEY_TIP_KEY = "buzz.color-key.tip.v1";

const KEY_BUTTON_CLASS =
  "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0";

const preventDefault = (event: Event) => event.preventDefault();

function Section({
  children,
  intro,
  title,
}: {
  children: React.ReactNode;
  intro: string;
  title: string;
}) {
  const id = React.useId();
  return (
    <section aria-labelledby={id} className="space-y-2">
      <div className="space-y-0.5">
        <h2 className="text-sm font-semibold text-foreground" id={id}>
          {title}
        </h2>
        <p className="text-xs text-muted-foreground">{intro}</p>
      </div>
      {children}
    </section>
  );
}

function Meaning({ children }: { children: React.ReactNode }) {
  return <p className="text-xs text-muted-foreground">{children}</p>;
}

function ProjectsSection() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const projectsQuery = useProjectsQuery();
  // The sidebar's own inputs, read without starting a second membership sync.
  const relayOrigin = getCachedRelayOrigin();
  const projects = colorKeyProjects({
    addedProjectAddresses: new Set(
      readProjectSidebarMembership(relayOrigin, pubkey),
    ),
    currentPubkey: pubkey,
    filter: readSidebarProjectsFilter(relayOrigin, pubkey),
    projects: projectsQuery.data ?? [],
    sort: readSidebarProjectsSort(relayOrigin, pubkey),
  });
  return (
    <Section
      intro="The projects in your sidebar, each with its color."
      title="Your projects"
    >
      {projects.length > 0 ? (
        <ul className="space-y-1" data-testid="color-key-projects">
          {projects.map((project) => (
            <li
              className="flex items-center gap-2 text-sm"
              key={project.address}
            >
              <span
                aria-hidden
                className={cn(
                  "size-2 shrink-0 rounded-full",
                  project.color.dot,
                )}
                data-project-color={project.color.id}
              />
              <span className="min-w-0 truncate">{project.name}</span>
            </li>
          ))}
        </ul>
      ) : (
        <Meaning>No projects in your sidebar yet.</Meaning>
      )}
      <Meaning>
        An Inbox row's left stripe shows its project's color when it belongs to
        a project, otherwise its item type's color.
      </Meaning>
      <Meaning>Change a project's color on its home page, under Color.</Meaning>
    </Section>
  );
}

function AgentsSection() {
  const agentsQuery = useManagedAgentsQuery();
  const agents = agentsQuery.data ?? [];
  const abouts = useQueries({
    queries: agents.map((agent) => agentAboutQueryOptions(agent.pubkey)),
  });
  const aboutByPubkey = new Map(
    agents.map((agent, index) => [agent.pubkey, abouts[index]?.data]),
  );
  const rows = colorKeyAgents(agents, (pubkey) => aboutByPubkey.get(pubkey));
  return (
    <Section intro="Your agents and what each one does." title="Who's who">
      {agentsQuery.isPending ? (
        <Meaning>Loading agents...</Meaning>
      ) : rows.length > 0 ? (
        <ul className="space-y-2" data-testid="color-key-agents">
          {rows.map((agent) => (
            <li className="flex items-start gap-2" key={agent.pubkey}>
              <UserAvatar
                avatarUrl={agent.avatarUrl}
                displayName={agent.name}
                shape="squircle"
                size="sm"
              />
              <div className="min-w-0">
                <p className="text-sm font-medium [overflow-wrap:anywhere]">
                  {agent.name}
                </p>
                <Meaning>{agent.job}</Meaning>
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <Meaning>No agents yet.</Meaning>
      )}
    </Section>
  );
}

/**
 * The color key: Inbox item types, project colors, Bench card labels and
 * who's who. A color always sits beside its name or icon.
 */
export function ColorKeyContent() {
  return (
    <div className="space-y-5" data-testid="color-key">
      <Section
        intro="Every Inbox row shows one type, with its icon, name and color."
        title="Item types"
      >
        <ul className="space-y-2">
          {COLOR_KEY_TYPES.map(({ meaning, stripe, type }) => (
            <li className="flex items-start gap-2" key={type}>
              <span
                aria-hidden
                className={cn(
                  "mt-0.5 h-8 shrink-0 border-l-2 border-l-transparent",
                  stripe,
                )}
              />
              <div className="min-w-0 text-xs">
                <InboxTypeTag type={type} />
                <Meaning>{meaning}</Meaning>
              </div>
            </li>
          ))}
        </ul>
      </Section>
      <FeatureGate feature="projects">
        <ProjectsSection />
      </FeatureGate>
      <Section intro="The labels a Bench card can show." title="Bench cards">
        <ul className="space-y-2">
          {BENCH_KEY_LABELS.map(({ chips, meaning }) => (
            <li className="space-y-1" key={chips[0].label}>
              <div className="flex flex-wrap items-center gap-1.5">
                {chips.map(({ label, variant }) =>
                  variant ? (
                    <Badge key={label} variant={variant}>
                      {label}
                    </Badge>
                  ) : (
                    <span
                      className="text-xs font-medium text-foreground"
                      key={label}
                    >
                      {label}
                    </span>
                  ),
                )}
              </div>
              <Meaning>{meaning}</Meaning>
            </li>
          ))}
        </ul>
      </Section>
      <AgentsSection />
    </div>
  );
}

/**
 * The "Key" button and its popover. With `firstRunTip`, a one-time tip points
 * at it until it is dismissed or the key is opened; the tip never takes focus.
 */
export function ColorKeyButton({
  firstRunTip = false,
}: {
  firstRunTip?: boolean;
}) {
  const [tip, setTip] = React.useState(
    () => firstRunTip && getStorageItem(COLOR_KEY_TIP_KEY) === null,
  );
  const seen = () => {
    setTip(false);
    setStorageItem(COLOR_KEY_TIP_KEY, "seen");
  };
  return (
    <span className="relative inline-flex shrink-0">
      {firstRunTip ? (
        <Popover
          onOpenChange={(open) => {
            if (!open) seen();
          }}
          open={tip}
        >
          <PopoverAnchor asChild>
            <span
              aria-hidden
              className="pointer-events-none absolute inset-0"
            />
          </PopoverAnchor>
          <PopoverContent
            align="end"
            aria-label="Tip"
            className="w-60 p-3"
            data-testid="color-key-tip"
            onCloseAutoFocus={preventDefault}
            // Focus moving elsewhere is not a dismissal; a click or Esc is.
            onFocusOutside={preventDefault}
            onOpenAutoFocus={preventDefault}
          >
            <p className="text-sm">New here? The Key explains the colors.</p>
            <div className="mt-2 flex justify-end">
              <Button onClick={seen} size="xs" type="button" variant="ghost">
                Got it
              </Button>
            </div>
          </PopoverContent>
        </Popover>
      ) : null}
      <Popover
        onOpenChange={(open) => {
          if (open) seen();
        }}
      >
        <PopoverTrigger asChild>
          <button
            aria-label="Color key"
            className={KEY_BUTTON_CLASS}
            data-testid="color-key-trigger"
            type="button"
          >
            <Palette aria-hidden />
            Key
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="end"
          aria-label="Color key"
          className="max-h-[min(32rem,var(--radix-popover-content-available-height))] w-96 max-w-[calc(100vw-2rem)] overflow-y-auto overscroll-contain"
          data-testid="color-key-popover"
        >
          <ColorKeyContent />
        </PopoverContent>
      </Popover>
    </span>
  );
}

/** Settings > Color key: the same key, as a page. */
export function ColorKeySettingsCard() {
  return (
    <section className="min-w-0" data-testid="settings-color-key">
      <SettingsSectionHeader
        description="What the colors and labels in the Inbox and on the Bench mean."
        title="Color key"
      />
      <ColorKeyContent />
    </section>
  );
}
