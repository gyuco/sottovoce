import { CheckCircle2, Cloud, Download, Laptop, Loader2, RefreshCw, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
import { Choice, StatusIcon } from "@/components/onboarding/parts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  type LlmProvider,
  type ModelInfo,
  type Settings,
  errorMessage,
  listLlmModels,
  llmKeyAccount,
  testLlm,
} from "@/lib/ipc";
import { effectiveEngine } from "@/lib/privacy";
import { cn, formatBytes } from "@/lib/utils";
import { useModelsStore } from "@/stores/models";
import { useSettingsStore } from "@/stores/settings";

const VAD = "silero-vad";
const SPEAKERS = "campplus-zh-en";
const EMBEDDING = "embeddinggemma-2-q4";

// --- Motore di trascrizione --------------------------------------------------

export function EngineStep() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const hasKey = useSettingsStore((s) => s.hasKey);
  if (!settings) return null;
  const engine = effectiveEngine(settings);

  return (
    <div className="flex flex-col gap-4">
      <div role="radiogroup" aria-label={t("onboarding.engine.title")} className="grid gap-3 sm:grid-cols-2">
        <Choice
          selected={engine === "local"}
          onSelect={() => void update({ sttEngine: "local" })}
          className="flex flex-col gap-2 p-4"
        >
          <span className="flex items-center gap-2 font-medium">
            <Laptop className="size-4 text-primary" aria-hidden />
            {t("onboarding.engine.local.title")}
            <Badge variant="secondary" className="ml-auto font-normal">
              {t("onboarding.recommended")}
            </Badge>
          </span>
          <span className="text-muted-foreground">{t("onboarding.engine.local.text")}</span>
        </Choice>
        <Choice
          selected={engine === "deepgram"}
          onSelect={() => void update({ sttEngine: "deepgram", localOnly: false })}
          className="flex flex-col gap-2 p-4"
        >
          <span className="flex items-center gap-2 font-medium">
            <Cloud className="size-4 text-primary" aria-hidden />
            {t("onboarding.engine.cloud.title")}
          </span>
          <span className="text-muted-foreground">{t("onboarding.engine.cloud.text")}</span>
        </Choice>
      </div>

      {engine === "local" ? (
        <label className="flex items-center justify-between gap-4 rounded-xl border bg-card p-4 text-sm shadow-xs">
          <span className="flex gap-3">
            <ShieldCheck className="mt-0.5 size-4 shrink-0 text-emerald-600" aria-hidden />
            <span>
              {t("onboarding.engine.localOnly")}
              <span className="block text-xs text-muted-foreground">{t("onboarding.engine.localOnlyHint")}</span>
            </span>
          </span>
          <Switch checked={settings.localOnly} onCheckedChange={(localOnly) => void update({ localOnly })} />
        </label>
      ) : (
        <div className="flex flex-col gap-2 rounded-xl border bg-card p-4 text-sm shadow-xs">
          <span className="flex items-center gap-2 font-medium">
            {t("settings.stt.keyLabel")}
            {hasKey && (
              <Badge variant="secondary" className="ml-auto bg-emerald-500/12 font-normal text-emerald-700 dark:text-emerald-400">
                {t("settings.key.saved")}
              </Badge>
            )}
          </span>
          <p className="text-xs text-muted-foreground">{t("onboarding.engine.cloud.keyHint")}</p>
          <KeyForm provider="deepgram" label={t("settings.stt.keyLabel")} />
        </div>
      )}
    </div>
  );
}

// --- Modelli locali ----------------------------------------------------------

interface ModelChoice {
  id: string;
  /** Chiave della descrizione in `onboarding.models.purpose`. */
  purpose: string;
  required: boolean;
  /** Proposto acceso finché l'utente non sceglie. */
  defaultOn: boolean;
  /** Setting legato al modello: spegnere il modello lo disattiva. */
  setting?: "diarization" | "semanticSearch";
}

