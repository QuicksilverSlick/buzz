export const SECTION_ICON_BUTTON_CLASS =
  "sidebar-section-action flex size-6 items-center justify-center rounded-[4px] p-1 text-sidebar-foreground/50 transition-colors hover:bg-sidebar-border/35 hover:text-sidebar-foreground focus-visible:bg-sidebar-border/35 focus-visible:text-sidebar-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring [&>svg]:size-4 [&>svg]:shrink-0";

export const SECTION_ACTION_VISIBILITY_CLASS =
  "opacity-0 transition-opacity group-hover/sidebar-section:opacity-100 group-focus-within/sidebar-section:opacity-100 group-data-[section-actions-open=true]/sidebar-section:opacity-100";

/** The primary menu's count pill (Inbox, Bench) on a SidebarMenuBadge. */
export const SIDEBAR_COUNT_BADGE_CLASS =
  "right-2 rounded-full bg-primary/15 px-1.5 text-2xs text-primary peer-data-[active=true]/menu-button:bg-sidebar-active-foreground/20 peer-data-[active=true]/menu-button:text-sidebar-active-foreground";
