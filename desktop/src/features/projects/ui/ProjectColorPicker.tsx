import { PROJECT_COLORS } from "@/features/projects/lib/projectColors";
import { cn } from "@/shared/lib/cn";

/**
 * The eight project colors as native radios sharing one name: the browser
 * gives one Tab stop and arrow keys between them. The ring marks the pick
 * and its name shows beside the swatches, so color is never the only sign.
 */
export function ProjectColorPicker({
  disabled,
  legendClassName,
  name,
  onChange,
  title,
  value,
}: {
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
