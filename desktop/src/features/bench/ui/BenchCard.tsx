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
  benchDeadlineSeconds,
  benchForLabel,
  benchKindLabel,
  benchOptionViews,
  benchPlainView,
  benchPreviousAnswerLine,
  benchReceipt,
  benchRecordedLine,
  benchSeverityChip,
  benchStateLabel,
  benchValidityLabel,
  safeBenchLinks,
} from "@/features/bench/lib/benchCards";
import { useBenchAnswer } from "@/features/bench/useBenchCards";
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
 * it. `onOther` opens the card's thread: Other, where a reply is the
 * answer, and Open thread, on every card that has one. A closed card is its
 * receipt: the question, the answer and the outcome.
 */
export function BenchCard({
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
  /** Leads the muted line: the Bench screen's All tab names the project. */
  projectLabel?: string;
}) {
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
      <BenchOpenThread card={card} onOpenThread={onOther} />
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
      <BenchOpenThread card={card} onOpenThread={onOther} />
    </Card>
  );
}

/** Hidden until the card has a live (or kept) message to thread on. */
function BenchOpenThread({
  card,
  onOpenThread,
}: {
  card: BenchCardData;
  onOpenThread?: () => void;
}) {
  if (!onOpenThread || !card.cardEventId) return null;
  return (
    <Button
      className="-ml-2 mt-1.5 h-7 px-2 text-xs"
      onClick={onOpenThread}
      size="sm"
      type="button"
      variant="ghost"
    >
      Open thread
    </Button>
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
  const meta = [projectLabel, benchKindLabel(card.kind), card.area, card.writer]
    .filter(Boolean)
    .join(" · ");
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
  const plain = benchPlainView(card);
  return (
    <>
      <p className="mt-1.5 text-message font-semibold text-foreground [overflow-wrap:anywhere]">
        {card.title}
      </p>
      {plain.chips.length > 0 ? (
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          {plain.chips.map((chip) => (
            <Badge
              className="max-w-full [overflow-wrap:anywhere]"
              key={chip.label}
              variant={chip.variant}
            >
              {chip.label}
            </Badge>
          ))}
        </div>
      ) : null}
      {plain.lines.map((line) => (
        <p
          className="mt-1 text-sm text-foreground/90 [overflow-wrap:anywhere]"
          key={line.label}
        >
          <span className="font-medium text-foreground">{line.label}:</span>{" "}
          {line.text}
        </p>
      ))}
      {card.body ? (
        <p className="mt-1 whitespace-pre-wrap text-message text-foreground/90 [overflow-wrap:anywhere]">
          {card.body}
        </p>
      ) : null}
      {plain.recommended ? (
        <p className="mt-1.5 text-sm text-foreground/90 [overflow-wrap:anywhere]">
          {plain.recommended}
        </p>
      ) : null}
      {plain.action ? (
        <p
          className="mt-1.5 text-xs text-muted-foreground [overflow-wrap:anywhere]"
          data-testid="bench-card-action"
        >
          {plain.action.lead}{" "}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-foreground">
            {plain.action.command}
          </code>
        </p>
      ) : null}
      {plain.details ? (
        <details className="mt-1.5 text-sm">
          <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
            Details
          </summary>
          <p className="mt-1 whitespace-pre-wrap text-foreground/90 [overflow-wrap:anywhere]">
            {plain.details}
          </p>
        </details>
      ) : null}
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