function choicesFor(s: Settings): ModelChoice[] {
  const local = effectiveEngine(s) === "local";
  return [
    { id: VAD, purpose: "vad", required: local, defaultOn: local },
    { id: s.localSttModel, purpose: local ? "stt" : "sttBackup", required: local, defaultOn: local },
    // Con Deepgram i parlanti li separa già il cloud: CAM++ serve solo ai
    // profili vocali, quindi spegnerlo non tocca la diarization.
    {
      id: SPEAKERS,
      purpose: "speakers",
      required: false,
      defaultOn: s.diarization,
      setting: local ? "diarization" : undefined,
    },
    {
      id: EMBEDDING,
      purpose: "search",
      required: false,
      defaultOn: s.semanticSearch,
      setting: "semanticSearch",
    },
  ];
}

export function ModelsStep() {
  const { t, i18n } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const models = useModelsStore((s) => s.models);
  const progress = useModelsStore((s) => s.progress);
  const errors = useModelsStore((s) => s.errors);
  const download = useModelsStore((s) => s.download);
  const load = useModelsStore((s) => s.load);
  /** Scelte facoltative dell'utente (id → scaricare). */
  const [picked, setPicked] = useState<Record<string, boolean>>({});

  useEffect(() => {
    void load();
  }, [load]);

  if (!settings) return null;
  const choices = choicesFor(settings);
  const isOn = (c: ModelChoice) => c.required || (picked[c.id] ?? c.defaultOn);
  const toggle = (c: ModelChoice, on: boolean) => {
    setPicked((p) => ({ ...p, [c.id]: on }));
    if (c.setting) void update({ [c.setting]: on });
  };

  const rows = choices.flatMap((c) => {
    const model = models.find((m) => m.id === c.id);
    return model ? [{ choice: c, model }] : [];
  });
  const todo = rows.filter(
    ({ choice, model }) => isOn(choice) && model.status.state !== "ready" && !model.downloading && !progress[model.id],
  );
  const busy = rows.some(({ model }) => model.downloading || progress[model.id]);
  const size = todo.reduce((sum, { model }) => sum + model.size, 0);
  const fmt = (n: number) => formatBytes(n, i18n.language);

  const downloadAll = async () => {
    // Uno alla volta: banda piena al modello che serve prima.
    for (const { model } of todo) await download(model.id);
  };

  return (
    <div className="flex flex-col gap-4">
      <ul className="flex flex-col divide-y rounded-xl border bg-card shadow-xs">
        {rows.map(({ choice, model }) => {
          const p = progress[model.id];
          const ready = model.status.state === "ready";
          const pct = p && model.size > 0 ? Math.min(100, Math.round((p.downloaded / model.size) * 100)) : 0;
          return (
            <li key={model.id} className="flex flex-col gap-2 px-4 py-3">
              <div className="flex items-center gap-3 text-sm">
                <div className="min-w-0 flex-1">
                  <p className="font-medium">{t(`onboarding.models.purpose.${choice.purpose}.title`)}</p>
                  <p className="text-xs text-muted-foreground">
                    {t(`onboarding.models.purpose.${choice.purpose}.text`)} · {fmt(model.size)}
                  </p>
                </div>
                {ready ? (
                  <StatusIcon status="ok" />
                ) : p || model.downloading ? (
                  <span className="text-xs text-muted-foreground tabular-nums">{pct}%</span>
                ) : choice.required ? (
                  <Badge variant="secondary" className="font-normal">
                    {t("onboarding.models.required")}
                  </Badge>
                ) : (
                  <Switch
                    checked={isOn(choice)}
                    onCheckedChange={(on) => toggle(choice, on)}
                    aria-label={t(`onboarding.models.purpose.${choice.purpose}.title`)}
                  />
                )}
              </div>
              {(p || model.downloading) && (
                <div
                  role="progressbar"
                  aria-label={model.name}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={pct}
                  className="h-1.5 overflow-hidden rounded-full bg-muted"
                >
                  <div className="h-full bg-primary transition-[width] duration-200" style={{ width: `${pct}%` }} />
                </div>
              )}
              {errors[model.id] && (
                <p role="alert" className="text-xs text-destructive">
                  {errors[model.id]}
                </p>
              )}
            </li>
          );
        })}
      </ul>

      <div className="flex flex-wrap items-center gap-3">
        <Button onClick={() => void downloadAll()} disabled={todo.length === 0}>
          <Download />
          {todo.length > 0 ? t("onboarding.models.download", { size: fmt(size) }) : t("onboarding.models.allReady")}
        </Button>
        {busy && (
          <span className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" aria-hidden />
            {t("onboarding.models.background")}
          </span>
        )}
      </div>
      {errors._list && <p className="text-sm text-destructive">{errors._list}</p>}
    </div>
  );
}

