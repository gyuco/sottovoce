import {
  AudioLines,
  BellRing,
  Brain,
  HardDriveDownload,
  Languages,
  type LucideIcon,
  Plug,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { CalendarSettings } from "@/components/CalendarSettings";
import { EmailSettings } from "@/components/EmailSettings";
import { KeyForm } from "@/components/KeyForm";
import { LlmSettings } from "@/components/LlmSettings";
import { LocalModels } from "@/components/LocalModels";
import { SearchSettings } from "@/components/SearchSettings";
import { VoiceProfiles } from "@/components/VoiceProfiles";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { UI_LANGUAGES, type UiLanguage, setUiLanguage } from "@/i18n";
import {
  type AppInfo,
  CHUNKED_ENGINES,
  CONCRETE_LANGUAGES,
  type Language,
  STREAMING_ENGINES,
  STT_ENGINE_NAMES,
  type SttEngine,
  appInfo,
  sttKeyAccount,
} from "@/lib/ipc";
import { effectiveEngine, isChunkedEngine, localOnlyViolation, requiredLocalModels } from "@/lib/privacy";
import type { SettingsTab } from "@/lib/settingsTabs";
import { cn } from "@/lib/utils";
import { useAudioStore } from "@/stores/audio";
import { useModelsStore } from "@/stores/models";
import { useSettingsStore } from "@/stores/settings";

const LANGUAGES: Language[] = ["multi", ...CONCRETE_LANGUAGES];
const ENGINES: SttEngine[] = ["local", ...STREAMING_ENGINES, ...CHUNKED_ENGINES];
/** Altri partecipanti: 0 = automatico. */
const SPEAKER_COUNTS = [0, 1, 2, 3, 4, 5, 6, 7, 8];

const TABS: { id: SettingsTab; icon: LucideIcon }[] = [
  { id: "general", icon: SlidersHorizontal },
  { id: "audio", icon: AudioLines },
  { id: "ai", icon: Brain },
  { id: "models", icon: HardDriveDownload },
  { id: "integrations", icon: Plug },
];

/** Impostazioni divise in sezioni (tab); la sezione aperta la tiene l'App. */
export function SettingsPanel({ tab, onTabChange }: { tab: SettingsTab; onTabChange: (tab: SettingsTab) => void }) {
  const { t, i18n } = useTranslation();
  const { settings, keys, error, update } = useSettingsStore();
  const systemSupported = useAudioStore((s) => s.systemSupported);
  const sessionActive = useAudioStore((s) => s.status !== "idle");

  if (!settings) return null;
  const engine = effectiveEngine(settings);
  const sttAccount = sttKeyAccount(engine);
  const sttHasKey = sttAccount ? (keys[sttAccount] ?? false) : false;

  const currentUiLanguage =
    UI_LANGUAGES.find((language) => language.code === i18n.resolvedLanguage)?.code ?? "it";

  return (
    <div className="flex flex-col gap-4">
      <Tabs value={tab} onValueChange={(v) => onTabChange(v as SettingsTab)}>
        <TabsList aria-label={t("settings.tabs.label")} className="sticky top-0 z-10 shadow-xs">
          {TABS.map(({ id, icon: Icon }) => (
            <TabsTrigger key={id} value={id}>
              <Icon aria-hidden />
              {t(`settings.tabs.${id}`)}
            </TabsTrigger>
          ))}
        </TabsList>

        <TabsContent value="general">
          <Card className={cn(settings.localOnly && "border-emerald-500/50 bg-emerald-500/5")}>
            <CardHeader>
              <div className="flex items-start justify-between gap-4">
                <div>
                  <CardTitle className="flex items-center gap-2 text-base">
                    <ShieldCheck
                      className={cn("size-4", settings.localOnly ? "text-emerald-600" : "text-muted-foreground")}
                      aria-hidden
                    />
                    {t("settings.localOnly.title")}
                  </CardTitle>
                  <CardDescription className="mt-1.5">{t("settings.localOnly.hint")}</CardDescription>
                </div>
                <Switch
                  checked={settings.localOnly}
                  onCheckedChange={(localOnly) => void update({ localOnly })}
                  aria-label={t("settings.localOnly.title")}
                />
              </div>
            </CardHeader>
            {localOnlyViolation(settings) && (
              <CardContent>
                <p role="alert" className="flex gap-2 text-sm text-destructive">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
                  {t("settings.localOnly.violation")}
                </p>
              </CardContent>
            )}
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <Languages className="size-4 text-muted-foreground" aria-hidden />
                {t("settings.ui.title")}
              </CardTitle>
              <CardDescription>{t("settings.ui.languageHint")}</CardDescription>
            </CardHeader>
            <CardContent>
              <label className="flex items-center justify-between gap-4 text-sm">
                <span>{t("settings.ui.language")}</span>
                <Select
                  value={currentUiLanguage}
                  onValueChange={(value) => void setUiLanguage(value as UiLanguage)}
                >
                  <SelectTrigger className="w-48">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {UI_LANGUAGES.map((language) => (
                      <SelectItem key={language.code} value={language.code}>
                        {language.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <BellRing className="size-4 text-muted-foreground" aria-hidden />
                {t("settings.duringCall.title")}
              </CardTitle>
              <CardDescription>{t("settings.duringCall.widgetHint")}</CardDescription>
            </CardHeader>
            <CardContent>
              <label className="flex items-center justify-between gap-4 text-sm">
                <span>
                  {t("settings.duringCall.nudges")}
                  <span className="block text-xs text-muted-foreground">{t("settings.duringCall.nudgesHint")}</span>
                </span>
                <Switch checked={settings.nudges} onCheckedChange={(nudges) => void update({ nudges })} />
              </label>
              <label className="mt-4 flex items-center justify-between gap-4 text-sm">
                <span>
                  {t("settings.duringCall.liveAssist")}
                  <span className="block text-xs text-muted-foreground">{t("settings.duringCall.liveAssistHint")}</span>
                </span>
                <Switch
                  checked={settings.liveAssist}
                  onCheckedChange={(liveAssist) => void update({ liveAssist })}
                />
              </label>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="audio">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <AudioLines className="size-4 text-muted-foreground" aria-hidden />
                {t("settings.stt.title")}
              </CardTitle>
              <CardDescription>{t(`settings.stt.engines.${engine}.hint`)}</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-4">
              <label className="flex items-center justify-between gap-4 text-sm">
                <span>
                  {t("settings.stt.engine")}
                  {settings.localOnly && (
                    <span className="block text-xs text-muted-foreground">{t("settings.stt.lockedByLocalOnly")}</span>
                  )}
                </span>
                <Select
                  value={engine}
                  disabled={settings.localOnly}
                  onValueChange={(v) => void update({ sttEngine: v as SttEngine })}
                >
                  <SelectTrigger className="w-48">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ENGINES.map((e) => (
                      <SelectItem key={e} value={e}>
                        {t(`settings.stt.engines.${e}.name`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>

              {sttAccount && (
                <div className="flex flex-col gap-2">
                  <Badge variant={sttHasKey ? "default" : "secondary"} className="self-end font-normal">
                    {sttHasKey ? t("settings.key.saved") : t("settings.key.missing")}
                  </Badge>
                  <KeyForm
                    provider={sttAccount}
                    label={t("settings.stt.keyLabelFor", { name: STT_ENGINE_NAMES[engine] })}
                  />
                </div>
              )}
              {engine === "azure" && (
                <AzureRegionField
                  value={settings.azureSpeechRegion}
                  onSave={(azureSpeechRegion) => void update({ azureSpeechRegion })}
                />
              )}
              {isChunkedEngine(engine) && (
                <p className="text-xs text-muted-foreground">{t("settings.stt.chunkedHint")}</p>
              )}
              {requiredLocalModels(settings).length > 0 && <RequiredModels ids={requiredLocalModels(settings)} />}

              <LanguageSelect
                label={t("settings.language.mine")}
                hint={t("settings.language.mineHint")}
                value={settings.myLanguage}
                onChange={(myLanguage) => void update({ myLanguage })}
              />
              <LanguageSelect
                label={t("settings.language.theirs")}
                hint={t("settings.language.theirsHint")}
                value={settings.theirLanguage}
                onChange={(theirLanguage) => void update({ theirLanguage })}
              />

              <label className="flex items-center justify-between gap-4 text-sm">
                <span>
                  {t("controls.systemAudio")}
                  <span className="block text-xs text-muted-foreground">
                    {systemSupported ? t("settings.systemAudioHint") : t("channel.unsupported")}
                  </span>
                </span>
                <Switch
                  checked={settings.systemAudio && systemSupported}
                  disabled={!systemSupported}
                  onCheckedChange={(v) => void update({ systemAudio: v })}
                />
              </label>

              <label className="flex items-center justify-between gap-4 text-sm">
                <span>
                  {t("settings.echoCancellation")}
                  <span className="block text-xs text-muted-foreground">{t("settings.echoCancellationHint")}</span>
                </span>
                <Switch
                  checked={settings.echoCancellation}
                  disabled={!settings.systemAudio || !systemSupported}
                  onCheckedChange={(echoCancellation) => void update({ echoCancellation })}
                />
              </label>

              <div className="flex flex-col gap-3 border-t pt-4">
                <label className="flex items-center justify-between gap-4 text-sm">
                  <span>
                    {t("settings.speakers.title")}
                    <span className="block text-xs text-muted-foreground">{t("settings.speakers.hint")}</span>
                  </span>
                  <Switch
                    checked={settings.diarization}
                    onCheckedChange={(diarization) => void update({ diarization })}
                  />
                </label>
                {settings.diarization && (
                  <>
                    <label className="flex items-center justify-between gap-4 text-sm">
                      <span>
                        {t("settings.speakers.count")}
                        <span className="block text-xs text-muted-foreground">{t("settings.speakers.countHint")}</span>
                      </span>
                      <Select
                        value={String(settings.theirSpeakers)}
                        onValueChange={(v) => void update({ theirSpeakers: Number(v) })}
                      >
                        <SelectTrigger className="w-48">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {SPEAKER_COUNTS.map((n) => (
                            <SelectItem key={n} value={String(n)}>
                              {n === 0 ? t("settings.speakers.auto") : String(n)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </label>
                    <RequiredModels ids={["campplus-zh-en"]} />
                  </>
                )}
              </div>
            </CardContent>
          </Card>

          <VoiceProfiles />
        </TabsContent>

        <TabsContent value="ai">
          <LlmSettings />

          <SearchSettings />
        </TabsContent>

        <TabsContent value="models">
          <LocalModels />
        </TabsContent>

        <TabsContent value="integrations">
          <EmailSettings />

          <CalendarSettings provider="google" />

          <CalendarSettings provider="microsoft" />
        </TabsContent>
      </Tabs>

      {error && (
        <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">{t("settings.applyHint")}</p>
      <div className="flex items-center justify-between gap-4">
        <AboutLine />
        <Button
          variant="outline"
          size="sm"
          className="ml-auto"
          disabled={sessionActive}
          title={sessionActive ? t("settings.setup.busy") : undefined}
          onClick={() => void update({ onboardingDone: false })}
        >
          <Sparkles /> {t("settings.setup.rerun")}
        </Button>
      </div>
    </div>
  );
}

/** Versione, commit e ora del build: per verificare quale build è in uso. */
function AboutLine() {
  const { t, i18n } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    appInfo()
      .then(setInfo)
      .catch(() => setInfo(null));
  }, []);
  if (!info) return null;
  const built = info.builtAt
    ? new Date(info.builtAt).toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" })
    : "";
  return (
    <p className="text-xs text-muted-foreground tabular-nums select-text">
      {t("settings.about.line", { version: info.version, commit: info.commit || "—", built })}
      {info.debug && ` · ${t("settings.about.debug")}`}
    </p>
  );
}

/** Stato dei modelli che servono al motore locale (gestiti in "Modelli locali"). */
/** Regione della risorsa Azure Speech: salvata quando si esce dal campo. */
function AzureRegionField({ value, onSave }: { value: string; onSave: (region: string) => void }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(value);
  const valid = /^[a-z0-9]{1,40}$/i.test(draft.trim());
  return (
    <label className="flex flex-col gap-1.5 text-sm">
      <span>{t("settings.stt.azureRegion")}</span>
      <Input
        value={draft}
        placeholder="westeurope"
        aria-invalid={!valid}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          const next = draft.trim().toLowerCase();
          if (/^[a-z0-9]{1,40}$/.test(next) && next !== value) onSave(next);
        }}
      />
      <span className="text-xs text-muted-foreground">{t("settings.stt.azureRegionHint")}</span>
    </label>
  );
}

function RequiredModels({ ids }: { ids: string[] }) {
  const { t } = useTranslation();
  const models = useModelsStore((s) => s.models);
  return (
    <div className="flex flex-col gap-1.5 text-sm">
      <span>{t("settings.stt.requiredModels")}</span>
      <ul className="flex flex-col gap-1">
        {ids.map((id) => {
          const m = models.find((x) => x.id === id);
          const ready = m?.status.state === "ready";
          return (
            <li key={id} className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
              <span>{m?.name ?? id}</span>
              <Badge
                variant="secondary"
                className={cn("font-normal", ready && "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400")}
              >
                {ready ? t("models.ready") : m?.status.state === "corrupt" ? t("models.corrupt") : t("models.missing")}
              </Badge>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function LanguageSelect(props: {
  label: string;
  hint: string;
  value: Language;
  onChange: (l: Language) => void;
}) {
  const { t } = useTranslation();
  return (
    <label className="flex items-center justify-between gap-4 text-sm">
      <span>
        {props.label}
        <span className="block text-xs text-muted-foreground">{props.hint}</span>
      </span>
      <Select value={props.value} onValueChange={(v) => props.onChange(v as Language)}>
        <SelectTrigger className="w-48">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {LANGUAGES.map((l) => (
            <SelectItem key={l} value={l}>
              {t(`settings.language.${l}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </label>
  );
}
