import * as React from "react";
import { Check } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type BenchCard as BenchCardData,
  type BenchOptionView,
  type SafeBenchLink,
  BENCH_SAVE_TIMEOUT_MS,
  INITIAL_BENCH_ANSWER_STATE,
  benchAnswerReducer,
  benchCardMode,
  benchCreatedSeconds,
  benchDeadlineSeconds,
  benchForLabel,
  benchOptionViews,
  benchPreviousAnswerLine,
  benchReceipt,
  benchRecordedLine,
  benchSeverityChip,
  benchStateLabel,
  benchValidityLabel,
  formatBenchCreated,
  safeBenchLinks,
} from "@/features/bench/lib/benchCards";
import {
  benchCardLayout,
  benchCardMeta,
  benchPostCard,
} from "@/features/bench/lib/benchCardLayout";
import {
  BenchCardBlocks,
  BenchCardDetails,
} from "@/features/bench/ui/BenchCardFields";
import {
  BenchWhyBox,
  benchWhyAfterClick,
} from "@/features/bench/ui/BenchWhyBox";
import { useBenchAnswer } from "@/features/bench/useBenchCards";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { openPopoverLink } from "@/features/messages/lib/openPopoverLink";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { ExternalLinkAnchor } from "@/shared/ui/markdown/ExternalLinkAnchor";
import { Spinner } from "@/shared/ui/spinner";

function formatBenchTime(unixSeconds: number): string {
  return formatItemTimestamp(unixSeconds, { withTime: true });
}

/**
 * A Bench decision card, in #bench or its project channel. Every string here
 * was written by an agent, so it renders as plain React text: never
 * markdown, never HTML. Option buttons answer through `bench_answer` with the
 * owner's keycap; the card shows "Saving..." until the Bench's tick records
 * it. A tap on a hold-type option (Hold, Keep holding, Send it back, Close
 * it) also offers an optional Why? line, sent as the owner's reply on the
 * card (`BenchWhyBox`); the tap never waits on it. `onOther` opens the
 * card's thread: Other, where a reply is the answer, and Open thread, on
 * every card that has one, beside when the card was created. A closed card
 * is its receipt: the question, the answer and the outcome. A `retiredPost`
 * (one a newer card replaced) is read-only, from its own text, so it never
 * answers twice.
 */
export function BenchCard({
  card,
  ownerPubkey,
  viewerPubkey,
  onOther,
  projectLabel,
  retiredPost,
}: {
  card: BenchCardData;
  ownerPubkey: string | null;
  viewerPubkey?: string;
  onOther?: () => void;
  /** Leads the muted line: the Bench screen's All tab names the project. */
  projectLabel?: string;
  retiredPost?: BenchRetiredPost;
}) {
  if (retiredPost) return <BenchRetiredCard card={card} post={retiredPost} />;
  return card.request ? (
    <BenchReceiptCard
      card={card}
      onOther={onOther}
      projectLabel={projectLabel}
    />
  ) : (
    <BenchQuestionCard
      card={card}
      onOther={onOther}
      ownerPubkey={ownerPubkey}
      projectLabel={projectLabel}
      viewerPubkey={viewerPubkey}
    />
  );
}

