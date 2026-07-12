import { Loader2, Mic, Radio, Send, Square, X } from "lucide-react";
import clsx from "clsx";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { runCuratorAgent, type CuratorAgentStatus } from "../features/curator/curatorAgent";
import { englishTrackReference, guardCuratorReply } from "../features/curator/curatorLanguageGuard";
import {
  askCurator,
  recordCuratorSignal,
  type DjNote,
  sanitizeCuratorReply,
} from "../features/curator/djCurator";
import {
  getSpeechProviderConfig,
  speakCuratorText,
  startSpeechRecording,
  transcribeRecordedSpeech,
  type SpeechRecordingSession,
} from "../features/speech/provider";
import type { MusicSourceSong } from "../features/musicSources/provider";
import type { RadioKind, RadioSession } from "../features/radio/omeRadio";
import type { Track } from "../types/music";

interface DjCuratorPanelProps {
  track: Track | null;
  tracks: Track[];
  isPlaying: boolean;
  volume: number;
  onPlayLocal: (track: Track) => void;
  onPlayNetEase: (song: MusicSourceSong) => Promise<boolean>;
  onImportNetEase: (song: MusicSourceSong) => Promise<Track | null>;
  onQueueLocal: (track: Track) => void;
  onPlayTemporaryPlaylist: (title: string, tracks: Track[]) => void;
  onPlayNextSoftly: (mood?: Track["moods"][number]) => void;
  onPlayPrevious: () => void;
  onTogglePlay: () => void;
  onSetVolume: (volume: number) => void;
  onCreateRadioSession: (args: {
    kind?: RadioKind;
    theme?: string;
    mood?: Track["moods"][number];
    scene?: string;
  }) => RadioSession | null;
  onStartRadioSession: (sessionId?: string) => boolean;
  onSoftenRadio: (level?: number) => RadioSession | null;
  onBrightenRadio: (level?: number) => RadioSession | null;
  onSwitchRadioTheme: (kind: RadioKind, theme?: string) => RadioSession | null;
  onDuckingChange: (enabled: boolean) => void;
}

const openingNote: DjNote = {
  id: "opening",
  role: "curator",
  text: "Good evening. Ask for a song, a mood, or a little silence. I will mind the records.",
  createdAt: new Date().toISOString(),
};

type VoiceInputStatus = "idle" | "recording" | "stopping" | "transcribing" | "ready" | "failed";

