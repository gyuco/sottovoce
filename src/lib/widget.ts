import { isTauri } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { useEffect } from "react";

import i18n from "@/i18n";
import type { AssistKind, ChannelId } from "@/lib/ipc";
import type { Nudge } from "@/lib/nudges";
import { speakerColor } from "@/lib/speakers";
import { showMainWindow } from "@/lib/tray";
import { useAssistStore } from "@/stores/assist";
import { useAudioStore } from "@/stores/audio";
import { useMeetingStore } from "@/stores/meeting";
import { useNudgeStore } from "@/stores/nudges";
import { useTranscriptStore } from "@/stores/transcript";

/**
 * Il widget overlay non ha una sessione propria: la finestra principale gli
 * invia uno snapshot dello stato con gli eventi Tauri e riceve le sue azioni.
 */
export const WIDGET_LABEL = "widget";
const STATE_EVENT = "widget:state";
const READY_EVENT = "widget:ready";
const ACTION_EVENT = "widget:action";

/** Frasi di "Loro" mostrate nel widget. */
const WIDGET_LINES = 3;
/** Suggerimenti mostrati nel widget (non ancora usati). */
const WIDGET_ASSISTS = 3;
/** Invio al massimo ogni… (le parziali non contano, ma i livelli audio sì). */
const THROTTLE_MS = 200;

export interface WidgetLine {
  id: number;
  channel: ChannelId;
  speaker: string;
  color: string;
  text: string;
  translation: string | null;
}

export interface WidgetAssist {
  id: number;
  kind: AssistKind;
  text: string;
}

export interface WidgetSnapshot {
  language: string;
  status: "idle" | "starting" | "running" | "stopping";
  /** Epoch ms dell'avvio, per il cronometro. */
  startedWall: number;
  meetingName: string;
  lines: WidgetLine[];
  assists: WidgetAssist[];
  nudge: Nudge | null;
}

export type WidgetAction =
  | { type: "stop" }
  | { type: "openMain" }
  | { type: "dismissNudge" }
  | { type: "assistUsed"; id: number };

function snapshot(): WidgetSnapshot {
  const audio = useAudioStore.getState();
  const transcript = useTranscriptStore.getState();
  const names = useMeetingStore.getState().context.speakers;
  const assist = useAssistStore.getState();

  const lines = transcript.lines
    .filter((l) => l.channel === "them")
    .slice(-WIDGET_LINES)
    .map((l) => ({
      id: l.id,
      channel: l.channel,
      speaker:
        l.speaker === null
          ? i18n.t("channel.them")
          : (names[l.speaker] ?? i18n.t("speakers.label", { n: l.speaker + 1 })),
      color: speakerColor(l.channel, l.speaker) ?? "",
      text: l.text,
      translation: transcript.translations[l.id]?.text ?? null,
    }));

  return {
    language: i18n.resolvedLanguage ?? "it",
    status: audio.status,
    startedWall: audio.startedWall,
    meetingName: useMeetingStore.getState().context.name.trim(),
    lines,
    assists: assist.items
      .filter((a) => !assist.used.includes(a.id))
      .slice(0, WIDGET_ASSISTS)
      .map(({ id, kind, text }) => ({ id, kind, text })),
    nudge: useNudgeStore.getState().active,
  };
}

function handleAction(action: WidgetAction) {
  switch (action.type) {
    case "stop":
      if (useAudioStore.getState().status === "running") void useAudioStore.getState().stop();
      break;
    case "openMain":
      void showMainWindow();
      break;
    case "dismissNudge":
      useNudgeStore.getState().dismiss();
      break;
    case "assistUsed":
      useAssistStore.getState().toggleUsed(action.id);
      break;
  }
}

/**
 * Finestra principale: tiene aggiornato il widget a ogni cambio di stato
 * rilevante e ne esegue le azioni. Fuori da Tauri non fa nulla.
 */
export function useWidgetSync() {
  useEffect(() => {
    if (!isTauri()) return;

    let timer: ReturnType<typeof setTimeout> | undefined;
    let last = "";
    const send = () => {
      timer = undefined;
      const payload = snapshot();
      const json = JSON.stringify(payload);
      if (json === last) return;
      last = json;
      void emitTo(WIDGET_LABEL, STATE_EVENT, payload).catch(() => {});
    };
    const schedule = () => {
      timer ??= setTimeout(send, THROTTLE_MS);
    };

    const stores = [useAudioStore, useTranscriptStore, useAssistStore, useNudgeStore, useMeetingStore] as const;
    const unsubscribers = stores.map((store) => store.subscribe(schedule));
    i18n.on("languageChanged", schedule);

    const ready = listen(READY_EVENT, () => {
      last = "";
      schedule();
    });
    const actions = listen<WidgetAction>(ACTION_EVENT, (e) => handleAction(e.payload));

    return () => {
      clearTimeout(timer);
      unsubscribers.forEach((stop) => stop());
      i18n.off("languageChanged", schedule);
      void ready.then((stop) => stop());
      void actions.then((stop) => stop());
    };
  }, []);
}

/** Widget: riceve gli snapshot e chiede subito lo stato corrente. */
export function listenWidgetState(onState: (s: WidgetSnapshot) => void) {
  const unlisten = listen<WidgetSnapshot>(STATE_EVENT, (e) => onState(e.payload));
  void unlisten.then(() => emitTo("main", READY_EVENT));
  return () => void unlisten.then((stop) => stop());
}

export function sendWidgetAction(action: WidgetAction) {
  void emitTo("main", ACTION_EVENT, action);
}