function BenchQuestionCard({
  card,
  ownerPubkey,
  viewerPubkey,
  onOther,
  projectLabel,
}: {
  card: BenchCardData;
  ownerPubkey: string | null;
  viewerPubkey?: string;
  onOther?: () => void;
  projectLabel?: string;
}) {
  const { interactive, readOnlyReason } = benchCardMode(
    card,
    ownerPubkey,
    viewerPubkey,
  );
  const byOwner = card.answer?.byOwner ?? true;
  const [storedState, dispatch] = React.useReducer(
    benchAnswerReducer,
    INITIAL_BENCH_ANSWER_STATE,
  );
  // Settle against the snapshot in render too, so the frame that shows the
  // new recorded answer never also shows "Saving...".
  const state = benchAnswerReducer(storedState, {
    type: "recorded",
    recorded: card.recorded,
    byOwner,
  });
  React.useEffect(() => {
    dispatch({ type: "recorded", recorded: card.recorded, byOwner });
  }, [card.recorded, byOwner]);

  const saving = state.phase === "saving";
  const attempt = state.attempt;
  React.useEffect(() => {
    if (!saving) return;
    const timer = window.setTimeout(
      () => dispatch({ type: "timedOut", attempt }),
      BENCH_SAVE_TIMEOUT_MS,
    );
    return () => window.clearTimeout(timer);
  }, [attempt, saving]);

  const { mutateAsync: answer } = useBenchAnswer();
  const handleOption = (option: number) => {
    // Only the card on screen: the Bench refuses it once it is replaced.
    const cardEventId = card.cardEventId;
    if (!cardEventId) return;
    const action = {
      type: "click",
      option,
      recorded: card.recorded,
      byOwner,
    } as const;
    const next = benchAnswerReducer(storedState, action);
    // Unchanged attempt: a save is still in flight, so the click is ignored.
    if (next.attempt === storedState.attempt) return;
    dispatch(action);
    benchWhyAfterClick(card, cardEventId, next.target);
    answer({ itemId: card.id, cardEventId, option: next.target }).catch(
      (error: unknown) => {
        dispatch({
          type: "commandFailed",
          attempt: next.attempt,
          message: error instanceof Error ? error.message : String(error),
        });
      },
    );
  };

  const options =
    card.kind === "pending" ? benchOptionViews(card, state, interactive) : [];
  const recordedLine = benchRecordedLine(card, formatBenchTime);
  const overrides = interactive && card.recorded !== null && !byOwner;
  const other =
    onOther && card.kind === "pending" && card.scoped && card.cardEventId;

  return (
    <Card
      className="mt-1 max-w-[min(100%,36rem)] px-3 py-2.5"
      data-bench-item={card.id}
      data-testid="bench-card"
    >
      <BenchCardHeader card={card} projectLabel={projectLabel} />
      <BenchCardText card={card} />
      {overrides ? (
        <p className="mt-2.5 text-xs text-muted-foreground">
          Answer as owner (overrides)
        </p>
      ) : null}
      {options.length > 0 ? (
        <fieldset
          aria-busy={saving}
          aria-label="Answer"
          className={cn(
            "flex flex-wrap gap-1.5",
            overrides ? "mt-1" : "mt-2.5",
          )}
        >
          {options.map((option) => (
            <BenchOptionButton
              key={option.index}
              onSelect={handleOption}
              option={option}
            />
          ))}
          {other ? (
            <Button
              aria-label="Other: reply in this card's thread"
              className="h-auto min-h-8 py-1.5"
              onClick={onOther}
              size="sm"
              type="button"
              variant="ghost"
            >
              Other...
            </Button>
          ) : null}
        </fieldset>
      ) : null}
      {interactive && card.kind === "pending" && card.cardEventId ? (
        <BenchWhyBox card={card} cardEventId={card.cardEventId} />
      ) : null}
      <div aria-live="polite" className="text-xs">
        {saving ? (
          <p className="mt-1.5 text-muted-foreground">Saving...</p>
        ) : recordedLine ? (
          <p
            className="mt-1.5 font-medium text-foreground [overflow-wrap:anywhere]"
            data-testid="bench-card-recorded"
          >
            {recordedLine}
          </p>
        ) : null}
        {state.phase === "failed" && state.error ? (
          <p className="mt-1.5 text-destructive [overflow-wrap:anywhere]">
            {state.error}
          </p>
        ) : null}
        {readOnlyReason ? (
          <p className="mt-1.5 text-muted-foreground">{readOnlyReason}</p>
        ) : null}
      </div>
      <BenchCardFooter card={card} onOpenThread={onOther} />
    </Card>
  );
}

/** The question as it closed, its answer, then the outcome: inert text. */
function BenchReceiptCard({
  card,
  onOther,
  projectLabel,
}: {
  card: BenchCardData;
  onOther?: () => void;
  projectLabel?: string;
}) {
  const receipt = benchReceipt(card, formatBenchTime);
  if (!receipt) return null;
  const { request } = receipt;
  const options = benchOptionViews(request, INITIAL_BENCH_ANSWER_STATE, false);
  return (
    <Card
      className="mt-1 max-w-[min(100%,36rem)] px-3 py-2.5"
      data-bench-item={card.id}
      data-testid="bench-card"
    >
      {/* The header says where the decision stands now, not what it was. */}
      <BenchCardHeader card={card} projectLabel={projectLabel} />
      <BenchCardText card={request} />
      {options.length > 0 ? (
        <fieldset
          aria-label="Options"
          className="mt-2.5 flex flex-wrap gap-1.5"
        >
          {options.map((option) => (
            <BenchOptionButton
              key={option.index}
              onSelect={() => {}}
              option={option}
            />
          ))}
        </fieldset>
      ) : null}
      <p
        className="mt-1.5 text-xs font-medium text-foreground [overflow-wrap:anywhere]"
        data-testid="bench-card-answered"
      >
        {receipt.answeredLine}
      </p>
      <section
        aria-label="Outcome"
        className="mt-2.5 border-t border-border/70 pt-2"
        data-testid="bench-card-outcome"
      >
        <p className="text-xs text-muted-foreground">{receipt.outcomeLine}</p>
        <BenchCardText card={card} />
      </section>
      <BenchCardFooter card={card} onOpenThread={onOther} />
    </Card>
  );
}

