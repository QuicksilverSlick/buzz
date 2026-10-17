import * as React from "react";
import {
  AtSign,
  CircleAlert,
  GitPullRequest,
  ListChecks,
  type LucideIcon,
  MessageCircle,
  MessagesSquare,
} from "lucide-react";

import {
  type BenchCard,
  INITIAL_BENCH_ANSWER_STATE,
  benchDeadlineSeconds,
  benchOptionViews,
  benchSeverityChip,
  benchStateLabel,
} from "@/features/bench/lib/benchCards";
import {
  benchCardLayout,
  benchCardMeta,
} from "@/features/bench/lib/benchCardLayout";
import {
  BenchCardBlocks,
  BenchCardDetails,
} from "@/features/bench/ui/BenchCardFields";
import type { InboxItem, InboxTypeLabel } from "@/features/home/lib/inbox";
import {
  type InboxClass,
  type InboxItemType,
  inboxRepoDtag,
} from "@/features/home/lib/inboxClass";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { Badge } from "@/shared/ui/badge";
import { Markdown } from "@/shared/ui/markdown";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

export const INBOX_PREVIEW_OPEN_DELAY_MS = 700;
export const INBOX_PREVIEW_CLOSE_DELAY_MS = 180;

/**
 * Each Inbox type's icon, label and color. The color is never alone: the
 * icon and label always show with it. Text is 800 light / 300 dark, 4.5:1
 * on the default themes (pinned by the test); the row's left stripe is the
 * 500 at 70% when the row has no project (a project's row takes its
 * project's color, never one of these hues).
 */
export const INBOX_TYPE_META: Record<
  InboxItemType,
  { label: string; icon: LucideIcon; text: string; stripe: string }
> = {
  "needs-you": {
    label: "Needs you",
    icon: CircleAlert,
    text: "text-amber-800 dark:text-amber-300",
    stripe: "border-l-amber-500/70",
  },
  bench: {
    label: "Bench",
    icon: ListChecks,
    text: "text-violet-800 dark:text-violet-300",
    stripe: "border-l-violet-500/70",
  },
  code: {
    label: "Code",
    icon: GitPullRequest,
    text: "text-blue-800 dark:text-blue-300",
    stripe: "border-l-blue-500/70",
  },
  dm: {
    label: "DM",
    icon: MessageCircle,
    text: "text-emerald-800 dark:text-emerald-300",
    stripe: "border-l-emerald-500/70",
  },
  mention: {
    label: "Mention",
    icon: AtSign,
    text: "text-fuchsia-800 dark:text-fuchsia-300",
    stripe: "border-l-fuchsia-500/70",
  },
  thread: {
    label: "Thread",
    icon: MessagesSquare,
    text: "text-muted-foreground",
    stripe: "border-l-transparent",
  },
};

/** The row's left stripe: its project's color, else its type's. */
export function inboxRowStripe(
  inboxClass: Pick<InboxClass, "color" | "type">,
): string {
  return inboxClass.color?.stripe ?? INBOX_TYPE_META[inboxClass.type].stripe;
}

/**
 * The row's label, led by its type: "Needs you in", "Bench card in", or the
 * Inbox's own wording ("Mentioned in", "DM from Ana", "Review") otherwise.
 */
export function inboxRowLabel(
  type: InboxItemType,
  label: InboxTypeLabel,
): InboxTypeLabel {
  const lead =
    type === "needs-you" ? "Needs you" : type === "bench" ? "Bench card" : null;
  return lead
    ? {
        text: label.channelLabel ? `${lead} in` : lead,
        channelLabel: label.channelLabel,
      }
    : label;
}

/** Rows ignore hovers here: profile popovers and the action bar own them. */
const PREVIEW_IGNORE_SELECTOR =
  "[data-inbox-profile-trigger], [data-inbox-preview-ignore]";

/**
 * Events from the avatar, name or action bar, or from a portal the row owns
 * (a profile popover, the context menu, a tooltip), which React bubbles here
 * though the DOM node is elsewhere.
 */
function ignored(event: React.SyntheticEvent<HTMLElement>): boolean {
  const target = event.target;
  return (
    !(target instanceof Element) ||
    !event.currentTarget.contains(target) ||
    target.closest(PREVIEW_IGNORE_SELECTOR) !== null
  );
}

