import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { Textarea } from "@/shared/ui/textarea";
import { PERSONA_LABEL_OPTIONAL_CLASS } from "./agentConfigOptions";
import type { VoiceDraft } from "./personaBehaviorDraft";

const TEXT_FIELDS = [
  {
    key: "voice",
    label: "Voice",
    placeholder: "marin",
    help: "A GPT-Live voice name or custom voice id. Empty uses the default.",
  },
  {
    key: "persona",
    label: "How it sounds",
    placeholder: "Warm and brief. Answers in a sentence or two.",
  },
  {
    key: "aliases",
    label: "Also answers to",
    placeholder: "Orch, O",
    help: "Other names, separated by commas. Its own name always works.",
  },
] as const;

/**
 * Huddle voice for a definition (huddle voices, slice 9). Off by default.
 * The live voice lands later; today the switch only stops speech in this
 * agent's huddles from also being posted as text. `voiceWithoutOwner` stays
 * off until Phase 4, so it has no control yet.
 */
export function VoiceProfileField({
  value,
  onChange,
  disabled,
}: {
  value: VoiceDraft;
  onChange: (next: VoiceDraft) => void;
  disabled?: boolean;
}) {
  return (
    <fieldset className="space-y-3" disabled={disabled}>
      <div className="flex items-start gap-3">
        <Switch
          aria-describedby="persona-voice-enabled-description"
          checked={value.enabled}
          className="mt-0.5"
          id="persona-voice-enabled"
          onCheckedChange={(enabled) => onChange({ ...value, enabled })}
        />
        <label className="min-w-0 flex-1" htmlFor="persona-voice-enabled">
          <span className="block text-sm font-medium text-foreground">
            Huddle voice
          </span>
          <span
            className="block text-xs text-muted-foreground"
            id="persona-voice-enabled-description"
          >
            Lets this agent talk in huddles in its own voice. While it is in a
            huddle, what you say there is not also posted as text.
          </span>
        </label>
      </div>
      {value.enabled
        ? TEXT_FIELDS.map((field) => {
            const id = `persona-voice-${field.key}`;
            const Control = field.key === "persona" ? Textarea : Input;
            return (
              <div className="space-y-1.5 pl-12" key={field.key}>
                <label className="text-sm text-foreground" htmlFor={id}>
                  {field.label}
                  <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
                </label>
                <Control
                  id={id}
                  maxLength={field.key === "persona" ? 1000 : undefined}
                  onChange={(event) =>
                    onChange({ ...value, [field.key]: event.target.value })
                  }
                  placeholder={field.placeholder}
                  spellCheck={false}
                  value={value[field.key]}
                />
                {"help" in field ? (
                  <p className="text-xs text-muted-foreground">{field.help}</p>
                ) : null}
              </div>
            );
          })
        : null}
    </fieldset>
  );
}