export function DjCuratorPanel({
  track,
  tracks,
  isPlaying,
  volume,
  onPlayLocal,
  onPlayNetEase,
  onImportNetEase,
  onQueueLocal,
  onPlayTemporaryPlaylist,
  onPlayNextSoftly,
  onPlayPrevious,
  onTogglePlay,
  onSetVolume,
  onCreateRadioSession,
  onStartRadioSession,
  onSoftenRadio,
  onBrightenRadio,
  onSwitchRadioTheme,
  onDuckingChange,
}: DjCuratorPanelProps) {
  const [isOpen, setOpen] = useState(false);
  const [input, setInput] = useState("");
  const [notes, setNotes] = useState<DjNote[]>([openingNote]);
  const [isThinking, setThinking] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<VoiceInputStatus>("idle");
  const [waveform, setWaveform] = useState<number[]>(() => Array.from({ length: 15 }, () => 0.06));
  const [agentStatus, setAgentStatus] = useState<CuratorAgentStatus>("Listening...");
  const [voiceHint, setVoiceHint] = useState<string | null>(null);
  const notesRef = useRef<HTMLDivElement | null>(null);
  const recordingRef = useRef<SpeechRecordingSession | null>(null);
  const recordingTimeoutRef = useRef<number | null>(null);
  const scrollFrameRef = useRef<number | null>(null);
  const isNearBottomRef = useRef(true);
  const lastWaveformUpdateRef = useRef(0);
  const agentStatusTimerRef = useRef<number | null>(null);

  const currentWhisper = useMemo(() => {
    if (!track) return "Waiting for the first record.";
    return `${englishTrackReference(track)} is on the turntable.`;
  }, [track]);

  useEffect(() => {
    if (!notesRef.current || !isNearBottomRef.current) return;
    if (scrollFrameRef.current !== null) window.cancelAnimationFrame(scrollFrameRef.current);
    scrollFrameRef.current = window.requestAnimationFrame(() => {
      notesRef.current?.scrollTo({ top: notesRef.current.scrollHeight, behavior: "smooth" });
    });
    return () => {
      if (scrollFrameRef.current !== null) window.cancelAnimationFrame(scrollFrameRef.current);
    };
  }, [notes.length]);

  useEffect(() => {
    if (!isOpen || !notesRef.current) return;
    const frame = window.requestAnimationFrame(() => {
      notesRef.current?.scrollTo({ top: notesRef.current.scrollHeight });
      isNearBottomRef.current = true;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [isOpen]);

  useEffect(
    () => () => {
      recordingRef.current?.cancel();
      recordingRef.current = null;
      if (recordingTimeoutRef.current !== null) window.clearTimeout(recordingTimeoutRef.current);
      if (agentStatusTimerRef.current !== null) window.clearTimeout(agentStatusTimerRef.current);
    },
    [],
  );

  const speak = (text: string) => {
    void speakCuratorText(text, getSpeechProviderConfig(), {
      onStart: () => onDuckingChange(true),
      onEnd: () => onDuckingChange(false),
      onError: () => onDuckingChange(false),
    });
  };

  const submit = async (overrideText?: string) => {
    const text = (overrideText ?? input).trim();
    if (!text || isThinking) return;
    if (agentStatusTimerRef.current !== null) {
      window.clearTimeout(agentStatusTimerRef.current);
      agentStatusTimerRef.current = null;
    }

    const listenerNote: DjNote = {
      id: crypto.randomUUID(),
      role: "listener",
      text,
      createdAt: new Date().toISOString(),
    };
    const nextNotes = [...notes, listenerNote];
    setNotes(nextNotes);
    setInput("");
    setThinking(true);
    setAgentStatus("Listening...");

    try {
      void recordCuratorSignal(text).catch((error) =>
        console.warn("curator signal was not recorded", error),
      );
      const agentResult = await runCuratorAgent(text, {
        currentTrack: track,
        localTracks: tracks,
        isPlaying,
        volume,
        playLocalTrack: onPlayLocal,
        playNetEaseSong: onPlayNetEase,
        importNetEaseSong: onImportNetEase,
        queueTrack: onQueueLocal,
        playTemporaryPlaylist: onPlayTemporaryPlaylist,
        playNextTrack: onPlayNextSoftly,
        playPreviousTrack: onPlayPrevious,
        togglePlayback: onTogglePlay,
        setVolume: onSetVolume,
        createRadioSession: onCreateRadioSession,
        startRadioSession: onStartRadioSession,
        softenRadio: onSoftenRadio,
        brightenRadio: onBrightenRadio,
        switchRadioTheme: onSwitchRadioTheme,
      });
      setAgentStatus(agentResult.status);
      const replyText = agentResult.handled
        ? sanitizeCuratorReply(agentResult.reply)
        : sanitizeCuratorReply(await askCurator(text, track, nextNotes));
      const reply = guardCuratorReply(replyText, {
        track,
        originalTrackTitle: getAgentTrackTitle(agentResult),
      });
      setNotes((value) => [
        ...value,
        {
          id: crypto.randomUUID(),
          role: "curator",
          text: reply.displayText,
          createdAt: new Date().toISOString(),
        },
      ]);
      speak(reply.spokenText);
    } catch {
      const fallbackReply = guardCuratorReply(
        "The booth caught a little dust there. Say it once more, and I will keep it simple.",
        { track },
      );
      setNotes((value) => [
        ...value,
        {
          id: crypto.randomUUID(),
          role: "curator",
          text: fallbackReply.displayText,
          createdAt: new Date().toISOString(),
        },
      ]);
      speak(fallbackReply.spokenText);
    } finally {
      setThinking(false);
      agentStatusTimerRef.current = window.setTimeout(() => {
        agentStatusTimerRef.current = null;
        setAgentStatus("Listening...");
      }, 1600);
    }
  };

  const startVoiceChat = async () => {
    if (
      recordingRef.current ||
      isThinking ||
      voiceStatus === "stopping" ||
      voiceStatus === "transcribing"
    )
      return;
    setVoiceHint(null);
    setVoiceStatus("idle");
    try {
      const session = await startSpeechRecording((level) => {
        const now = performance.now();
        if (now - lastWaveformUpdateRef.current < 90) return;
        lastWaveformUpdateRef.current = now;
        setWaveform((current) =>
          current.map((_, index) => {
            const shape = 0.42 + Math.abs(Math.sin((index + 1) * 1.37)) * 0.58;
            return Math.max(0.06, Math.min(1, level * shape));
          }),
        );
      });
      recordingRef.current = session;
      setVoiceStatus("recording");
      recordingTimeoutRef.current = window.setTimeout(() => {
        setVoiceHint("The microphone has reached one minute. Finishing this note now.");
        void stopVoiceChat();
      }, 60_000);
    } catch (error) {
      setVoiceStatus("failed");
      setVoiceHint(friendlyVoiceError(error));
    }
  };

  const stopVoiceChat = async () => {
    const session = recordingRef.current;
    if (!session || voiceStatus === "stopping" || voiceStatus === "transcribing") return;
    recordingRef.current = null;
    if (recordingTimeoutRef.current !== null) {
      window.clearTimeout(recordingTimeoutRef.current);
      recordingTimeoutRef.current = null;
    }
    setVoiceStatus("stopping");
    try {
      const clip = await session.stop();
      setVoiceStatus("transcribing");
      const transcript = await transcribeRecordedSpeech(clip, getSpeechProviderConfig());
      setInput(transcript);
      setVoiceStatus("ready");
      await submit(transcript);
      setVoiceStatus("idle");
    } catch (error) {
      setVoiceStatus("failed");
      setVoiceHint(friendlyVoiceError(error));
    }
  };

  const cancelVoiceChat = useCallback(() => {
    recordingRef.current?.cancel();
    recordingRef.current = null;
    if (recordingTimeoutRef.current !== null) window.clearTimeout(recordingTimeoutRef.current);
    recordingTimeoutRef.current = null;
    setVoiceStatus("idle");
    setWaveform(Array.from({ length: 15 }, () => 0.06));
  }, []);

  const openPanel = () => setOpen(true);

  const closePanel = useCallback(() => {
    if (recordingRef.current) cancelVoiceChat();
    setOpen(false);
  }, [cancelVoiceChat]);

  useEffect(() => {
    if (!isOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closePanel();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [closePanel, isOpen]);

  return (
    <>
      <button
        type="button"
        onClick={openPanel}
        data-danmaku-safe-zone="curator-dock"
        className={clsx(
          "ambient-dj-dock group fixed right-0 top-1/2 z-40 flex h-[76px] -translate-y-1/2 items-center overflow-hidden rounded-l-[24px] border border-r-0 border-white/[0.16] text-left text-[#4a2108] transition-[width,transform,opacity,background,box-shadow] duration-300 ease-out",
          isOpen
            ? "pointer-events-none w-12 translate-x-full opacity-0"
            : "pointer-events-auto w-12 translate-x-0 opacity-60 hover:w-[150px] hover:opacity-100",
        )}
        aria-label="Open Private DJ"
        aria-expanded={isOpen}
      >
        <span className="relative ml-2.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-[#4a2108]/[0.07] text-[#4a2108]/54 transition-[background,color,transform] duration-300 group-hover:scale-105 group-hover:bg-[#4a2108]/[0.11] group-hover:text-[#4a2108]/78">
          <Radio className="h-4 w-4" />
          <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-[#7e4a2e]/55 shadow-[0_0_10px_rgba(126,74,46,0.34)]" />
        </span>
        <span className="ml-3 min-w-[84px] translate-x-2 opacity-0 transition-[transform,opacity] duration-300 group-hover:translate-x-0 group-hover:opacity-100">
          <span className="block text-[11px] font-black tracking-[0.04em] text-[#4a2108]/70">
            Ome Radio
          </span>
          <span className="mt-0.5 block whitespace-nowrap text-[9px] font-semibold tracking-[0.04em] text-[#4a2108]/32">
            Tune the mood
          </span>
        </span>
      </button>

      {isOpen && (
        <div
          className="fixed inset-0 z-[39] bg-[#4a2108]/[0.015]"
          onPointerDown={closePanel}
          aria-hidden="true"
        />
      )}

      <aside
        data-danmaku-safe-zone="curator"
        className={clsx(
          "curator-panel fixed right-4 top-1/2 z-40 w-[min(390px,calc(100vw-32px))] -translate-y-1/2 transition-[transform,opacity] duration-300 ease-out",
          isOpen
            ? "pointer-events-auto translate-x-0 opacity-100"
            : "pointer-events-none translate-x-[calc(100%+2rem)] opacity-0",
        )}
        aria-hidden={!isOpen}
      >
        <div
          className={clsx(
            "curator-panel-shell flex max-h-[calc(100svh-32px)] flex-col overflow-hidden rounded-[28px] border border-white/[0.15] bg-[#eadbcd]/[0.56] shadow-[0_24px_72px_rgba(74,33,8,0.13)] transition-[backdrop-filter,background] duration-300",
            isOpen ? "backdrop-blur-[28px]" : "backdrop-blur-none",
          )}
        >
          <div className="flex shrink-0 items-center gap-3 px-3.5 py-3.5 text-[#4a2108]">
            <button
              type="button"
              onClick={closePanel}
              className="app-transition flex h-9 w-9 items-center justify-center rounded-full bg-[#4a2108]/[0.10] text-[#4a2108]/62 hover:bg-[#4a2108]/[0.16] hover:text-[#4a2108]"
              aria-label="Close Private DJ"
            >
              <X className="h-4 w-4" />
            </button>
            <div className="min-w-0">
              <p className="text-xs font-semibold text-[#4a2108]/46">On Air</p>
              <h2 className="truncate text-[15px] font-black text-[#4a2108]">Ome Radio</h2>
            </div>
          </div>

          <div
            className={clsx(
              "curator-panel-body min-h-0 shrink-0 overflow-hidden transition-opacity duration-200 ease-out",
              isOpen ? "pointer-events-auto opacity-100" : "pointer-events-none opacity-0",
            )}
            aria-hidden={!isOpen}
          >
            <div className="flex h-full min-h-0 flex-col px-5 pb-5 pt-3">
              <div className="mb-3 flex shrink-0 items-center justify-between gap-3">
                <p className="min-w-0 truncate text-xs leading-5 text-[#4a2108]/42">
                  {currentWhisper}
                </p>
                <p className="w-24 shrink-0 truncate text-right text-[10px] font-black uppercase tracking-[0.16em] text-[#4a2108]/32">
                  {agentStatus}
                </p>
              </div>

              <div
                ref={notesRef}
                onScroll={(event) => {
                  const target = event.currentTarget;
                  isNearBottomRef.current =
                    target.scrollHeight - target.scrollTop - target.clientHeight < 56;
                }}
                className="curator-notes min-h-0 flex-1 touch-pan-y space-y-4 overflow-y-auto overscroll-contain pr-2"
              >
                {notes.map((note) => (
                  <div
                    key={note.id}
                    className={clsx(
                      "curator-note",
                      note.role === "listener" && "curator-note-listener",
                    )}
                  >
                    <p className="text-[10px] font-semibold uppercase tracking-[0.14em] text-[#4a2108]/26">
                      {note.role === "curator" ? "On air" : "You"}
                    </p>
                    <p className="mt-1.5 text-[15px] leading-7 text-[#4a2108]/76">{note.text}</p>
                  </div>
                ))}
                {isThinking && (
                  <div className="flex items-center gap-2 text-sm text-[#4a2108]/46">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Checking the shelf
                  </div>
                )}
              </div>

              <div className="mt-2 flex h-11 shrink-0 items-center gap-3 overflow-hidden rounded-[18px] bg-[#4a2108]/[0.035] px-3">
                <VoiceWaveform
                  levels={waveform}
                  active={voiceStatus === "recording"}
                  busy={voiceStatus === "stopping" || voiceStatus === "transcribing"}
                />
                <span className="min-w-0 flex-1 truncate text-[10px] font-black uppercase tracking-[0.12em] text-[#4a2108]/38">
                  {voiceStatusLabel(voiceStatus)}
                </span>
              </div>

              <div className="h-6 shrink-0 pt-1">
                <p
                  className={clsx(
                    "truncate text-xs leading-5 text-[#4a2108]/48 transition-opacity duration-300",
                    voiceHint ? "opacity-100" : "opacity-0",
                  )}
                >
                  {voiceHint ?? "The microphone is ready."}
                </p>
              </div>

              <div className="mt-2 flex h-[76px] shrink-0 items-end gap-2 rounded-[24px] bg-[#4a2108]/[0.05] p-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.11)]">
                <textarea
                  value={input}
                  onChange={(event) => setInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.shiftKey) {
                      event.preventDefault();
                      void submit();
                    }
                  }}
                  placeholder="Ask for a song, a mood, or silence..."
                  rows={2}
                  className="h-[58px] min-w-0 flex-1 resize-none bg-transparent px-3 py-2 text-sm leading-5 text-[#4a2108] outline-none placeholder:text-[#4a2108]/32"
                />
                <button
                  type="button"
                  onClick={() =>
                    voiceStatus === "recording" ? void stopVoiceChat() : void startVoiceChat()
                  }
                  disabled={
                    voiceStatus === "stopping" || voiceStatus === "transcribing" || isThinking
                  }
                  className={clsx(
                    "curator-control-button",
                    voiceStatus === "recording" && "bg-[#4a2108]/[0.18] text-[#4a2108]",
                  )}
                  aria-label={voiceStatus === "recording" ? "Stop recording" : "Start recording"}
                >
                  {voiceStatus === "recording" ? (
                    <Square className="h-3.5 w-3.5 fill-current" />
                  ) : voiceStatus === "stopping" || voiceStatus === "transcribing" ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Mic className="h-4 w-4" />
                  )}
                </button>
                <button
                  type="button"
                  onClick={() => void submit()}
                  disabled={!input.trim() || isThinking}
                  className="curator-send-button"
                  aria-label="Send note"
                >
                  <Send className="h-4 w-4" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </aside>
    </>
  );
}

function getAgentTrackTitle(
  agentResult: Awaited<ReturnType<typeof runCuratorAgent>>,
): string | null {
  if (agentResult.debug.selectedTrack) return agentResult.debug.selectedTrack;
  const candidate = agentResult.candidates?.[0];
  return candidate?.title ?? null;
}

const VoiceWaveform = memo(function VoiceWaveform({
  levels,
  active,
  busy,
}: {
  levels: number[];
  active: boolean;
  busy: boolean;
}) {
  return (
    <div
      className={clsx(
        "flex h-8 w-[112px] shrink-0 items-center justify-center gap-[3px] transition-opacity duration-300",
        active || busy ? "opacity-70" : "opacity-24",
      )}
      aria-hidden="true"
    >
      {levels.map((level, index) => (
        <span
          key={index}
          className={clsx(
            "w-[3px] rounded-full bg-[#4a2108]/55 transition-[height,opacity] duration-100",
            busy && "animate-pulse",
          )}
          style={{ height: `${Math.max(3, Math.round(level * 26))}px` }}
        />
      ))}
    </div>
  );
});

function voiceStatusLabel(status: VoiceInputStatus): string {
  switch (status) {
    case "recording":
      return "Recording · tap to finish";
    case "stopping":
      return "Closing the microphone";
    case "transcribing":
      return "Writing down your note";
    case "ready":
      return "Note received";
    case "failed":
      return "Microphone paused";
    default:
      return "Tap the microphone to speak";
  }
}

function friendlyVoiceError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  if (/permission|notallowed/i.test(message)) return "Microphone permission is required.";
  if (/no microphone|notfound|device/i.test(message)) return "No microphone was found.";
  if (/too short|enough/i.test(message)) return "The recording is too short.";
  if (/transcription|speech source/i.test(message))
    return "Transcription failed. Please try again.";
  return "语音输入失败，请稍后再试 / Voice input failed, please try again.";
}
