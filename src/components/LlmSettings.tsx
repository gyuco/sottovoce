import { BrainCircuit, CheckCircle2, Loader2, RefreshCw, XCircle } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  type Language,
  type LlmProvider,
  type LlmTestResult,
  type ModelInfo,
  errorMessage,
  listLlmModels,
  llmKeyAccount,
  testLlm,
} from "@/lib/ipc";
import { useSettingsStore } from "@/stores/settings";

const PROVIDERS: LlmProvider[] = [
  "ollama",
  "opencode",
  "opencode-go",
  "openai",
  "anthropic",
  "custom",
];
/** "mine" = segue la lingua dell'utente (`targetLanguage: null`). */
const TARGETS = ["mine", "it", "en"] as const;

type Models =
  | { state: "loading" }
  | { state: "ok"; models: ModelInfo[] }
  | { state: "error"; message: string };

type Test =
  | { state: "idle" }
  | { state: "running" }
  | { state: "ok"; result: LlmTestResult }
  | { state: "error"; message: string };

/** Risultato legato all'endpoint che l'ha prodotto: cambiato l'endpoint, è scaduto. */
type Keyed<T> = { endpoint: string; value: T };

/** Modelli di un provider, ricaricati a ogni cambio di endpoint, chiave o richiesta. */
function useModels(provider: LlmProvider, reloads: number, enabled = true): Models {
  const settings = useSettingsStore((s) => s.settings);
  const keys = useSettingsStore((s) => s.keys);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const [loaded, setLoaded] = useState<Keyed<Models> | null>(null);

  const savedBaseUrl = settings?.llmBaseUrl ?? "";
  const account = llmKeyAccount(provider);
  const hasKey = account ? (keys[account] ?? false) : false;
  const noEndpoint = provider === "custom" && savedBaseUrl.trim() === "";
  const endpoint = `${provider}|${savedBaseUrl}|${hasKey}|${reloads}`;

  useEffect(() => {
    if (account && enabled) void refreshKey(account);
  }, [account, enabled, refreshKey]);

  useEffect(() => {
    if (noEndpoint || !enabled) return;
    let live = true;
    listLlmModels(provider).then(
      (models) => live && setLoaded({ endpoint, value: { state: "ok", models } }),
      (e: unknown) =>
        live && setLoaded({ endpoint, value: { state: "error", message: errorMessage(e) } }),
    );
    return () => {
      live = false;
    };
  }, [endpoint, noEndpoint, enabled, provider]);

  if (noEndpoint) return { state: "ok", models: [] };
  return loaded?.endpoint === endpoint ? loaded.value : { state: "loading" };
}

function usableModels(models: Models): ModelInfo[] | null {
  return models.state === "ok" ? models.models.filter((m) => m.usable) : null;
}

