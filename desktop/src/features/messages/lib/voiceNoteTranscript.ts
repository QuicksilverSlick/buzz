import { invoke } from "@tauri-apps/api/core";

/**
 * Transcribe a recorded voice note with the local speech model.
 *
 * A voice note posts as audio only, and agents read text, so without its
 * words an agent gets a player it cannot hear. Resolves to "" when no speech
 * was heard; rejects with a reason the composer can show.
 */
export async function transcribeVoiceNote(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const text = await invoke<string>("transcribe_voice_note", bytes);
  return typeof text === "string" ? text.trim() : "";
}

/** The toast for a voice note that will send without its words. */
export function voiceNoteWithoutWordsMessage(reason: string): string {
  return `Couldn't turn this voice note into text (${reason}). It will send as audio only, and agents can't hear audio, so type what you said if an agent needs it.`;
}