function focusVisible(target: EventTarget | null): boolean {
  try {
    return target instanceof Element && target.matches(":focus-visible");
  } catch {
    return false;
  }
}

/**
 * A read-only preview of an Inbox row, beside it: 700 ms after the pointer
 * settles on the row, at once on keyboard focus. Never over the avatar,
 * name or action bar; a click, right-click, scroll or
 * Escape closes it until the pointer leaves the row. It takes no focus and
 * holds no buttons or links.
 */
export function InboxItemPreview({
  channelDescription,
  children,
  inboxClass,
  item,
  label,
}: {
  channelDescription: string | null;
  children: React.ReactNode;
  inboxClass: InboxClass;
  item: InboxItem;
  label: InboxTypeLabel;
}) {
  const [open, setOpen] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  // Dismissed rows stay closed until the pointer leaves them.
  const dismissed = React.useRef(false);

  const clearTimer = React.useCallback(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  }, []);
  const later = React.useCallback(
    (next: boolean, ms: number) => {
      clearTimer();
      timer.current = setTimeout(() => {
        timer.current = null;
        setOpen(next);
      }, ms);
    },
    [clearTimer],
  );
  // Leaving closes an open preview after a beat, or cancels a pending one.
  const closeSoon = () => {
    if (open) later(false, INBOX_PREVIEW_CLOSE_DELAY_MS);
    else clearTimer();
  };
  const dismiss = React.useCallback(() => {
    clearTimer();
    dismissed.current = true;
    setOpen(false);
  }, [clearTimer]);

  React.useEffect(() => clearTimer, [clearTimer]);
  React.useEffect(() => {
    if (!open) return;
    // Scroll never bubbles; capture sees the list's and the page's.
    window.addEventListener("scroll", dismiss, true);
    return () => window.removeEventListener("scroll", dismiss, true);
  }, [dismiss, open]);

  return (
    <Popover
      onOpenChange={(next) => {
        if (!next) dismiss();
      }}
      open={open}
    >
      <PopoverAnchor asChild>
        {/* biome-ignore lint/a11y/noStaticElementInteractions: hover and focus bubble from the row's own controls; the wrapper only times the read-only preview. */}
        <div
          onBlur={closeSoon}
          onContextMenu={dismiss}
          onFocus={(event) => {
            if (ignored(event) || !focusVisible(event.target)) {
              return;
            }
            clearTimer();
            dismissed.current = false;
            setOpen(true);
          }}
          onMouseLeave={() => {
            dismissed.current = false;
            closeSoon();
          }}
          onMouseOver={(event) => {
            if (dismissed.current) return;
            if (ignored(event)) {
              clearTimer();
              setOpen(false);
            } else if (open) {
              clearTimer();
            } else if (timer.current === null) {
              later(true, INBOX_PREVIEW_OPEN_DELAY_MS);
            }
          }}
          onPointerDown={dismiss}
        >
          {children}
        </div>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        aria-label="Preview"
        className="w-80 p-3"
        data-testid="inbox-item-preview"
        onCloseAutoFocus={(event) => event.preventDefault()}
        onMouseEnter={clearTimer}
        onMouseLeave={closeSoon}
        onOpenAutoFocus={(event) => event.preventDefault()}
        side="right"
      >
        <PreviewHeader inboxClass={inboxClass} item={item} label={label} />
        {inboxClass.benchCard ? (
          <BenchPreview card={inboxClass.benchCard} />
        ) : inboxClass.type === "code" ? (
          <CodePreview item={item} label={label} />
        ) : (
          <MessagePreview channelDescription={channelDescription} item={item} />
        )}
      </PopoverContent>
    </Popover>
  );
}

/** The type's icon and label, in its color. */
export function InboxTypeTag({ type }: { type: InboxItemType }) {
  const meta = INBOX_TYPE_META[type];
  const Icon = meta.icon;
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1 font-medium",
        meta.text,
      )}
    >
      <Icon aria-hidden className="h-3 w-3 shrink-0" />
      {meta.label}
    </span>
  );
}

