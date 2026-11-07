import { ArrowLeft, KeyRound, Loader2, Play, Settings, Square } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ChannelCard } from "@/components/ChannelCard";
import { MeetingContextPanel } from "@/components/MeetingContextPanel";
import { SettingsPanel } from "@/components/SettingsPanel";
import { Transcript } from "@/components/Transcript";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAudioStore } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";

export default function App() {
  const { t } = useTranslation();
  const { devices, status, error, init, setMic, start, stop } = useAudioStore();
  const { settings, hasKey, load } = useSettingsStore();
  const [view, setView] = useState<"main" | "settings">("main");

  useEffect(() => {
    void load();
    void init();
  }, [load, init]);

  const busy = status === "starting" || status === "stopping";
  const running = status === "running";
  const micId =
    devices.find((d) => d.id === settings?.micDeviceId)?.id ??
    devices.find((d) => d.isDefault)?.id ??
    devices[0]?.id;

  return (
    <main className="mx-auto flex h-screen max-w-3xl flex-col gap-4 p-5">
      <header className="flex items-center gap-3">
        {view === "settings" && (
          <Button variant="ghost" size="icon-sm" aria-label={t("settings.back")} onClick={() => setView("main")}>
            <ArrowLeft />
          </Button>
        )}
        <h1 className="text-xl font-semibold tracking-tight">
          {view === "settings" ? t("settings.title") : t("app.title")}
        </h1>
        {view === "main" && <span className="text-sm text-muted-foreground">{t("app.subtitle")}</span>}
        {view === "main" && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="ml-auto"
            aria-label={t("settings.title")}
            title={t("settings.title")}
            onClick={() => setView("settings")}
          >
            <Settings />
          </Button>
        )}
      </header>

      {view === "settings" ? (
        <SettingsPanel />
      ) : (
        <>
          <section className="flex flex-wrap items-end gap-3">
            <label className="flex min-w-60 flex-1 flex-col gap-1.5 text-sm">
              <span className="text-muted-foreground">{t("controls.microphone")}</span>
              <Select value={micId} onValueChange={(id) => void setMic(id)} disabled={busy}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {devices.map((d) => (
                    <SelectItem key={d.id} value={d.id}>
                      {d.name}
                      {d.isDefault && (
                        <span className="text-muted-foreground"> · {t("controls.defaultDevice")}</span>
                      )}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            {running || status === "stopping" ? (
              <Button variant="outline" onClick={() => void stop()} disabled={busy}>
                {busy ? <Loader2 className="animate-spin" /> : <Square />} {t("controls.stop")}
              </Button>
            ) : (
              <Button onClick={() => void start()} disabled={busy || !micId || !hasKey}>
                {busy ? <Loader2 className="animate-spin" /> : <Play />}
                {busy ? t("controls.starting") : t("controls.start")}
              </Button>
            )}
          </section>

          {settings && !hasKey && (
            <div className="flex items-center gap-3 rounded-md border bg-muted/50 px-3 py-2 text-sm">
              <KeyRound className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              <span className="flex-1">{t("settings.stt.needKey")}</span>
              <Button size="sm" variant="outline" onClick={() => setView("settings")}>
                {t("settings.stt.openSettings")}
              </Button>
            </div>
          )}

          {error && (
            <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {t(error.key, { message: error.message })}
            </p>
          )}

          <section className="grid gap-3 sm:grid-cols-2">
            <ChannelCard channel="me" />
            <ChannelCard channel="them" />
          </section>

          <MeetingContextPanel />

          <Transcript />

          <footer className="text-xs text-muted-foreground">{t("permissions.hint")}</footer>
        </>
      )}
    </main>
  );
}
