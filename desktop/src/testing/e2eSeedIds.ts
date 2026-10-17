// Ids of the messages the e2e bridge seeds into #general, shared with the
// specs. Real 64-hex ids: the timeline honours edits, deletions and reactions
// only when their `e` target is one, so a short id leaves those inert.

/** "Welcome to #general", authored by the default mock identity. */
export const GENERAL_WELCOME_EVENT_ID =
  "986dae74862b950bd2cd271abe92887ca3012152a70e5c222308006dcbbc6b5f";

/** "Hey team — checking in.", authored by Alice. */
export const GENERAL_ALICE_EVENT_ID =
  "b89175fd5c978579aa65e02824795136a5340af4f71d97c89016766008d123f4";