function PreviewHeader({
  inboxClass,
  item,
  label,
}: {
  inboxClass: InboxClass;
  item: InboxItem;
  label: InboxTypeLabel;
}) {
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs">
      <InboxTypeTag type={inboxClass.type} />
      {inboxClass.workbench ? (
        <span
          className={cn(
            "inline-flex max-w-full items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-2xs font-medium",
            inboxClass.color?.text ?? "text-muted-foreground",
          )}
          data-inbox-project-chip=""
        >
          {inboxClass.color ? (
            <span
              aria-hidden
              className={cn(
                "size-1.5 shrink-0 rounded-full",
                inboxClass.color.dot,
              )}
            />
          ) : null}
          <span className="truncate">{inboxClass.workbench}</span>
        </span>
      ) : null}
      {label.channelLabel ? (
        <span className="min-w-0 truncate text-muted-foreground">
          #{label.channelLabel}
        </span>
      ) : null}
      <span className="ml-auto shrink-0 tabular-nums text-muted-foreground">
        {item.timestampLabel}
      </span>
    </div>
  );
}

function BenchPreview({ card }: { card: BenchCard }) {
  const severity = benchSeverityChip(card.severity);
  const state = card.kind === "status" ? benchStateLabel(card.state) : null;
  const layout = benchCardLayout(card);
  const options = benchOptionViews(
    card,
    INITIAL_BENCH_ANSWER_STATE,
    false,
  ).slice(0, 4);
  const deadline = benchDeadlineSeconds(card.deadline);
  // Agent-written text: plain React text, never markdown or HTML.
  return (
    <div className="mt-2 space-y-1.5 text-xs text-foreground/90">
      <div className="flex flex-wrap items-center gap-1.5">
        {severity ? (
          <Badge variant={severity.variant}>{severity.label}</Badge>
        ) : null}
        {state ? <Badge variant="secondary">{state}</Badge> : null}
        <span className="min-w-0 text-muted-foreground [overflow-wrap:anywhere]">
          {benchCardMeta(card)}
        </span>
      </div>
      <p className="text-sm font-semibold text-foreground [overflow-wrap:anywhere]">
        {card.title}
      </p>
      <BenchCardBlocks blocks={layout.blocks} clamp />
      {layout.details ? (
        <BenchCardDetails text={layout.details} toggle={false} />
      ) : null}
      {options.length > 0 ? (
        <ol className="space-y-0.5">
          {options.map((option) => (
            <li className="[overflow-wrap:anywhere]" key={option.index}>
              <span className="font-semibold tabular-nums">
                {option.number}.
              </span>{" "}
              {option.text}
              {option.recommended ? (
                <span className="text-muted-foreground"> · Recommended</span>
              ) : null}
              {option.selected ? (
                <span className="font-medium text-foreground"> · Answered</span>
              ) : null}
            </li>
          ))}
        </ol>
      ) : null}
      {card.deadline ? (
        <p className="text-muted-foreground">
          Due{" "}
          {deadline === null
            ? card.deadline
            : formatItemTimestamp(deadline, { withTime: true })}
        </p>
      ) : null}
    </div>
  );
}

function CodePreview({
  item,
  label,
}: {
  item: InboxItem;
  label: InboxTypeLabel;
}) {
  const repo = inboxRepoDtag(item);
  return (
    <div className="mt-2 space-y-1 text-xs">
      <p className="text-sm font-semibold text-foreground [overflow-wrap:anywhere]">
        {item.subject}
      </p>
      <p className="text-muted-foreground">
        {label.text}
        {repo ? ` · ${repo}` : null}
      </p>
    </div>
  );
}

function MessagePreview({
  channelDescription,
  item,
}: {
  channelDescription: string | null;
  item: InboxItem;
}) {
  const count = new Set(item.groupItems.map((event) => event.id)).size;
  return (
    <div className="mt-2 space-y-1.5 text-xs">
      <Markdown
        className="inbox-preview-markdown line-clamp-6 text-sm text-foreground"
        content={item.preview}
        interactive={false}
        mentionNames={item.mentionNames}
      />
      {channelDescription ? (
        <p className="line-clamp-2 text-muted-foreground [overflow-wrap:anywhere]">
          {channelDescription}
        </p>
      ) : null}
      {count > 1 ? (
        <p className="text-muted-foreground">{count} messages</p>
      ) : null}
    </div>
  );
}
