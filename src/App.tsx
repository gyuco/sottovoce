import { HardDriveDownload, KeyRound } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { AssistPanel } from "@/components/AssistPanel";
import { AudioPanel } from "@/components/AudioPanel";
import { History } from "@/components/History";
import { InsightsPanel } from "@/components/InsightsPanel";
import { MeetingChat } from "@/components/MeetingChat";
import { MeetingContextPanel } from "@/components/MeetingContextPanel";
import { NudgeCard } from "@/components/NudgeCard";
import { SessionHeader } from "@/components/SessionHeader";
import { SettingsPanel } from "@/components/SettingsPanel";
import { SummaryNotice } from "@/components/SummaryNotice";
import { Sidebar, type View } from "@/components/Sidebar";
import { TalkRatio } from "@/components/TalkRatio";
import { Transcript } from "@/components/Transcript";
import { UpdateBanner } from "@/components/UpdateBanner";
import { Button } from "@/components/ui/button";
import { onSummaryStatus } from "@/lib/ipc";
import { useSttBlocker } from "@/lib/readiness";
import { syncTray } from "@/lib/tray";
import { useWidgetSync } from "@/lib/widget";
import { useAudioStore } from "@/stores/audio";
import { useInsightsStore } from "@/stores/insights";
import { useModelsStore } from "@/stores/models";
import { useNudgeEngine, useNudgeStore } from "@/stores/nudges";
import { useSettingsStore } from "@/stores/settings";
import { useUpdateStore } from "@/stores/update";

export default function App() {
  const { t, i18n } = useTranslation();
  const { error, init, status, startedAt } = useAudioStore();
  const nudge = useNudgeStore((s) => s.active);
  const dismissNudge = useNudgeStore((s) => s.dismiss);
  const load = useSettingsStore((s) => s.load);
  const loadModels = useModelsStore((s) => s.load);
  const checkUpdate = useUpdateStore((s) => s.check);
  const blocker = useSttBlocker();
  const [view, setView] = useState<View>("live");
  /** Meeting da aprire nello Storico (dall'avviso del riepilogo). */
  const [historyFocus, setHistoryFocus] = useState<number | null>(null);

  useEffect(() => {
    void load();
    void init();
    void loadModels();
    void checkUpdate();
  }, [load, init, loadModels, checkUpdate]);

  // Riepiloghi finali generati in background dopo lo stop.
  useEffect(() => {
    const unlisten = onSummaryStatus(useInsightsStore.getState().applyStatus);
    return () => void unlisten.then((stop) => stop());
  }, []);

  // Icona nel menu bar / area di notifica, con le voci nella lingua corrente.
  useEffect(() => {
    void syncTray(t, i18n.resolvedLanguage ?? "it");
  }, [t, i18n.resolvedLanguage]);

  useNudgeEngine(status === "running", startedAt);
  useWidgetSync();

  const openSummary = (meetingId: number) => {
    // Ancora mostrato in Live (nessuna nuova sessione): basta tornarci.
    if (useInsightsStore.getState().summary?.meetingId === meetingId) {
      setView("live");
    } else {
      setHistoryFocus(meetingId);
      setView("history");
    }
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <UpdateBanner />

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <Sidebar
          view={view}
          onChange={(v) => {
            setHistoryFocus(null);
            setView(v);
          }}
        />

        {view === "settings" ? (
          <main className="min-w-0 flex-1 overflow-y-auto">
            <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-5">
              <h1 className="text-2xl font-semibold tracking-tight">{t("settings.title")}</h1>
              <SettingsPanel />
            </div>
          </main>
        ) : view === "history" ? (
          <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
            <History key={historyFocus ?? "list"} focusId={historyFocus} />
          </main>
        ) : (
          <main className="flex min-w-0 flex-1 flex-col overflow-y-auto md:overflow-hidden">
            <SessionHeader />

            {nudge && (
              <div className="px-6 pb-3">
                <NudgeCard nudge={nudge} onDismiss={dismissNudge} />
              </div>
            )}

            {blocker || error ? (
              <div className="flex flex-col gap-2 px-6 pb-3">
                {blocker && (
                  <div className="flex items-center gap-3 rounded-xl border bg-card px-3 py-2 text-sm shadow-xs">
                    {blocker === "needKey" ? (
                      <KeyRound className="size-4 shrink-0 text-primary" aria-hidden />
                    ) : (
                      <HardDriveDownload className="size-4 shrink-0 text-primary" aria-hidden />
                    )}
                    <span className="flex-1">{t(`settings.stt.${blocker}`)}</span>
                    <Button size="sm" variant="outline" onClick={() => setView("settings")}>
                      {blocker === "needKey" ? t("settings.stt.openSettings") : t("settings.stt.openModels")}
                    </Button>
                  </div>
                )}
                {error && (
                  <p
                    role="alert"
                    className="rounded-xl border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
                  >
                    {t(error.key, { message: error.message })}
                  </p>
                )}
              </div>
            ) : null}

            <div className="grid flex-1 gap-4 px-6 pb-6 md:min-h-0 md:grid-cols-[minmax(0,1fr)_minmax(0,1.15fr)]">
              <div className="flex flex-col gap-4 md:min-h-0">
                <AssistPanel />
                <InsightsPanel />
                <AudioPanel />
              </div>
              <div className="flex flex-col gap-4 md:min-h-0">
                <TalkRatio />
                <MeetingContextPanel />
                <MeetingChat />
                <Transcript />
              </div>
            </div>
          </main>
        )}

        <SummaryNotice onOpen={openSummary} />
      </div>
    </div>
  );
}
