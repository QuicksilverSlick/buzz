// The product name, in one place.
//
// This fork ships as Dreamforge. Upstream is `block/buzz`, and we track it, so
// the rename is deliberately confined to what a person actually sees. Nothing
// here changes an identifier that code or the protocol depends on:
//
//   RENAMED   window title, app name, and user-visible copy
//   UNCHANGED crate names (buzz-*), so upstream `use` paths keep resolving
//   UNCHANGED BUZZ_* environment variables, so upstream code that reads a new
//             variable still finds the one we set
//   UNCHANGED on-the-wire tags (buzz-channel, buzz-protect, buzz-visibility)
//             and event kinds, so we stay interoperable with other Buzz
//             relays and clients
//
// Those three exclusions are not cosmetic caution. Renaming an env var makes
// upstream code read a variable nobody sets — it compiles, runs, and silently
// takes a default. Renaming a wire tag diverges the protocol with no error at
// all. Both fail quietly, which is the failure mode this project exists to
// remove.
//
// To rename again later: change PRODUCT_NAME here, then sweep the remaining
// prose mentions. Sentences that mention the product by name hold the literal
// word rather than an interpolation — turning every sentence into a template
// literal buys nothing and makes each one a merge conflict against upstream.
// The identity surfaces below are what must stay centralized.

/** The product name shown to people. */
export const PRODUCT_NAME = "Dreamforge";

/** Window title and anywhere the app names itself standalone. */
export const APP_TITLE = PRODUCT_NAME;

/** Built-in terminal panel, e.g. the "Open …" control in a channel header. */
export const TERMINAL_LABEL = `${PRODUCT_NAME} Term`;

/** Shared compute pool, shown when choosing where an agent runs. */
export const SHARED_COMPUTE_LABEL = `${PRODUCT_NAME} shared compute`;
