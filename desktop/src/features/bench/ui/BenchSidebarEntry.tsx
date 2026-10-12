import { ListChecks } from "lucide-react";

import { useBenchNeedsYouCount } from "@/features/bench/useBenchCards";
import { SIDEBAR_COUNT_BADGE_CLASS } from "@/features/sidebar/ui/sidebarSectionStyles";
import {
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";

/**
 * The Bench in the sidebar's primary menu, shown only while the Bench is
 * enabled on this computer, with a badge for the items that need the owner.
 */
export function BenchSidebarEntry({
  isActive,
  onSelect,
}: {
  isActive: boolean;
  onSelect: () => void;
}) {
  const needsYou = useBenchNeedsYouCount();
  if (needsYou === null) return null;
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        className="data-[active=true]:font-normal"
        data-testid="open-bench-view"
        isActive={isActive}
        onClick={onSelect}
        tooltip="Bench"
        type="button"
      >
        <ListChecks className="h-4 w-4" />
        <SidebarMenuLabel>Bench</SidebarMenuLabel>
      </SidebarMenuButton>
      {needsYou > 0 ? (
        <SidebarMenuBadge
          className={SIDEBAR_COUNT_BADGE_CLASS}
          data-testid="sidebar-bench-count"
        >
          {Math.min(needsYou, 99)}
        </SidebarMenuBadge>
      ) : null}
    </SidebarMenuItem>
  );
}