/** A post a newer card replaced: its text and channel, from its row. */
type BenchRetiredPost = {
  text: string;
  channelId: string | null | undefined;
};

/**
 * A replaced post in the card's layout, from its own text: the owner
 * replied to that wording, never the live card's. Read-only: no answer
 * buttons, no Other, no line on how to answer; its other lines as posted,
 * a receipt's Outcome as the receipt card has it. It links the live card
 * when there is one; with none, nothing is asked again yet.
 */
function BenchRetiredCard({
  card,
  post,
}: {
  card: BenchCardData;
  post: BenchRetiredPost;
}) {
  const shown = benchPostCard(card, post.text);
  const live = card.cardEventId;
  return (
    <Card
      className="mt-1 max-w-[min(100%,36rem)] px-3 py-2.5"
      data-bench-item={card.id}
      data-testid="bench-card-retired"
    >
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="secondary">{live ? "Replaced" : "Closed"}</Badge>
        <span className="min-w-0 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {shown.meta}
        </span>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        {live ? "Asked again on a newer card." : "No longer asked here."}
        {live && post.channelId ? (
          <>
            {" "}
            <BenchLinkAnchor
              link={{
                label: "Go to the newer card",
                scheme: "buzz",
                url: buildMessageLink({
                  channelId: post.channelId,
                  messageId: live,
                }),
              }}
            />
          </>
        ) : null}
      </p>
      <BenchCardText card={shown.card} />
      {shown.card.options.length > 0 ? (
        <ol
          aria-label="Options"
          className="mt-2.5 space-y-0.5 text-message text-foreground/90"
        >
          {shown.card.options.map((option, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: options are positional
            <li className="[overflow-wrap:anywhere]" key={index}>
              <span className="font-semibold tabular-nums">{index + 1}.</span>{" "}
              {option}
            </li>
          ))}
        </ol>
      ) : null}
      <BenchPostNotes notes={shown.notes} />
      {shown.outcome ? (
        <section
          aria-label="Outcome"
          className="mt-2.5 border-t border-border/70 pt-2"
          data-testid="bench-card-outcome"
        >
          <p className="text-xs text-muted-foreground">
            Outcome · {shown.outcome.meta}
          </p>
          <BenchCardText card={shown.outcome.card} />
          <BenchPostNotes notes={shown.outcome.notes} />
        </section>
      ) : null}
    </Card>
  );
}

function BenchPostNotes({ notes }: { notes: readonly string[] }) {
  return notes.map((note, index) => (
    <p
      className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]"
      // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
      key={index}
    >
      {note}
    </p>
  ));
}

/**
 * Open thread at the left, hidden until the card has a live (or kept)
 * message to thread on; when the card was created at the right.
 */
function BenchCardFooter({
  card,
  onOpenThread,
}: {
  card: BenchCardData;
  onOpenThread?: () => void;
}) {
  const thread = onOpenThread && card.cardEventId;
  const created = benchCreatedSeconds(card);
  if (!thread && created === null) return null;
  return (
    <div className="mt-1.5 flex items-center gap-2">
      {thread ? (
        <Button
          className="-ml-2 h-7 px-2 text-xs"
          onClick={onOpenThread}
          size="sm"
          type="button"
          variant="ghost"
        >
          Open thread
        </Button>
      ) : null}
      {created === null ? null : (
        <time
          className="ml-auto text-xs tabular-nums text-muted-foreground"
          data-testid="bench-card-created"
          dateTime={new Date(created * 1_000).toISOString()}
        >
          {formatBenchCreated(created)}
        </time>
      )}
    </div>
  );
}

function BenchCardHeader({
  card,
  projectLabel,
}: {
  card: BenchCardData;
  projectLabel?: string;
}) {
  const severity = benchSeverityChip(card.severity);
  const validity = benchValidityLabel(card.validity);
  const stateLabel =
    card.kind === "status" ? benchStateLabel(card.state) : null;
  const forLabel = benchForLabel(card);
  const previous = benchPreviousAnswerLine(card, formatBenchTime);
  const meta = benchCardMeta(card, projectLabel);
  return (
    <>
      <div className="flex flex-wrap items-center gap-1.5">
        {severity ? (
          <Badge variant={severity.variant}>{severity.label}</Badge>
        ) : null}
        {card.backToYou ? <Badge variant="info">Back to you</Badge> : null}
        {forLabel ? <Badge variant="outline">{forLabel}</Badge> : null}
        {stateLabel ? <Badge variant="secondary">{stateLabel}</Badge> : null}
        {validity ? <Badge variant="outline">{validity}</Badge> : null}
        <span className="min-w-0 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {meta}
        </span>
      </div>
      {previous ? (
        <p className="mt-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {previous}
        </p>
      ) : null}
    </>
  );
}

