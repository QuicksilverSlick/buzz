import * as React from "react";

import type { Project } from "@/features/projects/projectModels";
import {
  PROJECT_COLORS,
  projectColorOf,
} from "@/features/projects/lib/projectColors";
import { cn } from "@/shared/lib/cn";

type ColorTarget = Pick<Project, "color" | "dtag" | "id">;

/**
 * Saves color picks one at a time, and only the latest: a pick made while a
 * save runs waits, replacing any pick already waiting. The radios stay
 * enabled meanwhile, so arrow keys keep focus and move on, and arrowing
 * across the palette publishes twice at most, not once per step. A pick
 * keeps the project it was made on: the same setting reused for another
 * project (a route change) never shows or saves it there.
 */
export function useQueuedColorSave<T extends ColorTarget>(
  target: T,
  save: (color: string, target: T) => Promise<T>,
): { pick: (color: string) => void; saving: boolean; value: string } {
  type Queued = { color: string; target: T };
  const [choice, setChoice] = React.useState<Queued | null>(null);
  const [saving, setSaving] = React.useState(false);
  const busy = React.useRef(false);
  const waiting = React.useRef<Queued | null>(null);
  const pick = (color: string) => {
    setChoice({ color, target });
    if (busy.current) {
      waiting.current = { color, target };
      return;
    }
    busy.current = true;
    setSaving(true);
    void (async () => {
      let next: Queued | null = { color, target };
      let saved = target;
      // The project just saved builds on that head; another on its own.
      const head = (queued: Queued) =>
        queued.target.id === saved.id ? saved : queued.target;
      try {
        while (next !== null) {
          saved = await save(next.color, head(next));
          next = waiting.current;
          waiting.current = null;
          if (next && next.color === projectColorOf(head(next)).id) next = null;
        }
      } catch {
        // `save` reports its own failure; a waiting pick is dropped with it.
      } finally {
        busy.current = false;
        waiting.current = null;
        setSaving(false);
        setChoice(null);
      }
    })();
  };
  return {
    pick,
    saving,
    value:
      choice?.target.id === target.id
        ? choice.color
        : projectColorOf(target).id,
  };
}

/**
 * The eight project colors as native radios sharing one name: the browser
 * gives one Tab stop and arrow keys between them. The ring marks the pick
 * and its name shows beside the swatches, so color is never the only sign.
 */
export function ProjectColorPicker({
  busy,
  disabled,
  legendClassName,
  name,
  onChange,
  title,
  value,
}: {
  busy?: boolean;
  disabled?: boolean;
  legendClassName?: string;
  name: string;
  onChange: (color: string) => void;
  title?: string;
  value: string;
}) {
  const selected = PROJECT_COLORS.find((color) => color.id === value);
  return (
    <fieldset
      aria-busy={busy || undefined}
      className="min-w-0 space-y-1.5 disabled:opacity-60"
      data-testid="project-color-picker"
      disabled={disabled}
      title={title}
    >
      <legend
        className={cn("text-sm font-medium text-foreground", legendClassName)}
      >
        Color
      </legend>
      <div className="flex flex-wrap items-center gap-2">
        {PROJECT_COLORS.map((color) => (
          <label
            className="relative inline-flex cursor-pointer"
            key={color.id}
            title={color.label}
          >
            <input
              checked={color.id === value}
              className="peer sr-only"
              name={name}
              onChange={() => onChange(color.id)}
              type="radio"
              value={color.id}
            />
            <span
              aria-hidden
              className={cn(
                "size-5 rounded-full ring-offset-2 ring-offset-background peer-checked:ring-2 peer-checked:ring-foreground peer-focus-visible:outline-2 peer-focus-visible:outline-offset-4 peer-focus-visible:outline-ring",
                color.dot,
              )}
            />
            <span className="sr-only">{color.label}</span>
          </label>
        ))}
        {selected ? (
          <span aria-hidden className="ml-1 text-xs text-muted-foreground">
            {selected.label}
          </span>
        ) : null}
      </div>
    </fieldset>
  );
}