export function LlmSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const keys = useSettingsStore((s) => s.keys);
  const [tested, setTested] = useState<Keyed<Test> | null>(null);
  const [reloads, setReloads] = useState(0);
  const [baseUrl, setBaseUrl] = useState(settings?.llmBaseUrl ?? "");

  const provider = settings?.llmProvider ?? "ollama";
  const translationProvider = settings?.translationProvider ?? provider;
  const assistProvider = settings?.assistProvider ?? provider;
  const savedBaseUrl = settings?.llmBaseUrl ?? "";
  const account = llmKeyAccount(provider);
  const hasKey = account ? (keys[account] ?? false) : false;
  const models = useModels(provider, reloads);
  const translationModels = useModels(
    translationProvider,
    reloads,
    translationProvider !== provider,
  );
  const assistModels = useModels(assistProvider, reloads, assistProvider !== provider);
  const modelsFor = (p: LlmProvider) =>
    p === provider ? models : p === translationProvider ? translationModels : assistModels;
  const endpoint = `${translationProvider}|${savedBaseUrl}|${JSON.stringify(keys)}|${reloads}`;
  const test: Test = tested?.endpoint === endpoint ? tested.value : { state: "idle" };

  if (!settings) return null;

  const usable = usableModels(models) ?? [];
  const hidden = models.state === "ok" ? models.models.length - usable.length : 0;
  const usesCustom = [provider, translationProvider, assistProvider].includes("custom");

  const runTest = async () => {
    const setTest = (value: Test) => setTested({ endpoint, value });
    setTest({ state: "running" });
    try {
      setTest({ state: "ok", result: await testLlm() });
    } catch (e) {
      setTest({ state: "error", message: errorMessage(e) });
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <BrainCircuit className="size-4 text-muted-foreground" aria-hidden />
          {t("settings.llm.title")}
          {account && (
            <Badge variant={hasKey ? "default" : "secondary"} className="ml-auto font-normal">
              {hasKey ? t("settings.key.saved") : t("settings.key.missing")}
            </Badge>
          )}
        </CardTitle>
        <CardDescription>{t(`settings.llm.providers.${provider}.hint`)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <Row label={t("settings.llm.provider")}>
          <ProviderSelect
            value={provider}
            onChange={(v) => v && void update({ llmProvider: v })}
          />
        </Row>

        {usesCustom && (
          <Row label={t("settings.llm.baseUrl")} hint={t("settings.llm.baseUrlHint")}>
            <Input
              className="w-56"
              spellCheck={false}
              placeholder="https://…/v1"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              onBlur={() => {
                if (baseUrl.trim() !== savedBaseUrl) void update({ llmBaseUrl: baseUrl.trim() });
              }}
            />
          </Row>
        )}

        {account && <KeyForm provider={account} label={t("settings.llm.keyLabel")} />}

        <div className="flex items-center justify-between gap-4 text-xs text-muted-foreground">
          <span>
            {models.state === "loading" && t("settings.llm.modelsLoading")}
            {models.state === "error" && (
              <span className="text-destructive">{models.message}</span>
            )}
            {models.state === "ok" &&
              (hidden > 0
                ? t("settings.llm.modelsHidden", { count: usable.length, hidden })
                : t("settings.llm.modelsCount", { count: usable.length }))}
          </span>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={t("settings.llm.reload")}
            title={t("settings.llm.reload")}
            disabled={models.state === "loading"}
            onClick={() => setReloads((n) => n + 1)}
          >
            <RefreshCw />
          </Button>
        </div>

        <TaskField
          label={t("settings.llm.translationModel")}
          hint={t("settings.llm.translationHint")}
          mainProvider={provider}
          provider={settings.translationProvider}
          onProviderChange={(translationProvider) =>
            void update({ translationProvider, translationModel: "" })
          }
          value={settings.translationModel}
          models={modelsFor(translationProvider)}
          onChange={(translationModel) => void update({ translationModel })}
        />
        <Row label={t("settings.llm.autoTranslate")} hint={t("settings.llm.autoTranslateHint")}>
          <Switch
            checked={settings.autoTranslate}
            onCheckedChange={(autoTranslate) => void update({ autoTranslate })}
            aria-label={t("settings.llm.autoTranslate")}
          />
        </Row>
        <TaskField
          label={t("settings.llm.assistModel")}
          hint={t("settings.llm.assistHint")}
          mainProvider={provider}
          provider={settings.assistProvider}
          onProviderChange={(assistProvider) => void update({ assistProvider, assistModel: "" })}
          value={settings.assistModel}
          models={modelsFor(assistProvider)}
          onChange={(assistModel) => void update({ assistModel })}
        />

        <Row label={t("settings.llm.autoSummary")} hint={t("settings.llm.autoSummaryHint")}>
          <Switch
            checked={settings.autoSummary}
            onCheckedChange={(autoSummary) => void update({ autoSummary })}
            aria-label={t("settings.llm.autoSummary")}
          />
        </Row>

        <Row label={t("settings.llm.target")} hint={t("settings.llm.targetHint")}>
          <Select
            value={settings.targetLanguage ?? "mine"}
            onValueChange={(v) =>
              void update({ targetLanguage: v === "mine" ? null : (v as Language) })
            }
          >
            <SelectTrigger className="w-56">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TARGETS.map((l) => (
                <SelectItem key={l} value={l}>
                  {l === "mine" ? t("settings.llm.targetMine") : t(`settings.language.${l}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>

        <div className="flex items-center gap-3">
          <Button
            variant="outline"
            disabled={
              test.state === "running" ||
              (settings.translationModel === "" && settings.assistModel === "")
            }
            onClick={() => void runTest()}
          >
            {test.state === "running" && <Loader2 className="animate-spin" />}
            {t("settings.llm.test")}
          </Button>
          <TestResult test={test} />
        </div>
      </CardContent>
    </Card>
  );
}

/** "main" = segue il provider principale (`null` nelle impostazioni). */
const MAIN = "main";

function ProviderSelect(props: {
  value: LlmProvider | null;
  onChange: (p: LlmProvider | null) => void;
  /** Se presente, prima voce "come il principale" (valore `null`). */
  mainProvider?: LlmProvider;
}) {
  const { t } = useTranslation();
  return (
    <Select
      value={props.value ?? MAIN}
      onValueChange={(v) => props.onChange(v === MAIN ? null : (v as LlmProvider))}
    >
      <SelectTrigger className="w-56" aria-label={t("settings.llm.provider")}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {props.mainProvider && (
          <SelectItem value={MAIN}>
            {t("settings.llm.sameProvider", {
              name: t(`settings.llm.providers.${props.mainProvider}.name`),
            })}
          </SelectItem>
        )}
        {PROVIDERS.map((p) => (
          <SelectItem key={p} value={p}>
            {t(`settings.llm.providers.${p}.name`)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** Compito LLM: provider (o quello principale) e modello; chiave se manca. */
function TaskField(props: {
  label: string;
  hint: string;
  mainProvider: LlmProvider;
  provider: LlmProvider | null;
  onProviderChange: (p: LlmProvider | null) => void;
  value: string;
  models: Models;
  onChange: (model: string) => void;
}) {
  const { t } = useTranslation();
  const own = props.provider !== null && props.provider !== props.mainProvider;
  const account = own && props.provider ? llmKeyAccount(props.provider) : null;
  const hasKey = useSettingsStore((s) => (account ? (s.keys[account] ?? false) : true));

  return (
    <div className="flex flex-col gap-2 text-sm">
      <span>
        {props.label}
        <span className="block text-xs text-muted-foreground">{props.hint}</span>
      </span>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <ProviderSelect
          value={props.provider}
          mainProvider={props.mainProvider}
          onChange={props.onProviderChange}
        />
        <ModelField value={props.value} models={usableModels(props.models)} onChange={props.onChange} />
      </div>
      {own && props.models.state === "error" && (
        <span className="text-right text-xs text-destructive">{props.models.message}</span>
      )}
      {account && !hasKey && <KeyForm provider={account} label={t("settings.llm.keyLabel")} />}
    </div>
  );
}

function TestResult({ test }: { test: Test }) {
  const { t } = useTranslation();
  if (test.state === "ok") {
    return (
      <span className="flex items-center gap-1.5 text-sm text-muted-foreground">
        <CheckCircle2 className="size-4 text-emerald-600" aria-hidden />
        {t("settings.llm.testOk", { ms: test.result.latencyMs, model: test.result.model })}
      </span>
    );
  }
  if (test.state === "error") {
    return (
      <span role="alert" className="flex items-center gap-1.5 text-sm text-destructive">
        <XCircle className="size-4 shrink-0" aria-hidden />
        {test.message}
      </span>
    );
  }
  return null;
}

/** Select dei modelli; se l'elenco non è disponibile, campo di testo libero. */
function ModelField(props: {
  value: string;
  models: ModelInfo[] | null;
  onChange: (model: string) => void;
}) {
  const { t } = useTranslation();

  const list = props.models;
  return list && list.length > 0 ? (
    <Select value={props.value || undefined} onValueChange={props.onChange}>
      <SelectTrigger className="w-56" aria-label={t("settings.llm.chooseModel")}>
        <SelectValue placeholder={t("settings.llm.chooseModel")} />
      </SelectTrigger>
      <SelectContent>
        {props.value && !list.some((m) => m.id === props.value) && (
          <SelectItem value={props.value}>
            {props.value} ({t("settings.llm.unavailable")})
          </SelectItem>
        )}
        {list.map((m) => (
          <SelectItem key={m.id} value={m.id}>
            {m.id}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  ) : (
    <Input
      key={props.value}
      className="w-56"
      spellCheck={false}
      aria-label={t("settings.llm.chooseModel")}
      placeholder={t("settings.llm.modelPlaceholder")}
      defaultValue={props.value}
      onBlur={(e) => {
        const v = e.target.value.trim();
        if (v !== props.value) props.onChange(v);
      }}
    />
  );
}

function Row(props: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 text-sm">
      <span>
        {props.label}
        {props.hint && (
          <span className="block text-xs text-muted-foreground">{props.hint}</span>
        )}
      </span>
      {props.children}
    </div>
  );
}
