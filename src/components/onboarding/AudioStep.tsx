import { Headphones, Mic, Settings2, Volume2 } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { LevelMeter } from "@/components/LevelMeter";
import { StatusIcon } from "@/components/onboarding/parts";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  type ChannelId,
  type MicPermission,
  errorMessage,
  micPermission,
  openPrivacySettings,
  requestMicPermission,
  startAudioCheck,
  stopAudioCheck,
} from "@/lib/ipc";
import { FLOOR_DB, useAudioStore, useSelectedMicId } from "@/stores/audio";
import { useSettingsStore } from "@/stores/settings";

/** Esito della prova audio, mostrato nel riepilogo finale. */
export interface AudioResult {
  mic: boolean;
  system: boolean;
}

/** Sopra queste soglie (RMS, dBFS) il canale "ha sentito" qualcosa. */
const HEARD_DB: Record<ChannelId, number> = { me: -42, them: -55 };
/** Dopo quanto suggerire il permesso se l'audio di sistema resta muto. */
const SYSTEM_HINT_MS = 8000;

interface Level {
  rmsDb: number;
  holdDb: number;
}

const silent: Level = { rmsDb: FLOOR_DB, holdDb: FLOOR_DB };

export function AudioStep({ onResult }: { onResult: (r: AudioResult) => void }) {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const devices = useAudioStore((s) => s.devices);
  const systemSupported = useAudioStore((s) => s.systemSupported);
  const micId = useSelectedMicId();
  const [permission, setPermission] = useState<MicPermission | null>(null);
  const [levels, setLevels] = useState<Record<ChannelId, Level>>({ me: silent, them: silent });
  const [heard, setHeard] = useState<AudioResult>({ mic: false, system: false });
  const [error, setError] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState(0);
  const [now, setNow] = useState(0);
  const heardRef = useRef(heard);

  const allowed = permission === "granted" || permission === "unsupported";
  const systemOn = (settings?.systemAudio ?? false) && systemSupported;
  const savedMic = settings?.micDeviceId;

  useEffect(() => {
    void micPermission().then(setPermission, () => setPermission("unsupported"));
  }, []);

  // Riavvia la prova a ogni cambio di microfono: il backend sostituisce la cattura.
  useEffect(() => {
    if (!allowed) return;
    let live = true;
    startAudioCheck((e) => {
      if (!live) return;
      setLevels((prev) => ({
        ...prev,
        [e.channel]: { rmsDb: e.rmsDb, holdDb: Math.max(e.peakDb, prev[e.channel].holdDb - 1.5) },
      }));
      const key = e.channel === "me" ? "mic" : "system";
      if (!heardRef.current[key] && e.rmsDb > HEARD_DB[e.channel]) {
        heardRef.current = { ...heardRef.current, [key]: true };
        setHeard(heardRef.current);
        onResult(heardRef.current);
      }
    }).then(
      () => {
        if (!live) return;
        setError(null);
        setStartedAt(Date.now());
      },
      (e: unknown) => live && setError(errorMessage(e)),
    );
    return () => {
      live = false;
    };
  }, [allowed, savedMic, systemOn, onResult]);

  // Ferma la prova solo uscendo dal passo (non a ogni riavvio: sostituirebbe quella nuova).
  useEffect(() => () => void stopAudioCheck(), []);

  // Orologio per il suggerimento sul permesso dell'audio di sistema.
  useEffect(() => {
    if (heard.system || !allowed) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [heard.system, allowed]);

  const ask = async () => {
    await requestMicPermission().catch(() => false);
    setPermission(await micPermission());
  };
  const recheck = async () => setPermission(await micPermission());

  if (!settings || permission === null) return null;

  if (!allowed) {
    const undetermined = permission === "undetermined";
    return (
      <div className="flex flex-col items-start gap-3 rounded-xl border bg-card p-5 text-sm shadow-xs">
        <span className="flex size-10 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Mic className="size-5" aria-hidden />
        </span>
        <p className="font-medium">{t(undetermined ? "onboarding.audio.askTitle" : "onboarding.audio.deniedTitle")}</p>
        <p className="text-muted-foreground">{t(undetermined ? "onboarding.audio.askText" : "onboarding.audio.deniedText")}</p>
        {undetermined ? (
          <Button onClick={() => void ask()}>
            <Mic /> {t("onboarding.audio.allow")}
          </Button>
        ) : (
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => void openPrivacySettings("microphone")}>
              <Settings2 /> {t("onboarding.audio.openSettings")}
            </Button>
            <Button variant="outline" onClick={() => void recheck()}>
              {t("onboarding.audio.recheck")}
            </Button>
          </div>
        )}
      </div>
    );
  }

  const systemSilent = systemOn && !heard.system && now - startedAt > SYSTEM_HINT_MS;

  return (
    <div className="flex flex-col gap-4">
      <ChannelTest
        icon={<Mic className="size-4" aria-hidden />}
        title={t("onboarding.audio.mic")}
        hint={t("onboarding.audio.micHint")}
        ok={heard.mic}
        level={levels.me}
        meterLabel={t("onboarding.audio.mic")}
        extra={
          devices.length > 0 && (
            <Select value={micId} onValueChange={(id) => void update({ micDeviceId: id })}>
              <SelectTrigger size="sm" className="max-w-60 min-w-0" aria-label={t("controls.microphone")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {devices.map((d) => (
                  <SelectItem key={d.id} value={d.id}>
                    {d.name}
                    {d.isDefault && <span className="text-muted-foreground"> · {t("controls.defaultDevice")}</span>}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )
        }
      />

      {systemOn ? (
        <ChannelTest
          icon={<Volume2 className="size-4" aria-hidden />}
          title={t("onboarding.audio.system")}
          hint={t("onboarding.audio.systemHint")}
          ok={heard.system}
          level={levels.them}
          meterLabel={t("onboarding.audio.system")}
          extra={null}
        >
          {systemSilent && (
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span className="flex-1">{t("onboarding.audio.systemSilent")}</span>
              <Button size="sm" variant="outline" onClick={() => void openPrivacySettings("systemAudio")}>
                <Settings2 /> {t("onboarding.audio.openSettings")}
              </Button>
            </div>
          )}
        </ChannelTest>
      ) : (
        <p className="rounded-xl border bg-card p-4 text-sm text-muted-foreground shadow-xs">
          {systemSupported ? t("onboarding.audio.systemOff") : t("channel.unsupported")}
        </p>
      )}

      <p className="flex gap-2 text-xs text-muted-foreground">
        <Headphones className="mt-0.5 size-3.5 shrink-0" aria-hidden />
        {t("onboarding.audio.headphones")}
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}

function ChannelTest(props: {
  icon: ReactNode;
  title: string;
  hint: string;
  ok: boolean;
  level: Level;
  meterLabel: string;
  extra: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3 rounded-xl border bg-card p-4 shadow-xs">
      <div className="flex items-center gap-3">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          {props.icon}
        </span>
        <div className="min-w-0 flex-1 text-sm">
          <p className="font-medium">{props.title}</p>
          <p className="text-xs text-muted-foreground">{props.hint}</p>
        </div>
        {props.extra}
        <StatusIcon status={props.ok ? "ok" : "pending"} />
      </div>
      <LevelMeter rmsDb={props.level.rmsDb} holdDb={props.level.holdDb} active label={props.meterLabel} compact />
      {props.children}
    </section>
  );
}
