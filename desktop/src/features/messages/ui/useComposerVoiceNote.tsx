import * as React from "react";
import { toast } from "sonner";

import {
  isVoiceNoteAttachment,
  isVoiceNoteFile,
  VOICE_NOTE_MAX_DURATION_SECONDS,
} from "@/features/messages/lib/audioAttachment";
import { useVoiceNoteRecorder } from "@/features/messages/lib/useVoiceNoteRecorder";
import {
  transcribeVoiceNote,
  voiceNoteWithoutWordsMessage,
} from "@/features/messages/lib/voiceNoteTranscript";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import { VoiceNoteRecorder } from "./VoiceNoteRecorder";

export function useComposerVoiceNote({
  draftKey,
  editTargetId,
  media,
  onTranscript,
  setEmojiPickerOpen,
  setFormattingOpen,
}: {
  draftKey: string | null | undefined;
  editTargetId: string | null;
  media: MediaUploadController;
  /** Puts a finished note's words in the composer, so they send as text. */
  onTranscript: (text: string) => void;
  setEmojiPickerOpen: (open: boolean) => void;
  setFormattingOpen: (open: boolean) => void;
}) {
  const recorder = useVoiceNoteRecorder();
  const limitReachedRef = React.useRef(false);
  // Transcribing keeps the recorder in "processing", so Send waits for the
  // words instead of posting audio an agent cannot hear.
  const [transcribing, setTranscribing] = React.useState(false);
  const transcriptionRef = React.useRef(0);
  const onTranscriptRef = React.useRef(onTranscript);
  onTranscriptRef.current = onTranscript;
  const status = transcribing ? "processing" : recorder.status;
  const statusRef = React.useRef(status);
  statusRef.current = status;
  const getAttachments = React.useCallback(
    () => ({
      pending: media.pendingImetaRef.current,
      queued: media.queuedAttachmentsRef.current,
    }),
    [media.pendingImetaRef, media.queuedAttachmentsRef],
  );
  const getAttachmentsRef = React.useRef(getAttachments);
  getAttachmentsRef.current = getAttachments;
  const onBeforeStartRef = React.useRef(() => {});
  onBeforeStartRef.current = () => {
    setEmojiPickerOpen(false);
    setFormattingOpen(false);
  };
  const currentContextRef = React.useRef({ draftKey, editTargetId });
  currentContextRef.current = { draftKey, editTargetId };
  const recordingContextRef = React.useRef({ draftKey, editTargetId });

  const cancel = React.useCallback(() => {
    transcriptionRef.current += 1;
    setTranscribing(false);
    recorder.cancel();
  }, [recorder.cancel]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: composer identity fields are the cancellation triggers
  React.useEffect(() => {
    cancel();
  }, [draftKey, editTargetId]);

  React.useEffect(() => {
    if (recorder.error) toast.error(recorder.error);
  }, [recorder.error]);

  // A late transcript must never reach an unmounted composer.
  React.useEffect(
    () => () => {
      transcriptionRef.current += 1;
    },
    [],
  );

  const finish = React.useCallback(async () => {
    const recording = await recorder.stop();
    const inRecordingContext = () =>
      recordingContextRef.current.draftKey ===
        currentContextRef.current.draftKey &&
      recordingContextRef.current.editTargetId ===
        currentContextRef.current.editTargetId;
    if (!recording || !inRecordingContext()) return recording;
    // Words first, then the audio: a Discard while transcribing (cancel bumps
    // the token) drops both, and nothing lands in a draft it was not made in.
    const transcription = ++transcriptionRef.current;
    const current = () =>
      transcription === transcriptionRef.current && inRecordingContext();
    setTranscribing(true);
    let text = "";
    let failure: string | null = null;
    try {
      text = await transcribeVoiceNote(recording.file);
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
    if (!current()) return null;
    try {
      await media.uploadFile(recording.file);
      if (!current()) return null;
      if (text) onTranscriptRef.current(text);
      else
        toast.warning(
          voiceNoteWithoutWordsMessage(failure ?? "no words were heard"),
        );
    } finally {
      if (transcription === transcriptionRef.current) setTranscribing(false);
    }
    return recording;
  }, [recorder.stop, media.uploadFile]);

  React.useEffect(() => {
    if (recorder.status === "idle") limitReachedRef.current = false;
    if (
      recorder.status === "recording" &&
      recorder.elapsedSeconds >= VOICE_NOTE_MAX_DURATION_SECONDS &&
      !limitReachedRef.current
    ) {
      limitReachedRef.current = true;
      void finish();
    }
  }, [finish, recorder.elapsedSeconds, recorder.status]);

  const toggle = React.useCallback(() => {
    if (statusRef.current === "recording") {
      void finish();
      return;
    }
    const attachments = getAttachmentsRef.current();
    if (attachments.pending.length > 0 || attachments.queued.length > 0) {
      toast.error("A voice note must be the only attachment.");
      return;
    }
    recordingContextRef.current = currentContextRef.current;
    onBeforeStartRef.current();
    void recorder.start();
  }, [finish, recorder.start]);

  const attachments = getAttachments();
  const hasAttachment =
    attachments.pending.some((attachment) =>
      isVoiceNoteAttachment({
        filename: attachment.filename,
        m: attachment.type,
      }),
    ) || attachments.queued.some(({ file }) => isVoiceNoteFile(file));
  const hasAttachmentRef = React.useRef(hasAttachment);
  hasAttachmentRef.current = hasAttachment;

  const acceptsNewAttachment = React.useCallback(() => {
    const attachments = getAttachmentsRef.current();
    const hasVoiceNoteAttachment =
      attachments.pending.some((attachment) =>
        isVoiceNoteAttachment({
          filename: attachment.filename,
          m: attachment.type,
        }),
      ) || attachments.queued.some(({ file }) => isVoiceNoteFile(file));
    if (statusRef.current !== "idle" || hasVoiceNoteAttachment) {
      toast.error(
        statusRef.current === "idle"
          ? "A voice note must be the only attachment."
          : "Finish or discard the voice note before attaching a file.",
      );
      return false;
    }
    return true;
  }, []);

  const uploadFileWhenIdle = React.useCallback(
    async (file: File) => {
      if (!acceptsNewAttachment()) return;
      await media.uploadFile(file);
    },
    [acceptsNewAttachment, media.uploadFile],
  );
  const setPendingImetaWhenIdle = React.useCallback(
    (update: Parameters<typeof media.setPendingImeta>[0]) => {
      if (acceptsNewAttachment()) media.setPendingImeta(update);
    },
    [acceptsNewAttachment, media.setPendingImeta],
  );

  return {
    ...recorder,
    cancel,
    status,
    acceptsAttachment: status === "idle" && !hasAttachment,
    hasAttachment,
    hasAttachmentRef,
    isIdle: status === "idle",
    recorderElement:
      status === "idle" ? null : (
        <VoiceNoteRecorder
          elapsedSeconds={recorder.elapsedSeconds}
          levels={recorder.levels}
          maxDurationSeconds={VOICE_NOTE_MAX_DURATION_SECONDS}
          onCancel={cancel}
          processing={status === "processing"}
          requesting={status === "requesting"}
        />
      ),
    statusRef,
    setPendingImetaWhenIdle,
    finish,
    toggle,
    uploadFileWhenIdle,
  };
}
