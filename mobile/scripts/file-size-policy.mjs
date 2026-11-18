export const rules = [
  // Fork only (QuicksilverSlick/buzz #155): Block's compose bar arrives at 1202
  // lines in one catch-up merge. Block's own ratchet allows it because Block's
  // base was already over the limit; ours was 1120, so the merge reads as growth.
  // shortcut: one-time ceiling for a Block-owned folder; remove it once #155 is
  // on main (the base is then over the limit, so hold-or-shrink applies anyway).
  {
    root: "lib/features/channels/compose_bar",
    extensions: new Set([".dart"]),
    maxLines: 1202,
  },
  {
    root: "lib",
    extensions: new Set([".dart"]),
    maxLines: 1200,
  },
];
