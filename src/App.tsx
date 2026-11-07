import { KeyRound } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { AssistPanel } from "@/components/AssistPanel";
import { AudioPanel } from "@/components/AudioPanel";
import { MeetingContextPanel } from "@/components/MeetingContextPanel";
import { SessionHeader } from "@/components/SessionHeader";
import { SettingsPanel } from "@/components/SettingsPanel";
import { Sidebar, type View } from "@/components/Sidebar";
import { TalkRatio } from "@/components/TalkRatio";
import { Transcript } from "@/components/Transcript";
import { Button } from "@/components/ui/button";
import { useAudioStore } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";

export default function App() {
  const { t } = useTranslation();
  const { error, init } = useAudioStore();
  const { settings, hasKey, load } = useSettingsStore();
  const [view, setView] = useState<View>("live");

  useEffect(() => {
    void load();
    void init();
  }, [load, init]);

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar view={view} onChange={setView} />

      {view === "settings" ? (
        <main className="min-w-0 flex-1 overflow-y-auto">
          <div className="mx-auto flex max-w-3xl flex-col gap-4 px-6 py-5">
            <h1 className="text-2xl font-semibold tracking-tight">{t("settings.title")}</h1>
            <SettingsPanel />
          </div>
        </main>
      ) : (
        <main className="flex min-w-0 flex-1 flex-col overflow-y-auto md:overflow-hidden">
          <SessionHeader />

          {(settings && !hasKey) || error ? (
            <div className="flex flex-col gap-2 px-6 pb-3">
              {settings && !hasKey && (
                <div className="flex items-center gap-3 rounded-xl border bg-card px-3 py-2 text-sm shadow-xs">
                  <KeyRound className="size-4 shrink-0 text-primary" aria-hidden />
                  <span className="flex-1">{t("settings.stt.needKey")}</span>
                  <Button size="sm" variant="outline" onClick={() => setView("settings")}>
                    {t("settings.stt.openSettings")}
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
              <AudioPanel />
            </div>
            <div className="flex flex-col gap-4 md:min-h-0">
              <TalkRatio />
              <MeetingContextPanel />
              <Transcript />
            </div>
          </div>
        </main>
      )}
    </div>
  );
}
