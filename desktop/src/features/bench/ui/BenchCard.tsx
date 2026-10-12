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
  benchKindLabel,
  benchOptionViews,
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
 * A Bench decision card inside #bench. Every string here was written by an
 * agent, so it renders as plain React text: never markdown, never HTML.
 * Option buttons answer through `bench_answer` with the owner's keycap; the
 * card shows "Saving..." until the Bench's tick records it.
 */
export function BenchCard({
  card,
  ownerPubkey,
  viewerPubkey,
}: {
  card: BenchCardData;
  ownerPubkey: string | null;
  viewerPubkey?: string;
}) {
  const { interactive, readOnlyReason } = benchCardMode(
    card,
    ownerPubkey,
    viewerPubkey,
  );
  const [storedState, dispatch] = React.useReducer(
    benchAnswerReducer,
    INITIAL_BENCH_ANSWER_STATE,
  );
  // Settle against the snapshot in render too, so the frame that shows the
  // new recorded answer never also shows "Saving...".
  const state = benchAnswerReducer(storedState, {
    type: "recorded",
    recorded: card.recorded,
  });
  React.useEffect(() => {
    dispatch({ type: "recorded", recorded: card.recorded });
  }, [card.recorded]);

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
    const action = {
      type: "click",
      option,
      recorded: card.recorded,
    } as const;
    const next = benchAnswerReducer(storedState, action);
    // Unchanged attempt: a save is still in flight, so the click is ignored.
    if (next.attempt === storedState.attempt) return;
    dispatch(action);
    answer({ itemId: card.id, option: next.target }).catch((error: unknown) => {
      dispatch({
        type: "commandFailed",
        attempt: next.attempt,
        message: error instanceof Error ? error.message : String(error),
      });
    });
  };

  const severity = benchSeverityChip(card.severity);
  const validity = benchValidityLabel(card.validity);
  const stateLabel =
    card.kind === "status" ? benchStateLabel(card.state) : null;
  const deadline = benchDeadlineSeconds(card.deadline);
  const links = safeBenchLinks(card.links);
  const options =
    card.kind === "pending" ? benchOptionViews(card, state, interactive) : [];
  const recordedLine = benchRecordedLine(card, formatBenchTime);
  const meta = [benchKindLabel(card.kind), card.area, card.writer]
    .filter(Boolean)
    .join(" · ");

  return (
    <Card
      className="mt-1 max-w-[min(100%,36rem)] px-3 py-2.5"
      data-bench-item={card.id}
      data-testid="bench-card"
    >
      <div className="flex flex-wrap items-center gap-1.5">
        {severity ? (
          <Badge variant={severity.variant}>{severity.label}</Badge>
        ) : null}
        {stateLabel ? <Badge variant="secondary">{stateLabel}</Badge> : null}
        {validity ? <Badge variant="outline">{validity}</Badge> : null}
        <span className="min-w-0 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {meta}
        </span>
      </div>
      <p className="mt-1.5 text-message font-semibold text-foreground [overflow-wrap:anywhere]">
        {card.title}
      </p>
      {card.body ? (
        <p className="mt-1 whitespace-pre-wrap text-message text-foreground/90 [overflow-wrap:anywhere]">
          {card.body}
        </p>
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
      {options.length > 0 ? (
        <fieldset
          aria-busy={saving}
          aria-label="Answer"
          className="mt-2.5 flex flex-wrap gap-1.5"
        >
          {options.map((option) => (
            <BenchOptionButton
              key={option.index}
              onSelect={handleOption}
              option={option}
            />
          ))}
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
    </Card>
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
  return (
    <Button
      aria-busy={option.pending || undefined}
      aria-label={
        option.selected ? `${label} (recorded, click to undo)` : label
      }
      aria-pressed={option.selected}
      className={cn(
        "h-auto min-h-8 max-w-full justify-start whitespace-normal py-1.5 text-left",
        option.pending && "ring-1 ring-ring",
      )}
      disabled={option.disabled}
      onClick={() => onSelect(option.index)}
      size="sm"
      title={option.selected ? "Recorded. Click to undo." : undefined}
      type="button"
      variant={option.selected ? "default" : "outline"}
    >
      <span className="font-semibold tabular-nums">{option.number}.</span>
      <span className="min-w-0 [overflow-wrap:anywhere]">{option.text}</span>
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