function BenchCardText({ card }: { card: BenchCardData }) {
  const deadline = benchDeadlineSeconds(card.deadline);
  const links = safeBenchLinks(card.links);
  const layout = benchCardLayout(card);
  return (
    <>
      <p className="mt-1.5 text-message font-semibold text-foreground [overflow-wrap:anywhere]">
        {card.title}
      </p>
      <BenchCardBlocks
        blocks={layout.blocks}
        className="mt-2 text-message text-foreground/90"
      />
      {layout.action ? (
        <p
          className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]"
          data-testid="bench-card-action"
        >
          {layout.action.lead}{" "}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">
            {layout.action.command}
          </code>
        </p>
      ) : null}
      {layout.details ? <BenchCardDetails text={layout.details} /> : null}
      {card.deadline ? (
        <p className="mt-1.5 text-xs text-muted-foreground">
          Due {deadline === null ? card.deadline : formatBenchTime(deadline)}
        </p>
      ) : null}
      {card.kind === "decision" && card.decidedBy ? (
        <p className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          Decided by {card.decidedBy}
        </p>
      ) : null}
      {links.length > 0 ? (
        <ul className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-sm">
          {links.map((link) => (
            <li className="min-w-0 [overflow-wrap:anywhere]" key={link.url}>
              <BenchLinkAnchor link={link} />
            </li>
          ))}
        </ul>
      ) : null}
    </>
  );
}

function BenchOptionButton({
  onSelect,
  option,
}: {
  onSelect: (index: number) => void;
  option: BenchOptionView;
}) {
  const label = `${option.number}. ${option.text}`;
  // A retired card shows its answer on a disabled button: nothing to undo.
  const undoable = option.selected && !option.disabled;
  const recommended = option.recommended ? ", recommended" : "";
  return (
    <Button
      aria-busy={option.pending || undefined}
      aria-label={
        undoable
          ? `${label}${recommended} (recorded, click to undo)`
          : option.selected
            ? `${label}${recommended} (recorded)`
            : `${label}${recommended}`
      }
      aria-pressed={option.selected}
      className={cn(
        "h-auto min-h-8 max-w-full justify-start whitespace-normal py-1.5 text-left",
        option.pending && "ring-1 ring-ring",
      )}
      disabled={option.disabled}
      onClick={() => onSelect(option.index)}
      size="sm"
      title={undoable ? "Recorded. Click to undo." : undefined}
      type="button"
      variant={option.selected ? "default" : "outline"}
    >
      <span className="font-semibold tabular-nums">{option.number}.</span>
      <span className="min-w-0 [overflow-wrap:anywhere]">{option.text}</span>
      {option.recommended ? (
        <Badge aria-hidden variant="secondary">
          Recommended
        </Badge>
      ) : null}
      {option.pending ? (
        <Spinner aria-hidden className="h-3.5 w-3.5 border-2" />
      ) : option.selected ? (
        <Check aria-hidden />
      ) : null}
    </Button>
  );
}

/** https opens like any chat link (OS browser, masked-URL tooltip). */
function BenchLinkAnchor({ link }: { link: SafeBenchLink }) {
  if (link.scheme === "buzz") return <BuzzBenchLinkAnchor link={link} />;
  return (
    <ExternalLinkAnchor
      anchorProps={{}}
      href={link.url}
      isLinearLink={false}
      label={link.label}
    >
      {link.label}
    </ExternalLinkAnchor>
  );
}

/** buzz://message opens in-app; any other buzz:// URL goes to the OS. */
function BuzzBenchLinkAnchor({ link }: { link: SafeBenchLink }) {
  const { goChannel } = useAppNavigation();
  return (
    <a
      className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80"
      href={link.url}
      onClick={(event) => {
        event.preventDefault();
        openPopoverLink(link.url, {
          openExternal: (url) => {
            void openUrl(url).catch(() => {
              toast.error("Failed to open link");
            });
          },
          openMessageLink: (target) => {
            void goChannel(target.channelId, {
              messageId: target.messageId,
              threadRootId: target.threadRootId,
            });
          },
        });
      }}
      title={link.url}
    >
      {link.label}
    </a>
  );
}
