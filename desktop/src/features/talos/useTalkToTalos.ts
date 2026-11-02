import { useIsMutating, useMutation, useQuery } from "@tanstack/react-query";
import { useLocation } from "@tanstack/react-router";
import * as React from "react";
import { toast } from "sonner";

import { useOpenDmMutation } from "@/features/channels/hooks";
import { useRelayMembersQuery } from "@/features/community-members/hooks";
import { useHuddle } from "@/features/huddle";
import { formatHuddleActionError } from "@/features/huddle/lib/huddleError";
import { invokeTauri } from "@/shared/api/tauri";
import { TALK_TO_TALOS_EVENT } from "@/shared/lib/keyboard-shortcuts";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { normalizePubkey } from "@/shared/lib/pubkey";

const TALK_MUTATION_KEY = ["talkToTalos"] as const;

const NOT_SET_UP = "Talk to Talos isn't set up yet:";

/** Hidden until the pinned key is a community member (setup step S4). */
function useTalosState() {
  const pin = useQuery({
    queryKey: ["talosPubkey"],
    queryFn: () => invokeTauri<string | null>("get_talos_pubkey"),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const members = useRelayMembersQuery(Boolean(pin.data));
  const pubkey = pin.data ? normalizePubkey(pin.data) : null;
  const isMember = members.data?.some(
    (member) => normalizePubkey(member.pubkey) === pubkey,
  );
  if (isMember) return { pubkey, why: "", recheck: () => {} };
  let why = `${NOT_SET_UP} Talos's key isn't a member of this community (setup step S4).`;
  if (pin.isSuccess && !pubkey) {
    why = `${NOT_SET_UP} this build has no Talos key (setup step S4).`;
  } else if (!members.data) {
    why =
      "Still checking whether Talos is in this community. Try again in a moment.";
  }
  // A press reads the list again, so a key admitted since is seen without a restart.
  return { pubkey: null, why, recheck: () => void members.refetch() };
}

/**
 * Only these surfaces, first match wins. An allowlist, so Settings and dialogs
 * (a revealed key) can never be sent.
 */
function visibleSurface() {
  const find = (selector: string) =>
    document.querySelector<HTMLElement>(selector);
  const cards = document.querySelectorAll<HTMLElement>("[data-bench-item]");
  return (
    find('[data-testid="message-thread-body"]') ??
    find("[data-bench-focused] [data-bench-item]") ??
    [...cards].find((card) => card.getBoundingClientRect().top >= 0) ??
    find('[data-testid="message-timeline"]')
  );
}

/**
 * Talos refuses the whole click unless each id matches this pattern, and Bench
 * card ids hold slashes (`steward/hold-talos-7`).
 */
function shortId(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const id = value.replaceAll("/", ":");
  return /^[\w.:-]{1,128}$/.test(id) ? id : null;
}

function talkContext(pathname: string, search: Record<string, unknown>) {
  const [, view, id] = pathname.split("/");
  const surface = visibleSurface();
  return {
    route: shortId(view || "home"),
    channel_id: view === "channels" ? shortId(id) : null,
    thread_root: shortId(search.threadRootId ?? search.thread),
    card_id: shortId(surface?.dataset.benchItem),
    project_id: view === "projects" ? shortId(id) : null,
    snapshot: surface?.innerText.trim() || null,
  };
}

export function useTalkToTalos() {
  const talos = useTalosState();
  const { startHuddle } = useHuddle();
  const location = useLocation();
  const { mutateAsync: openDm } = useOpenDmMutation();
  const pending = useIsMutating({ mutationKey: TALK_MUTATION_KEY }) > 0;
  const { mutate } = useMutation({
    mutationKey: TALK_MUTATION_KEY,
    mutationFn: async (pubkey: string) => {
      const huddle = await invokeTauri<{
        phase: string;
        participants: string[];
      } | null>("get_huddle_state");
      const phase = huddle?.phase ?? "idle";
      const live = phase === "connected" || phase === "active";
      if (!live && phase !== "idle") {
        throw new Error(
          "A huddle is starting or ending. Try again when it's done.",
        );
      }
      if (
        live &&
        huddle?.participants.some((p) => normalizePubkey(p) === pubkey)
      ) {
        toast("Talos is already in this huddle.");
        return;
      }
      await invokeTauri("talk_to_talos", {
        context: talkContext(
          location.pathname,
          location.search as Record<string, unknown>,
        ),
      });
      if (live) {
        await invokeTauri("add_talos_to_huddle");
      } else {
        // In the Talos DM, never the channel on screen: nobody else is rung.
        const dm = await openDm({ pubkeys: [pubkey] });
        await startHuddle(dm.id, []);
      }
      toast(
        "Talos was asked to join. It should be here in a few seconds. If it isn't, it may be offline; your huddle stays open.",
      );
    },
    onError: (error) => {
      toast.error(formatHuddleActionError(error, "start"));
    },
  });

  const talk = () => {
    if (!talos.pubkey) {
      toast(talos.why);
      talos.recheck();
    } else if (!pending) {
      mutate(talos.pubkey);
    }
  };
  return { pubkey: talos.pubkey, pending, talk };
}

/** The shortcut and the tray item. They explain themselves while the button is hidden. */
export function TalkToTalosTriggers() {
  const { talk } = useTalkToTalos();
  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (!hasPrimaryShortcutModifier(event) || !event.shiftKey) return;
      if (event.altKey || event.repeat || event.code !== "KeyT") return;
      event.preventDefault();
      talk();
    }
    window.addEventListener("keydown", handleKeyDown, { capture: true });
    window.addEventListener(TALK_TO_TALOS_EVENT, talk);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
      window.removeEventListener(TALK_TO_TALOS_EVENT, talk);
    };
  }, [talk]);
  return null;
}