// --- Modello linguistico -----------------------------------------------------

type AiMode = "ollama" | "cloud" | "later";
const CLOUD_PROVIDERS: LlmProvider[] = ["openai", "anthropic", "opencode", "opencode-go"];

type Models =
  | { state: "loading" }
  | { state: "ok"; models: ModelInfo[] }
  | { state: "error"; message: string };

type Test =
  | { state: "idle" }
  | { state: "running" }
  | { state: "ok"; ms: number }
  | { state: "error"; message: string };

export function AiStep() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const keys = useSettingsStore((s) => s.keys);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const configured = settings ? settings.translationModel.trim() !== "" : false;
  const [mode, setMode] = useState<AiMode>(() =>
    !settings || !configured
      ? settings?.localOnly
        ? "ollama"
        : "later"
      : settings.llmProvider === "ollama"
        ? "ollama"
        : "cloud",
  );
  const [cloud, setCloud] = useState<LlmProvider>(
    settings && CLOUD_PROVIDERS.includes(settings.llmProvider) ? settings.llmProvider : "openai",
  );
  const [reloads, setReloads] = useState(0);
  const [list, setList] = useState<{ key: string; value: Models } | null>(null);
  const [test, setTest] = useState<Test>({ state: "idle" });

  const provider: LlmProvider | null = mode === "ollama" ? "ollama" : mode === "cloud" ? cloud : null;
  const account = provider ? llmKeyAccount(provider) : null;
  const hasKey = account ? (keys[account] ?? false) : true;
  const listKey = `${provider}|${hasKey}|${reloads}`;
  const models: Models = list?.key === listKey ? list.value : { state: "loading" };

  useEffect(() => {
    if (account) void refreshKey(account);
  }, [account, refreshKey]);

  useEffect(() => {
    if (!provider || !hasKey) return;
    let live = true;
    listLlmModels(provider).then(
      (m) => live && setList({ key: listKey, value: { state: "ok", models: m.filter((x) => x.usable) } }),
      (e: unknown) => live && setList({ key: listKey, value: { state: "error", message: errorMessage(e) } }),
    );
    return () => {
      live = false;
    };
  }, [provider, hasKey, listKey]);

  if (!settings) return null;
  const current = settings.llmProvider === provider ? settings.translationModel : "";

  const choose = (model: string) => {
    if (!provider) return;
    setTest({ state: "idle" });
    void update({
      llmProvider: provider,
      translationModel: model,
      translationProvider: null,
      assistModel: model,
      assistProvider: null,
    });
  };

  const runTest = async () => {
    setTest({ state: "running" });
    try {
      const r = await testLlm();
      setTest({ state: "ok", ms: r.latencyMs });
    } catch (e) {
      setTest({ state: "error", message: errorMessage(e) });
    }
  };

  const modes: { mode: AiMode; disabled?: boolean }[] = [
    { mode: "ollama" },
    { mode: "cloud", disabled: settings.localOnly },
    { mode: "later" },
  ];

  return (
    <div className="flex flex-col gap-4">
      <div role="radiogroup" aria-label={t("onboarding.ai.title")} className="grid gap-3 sm:grid-cols-3">
        {modes.map(({ mode: m, disabled }) => (
          <Choice
            key={m}
            selected={mode === m}
            onSelect={() => !disabled && setMode(m)}
            className={cn("flex flex-col gap-1 p-4", disabled && "pointer-events-none opacity-50")}
          >
            <span className="font-medium">{t(`onboarding.ai.modes.${m}.title`)}</span>
            <span className="text-xs text-muted-foreground">{t(`onboarding.ai.modes.${m}.text`)}</span>
          </Choice>
        ))}
      </div>
      {settings.localOnly && <p className="text-xs text-muted-foreground">{t("onboarding.ai.localOnly")}</p>}

      {provider && (
        <div className="flex flex-col gap-4 rounded-xl border bg-card p-4 text-sm shadow-xs">
          {mode === "cloud" && (
            <>
              <label className="flex items-center justify-between gap-4">
                <span>{t("settings.llm.provider")}</span>
                <Select value={cloud} onValueChange={(v) => setCloud(v as LlmProvider)}>
                  <SelectTrigger className="w-56">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CLOUD_PROVIDERS.map((p) => (
                      <SelectItem key={p} value={p}>
                        {t(`settings.llm.providers.${p}.name`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <p className="text-xs text-muted-foreground">{t(`settings.llm.providers.${cloud}.hint`)}</p>
              {account && <KeyForm provider={account} label={t("settings.llm.keyLabel")} />}
            </>
          )}

          {!hasKey ? null : models.state === "loading" ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden /> {t("settings.llm.modelsLoading")}
            </p>
          ) : models.state === "error" ? (
            <div className="flex flex-col gap-2">
              <p className="text-destructive">
                {mode === "ollama" ? t("onboarding.ai.ollamaMissing") : models.message}
              </p>
              <Button variant="outline" size="sm" className="self-start" onClick={() => setReloads((n) => n + 1)}>
                <RefreshCw /> {t("onboarding.ai.retry")}
              </Button>
            </div>
          ) : models.models.length === 0 ? (
            <div className="flex flex-col gap-2">
              <p className="text-muted-foreground">
                {mode === "ollama" ? t("onboarding.ai.ollamaNoModels") : t("onboarding.ai.noModels")}
              </p>
              <Button variant="outline" size="sm" className="self-start" onClick={() => setReloads((n) => n + 1)}>
                <RefreshCw /> {t("onboarding.ai.retry")}
              </Button>
            </div>
          ) : (
            <>
              <label className="flex items-center justify-between gap-4">
                <span>
                  {t("onboarding.ai.model")}
                  <span className="block text-xs text-muted-foreground">{t("onboarding.ai.modelHint")}</span>
                </span>
                <Select value={current || undefined} onValueChange={choose}>
                  <SelectTrigger className="w-56">
                    <SelectValue placeholder={t("settings.llm.chooseModel")} />
                  </SelectTrigger>
                  <SelectContent>
                    {models.models.map((m) => (
                      <SelectItem key={m.id} value={m.id}>
                        {m.name ?? m.id}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              {current && (
                <>
                  <label className="flex items-center justify-between gap-4">
                    <span>
                      {t("settings.llm.autoTranslate")}
                      <span className="block text-xs text-muted-foreground">
                        {t("settings.llm.autoTranslateHint")}
                      </span>
                    </span>
                    <Switch
                      checked={settings.autoTranslate}
                      onCheckedChange={(autoTranslate) => void update({ autoTranslate })}
                    />
                  </label>
                  <div className="flex items-center gap-3">
                    <Button variant="outline" size="sm" onClick={() => void runTest()} disabled={test.state === "running"}>
                      {test.state === "running" && <Loader2 className="animate-spin" />}
                      {t("settings.llm.test")}
                    </Button>
                    {test.state === "ok" && (
                      <span className="flex items-center gap-1.5 text-emerald-600 dark:text-emerald-400">
                        <CheckCircle2 className="size-4" aria-hidden />
                        {t("onboarding.ai.testOk", { ms: test.ms })}
                      </span>
                    )}
                    {test.state === "error" && (
                      <span className="flex items-center gap-1.5 text-destructive">
                        <XCircle className="size-4 shrink-0" aria-hidden />
                        {test.message}
                      </span>
                    )}
                  </div>
                </>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
