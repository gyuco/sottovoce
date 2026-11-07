import { BrainCircuit, CheckCircle2, Loader2, RefreshCw, XCircle } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
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

export function LlmSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const keys = useSettingsStore((s) => s.keys);
  const [loaded, setLoaded] = useState<Keyed<Models> | null>(null);
  const [tested, setTested] = useState<Keyed<Test> | null>(null);
  const [reloads, setReloads] = useState(0);
  const [baseUrl, setBaseUrl] = useState(settings?.llmBaseUrl ?? "");

  const provider = settings?.llmProvider ?? "ollama";
  const savedBaseUrl = settings?.llmBaseUrl ?? "";
  const account = llmKeyAccount(provider);
  const hasKey = account ? (keys[account] ?? false) : false;
  const noEndpoint = provider === "custom" && savedBaseUrl.trim() === "";
  const endpoint = `${provider}|${savedBaseUrl}|${hasKey}|${reloads}`;

  // Ricarica a ogni cambio di endpoint o di chiave (e su richiesta).
  useEffect(() => {
    if (noEndpoint) return;
    let live = true;
    listLlmModels().then(
      (models) => live && setLoaded({ endpoint, value: { state: "ok", models } }),
      (e: unknown) =>
        live && setLoaded({ endpoint, value: { state: "error", message: errorMessage(e) } }),
    );
    return () => {
      live = false;
    };
  }, [endpoint, noEndpoint]);

  const models: Models = noEndpoint
    ? { state: "ok", models: [] }
    : loaded?.endpoint === endpoint
      ? loaded.value
      : { state: "loading" };
  const test: Test = tested?.endpoint === endpoint ? tested.value : { state: "idle" };

  useEffect(() => {
    if (account) void refreshKey(account);
  }, [account, refreshKey]);

  if (!settings) return null;

  const usable = models.state === "ok" ? models.models.filter((m) => m.usable) : [];
  const hidden = models.state === "ok" ? models.models.length - usable.length : 0;

  const runTest = async () => {
    const setTest = (value: Test) => setTested({ endpoint, value });
    setTest({ state: "running" });
    try {
      setTest({ state: "ok", result: await testLlm(settings.translationModel) });
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
          <Select
            value={provider}
            onValueChange={(v) => void update({ llmProvider: v as LlmProvider })}
          >
            <SelectTrigger className="w-56">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PROVIDERS.map((p) => (
                <SelectItem key={p} value={p}>
                  {t(`settings.llm.providers.${p}.name`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>

        {provider === "custom" && (
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

        <ModelField
          label={t("settings.llm.translationModel")}
          hint={t("settings.llm.translationHint")}
          value={settings.translationModel}
          models={models.state === "ok" ? usable : null}
          onChange={(translationModel) => void update({ translationModel })}
        />
        <ModelField
          label={t("settings.llm.assistModel")}
          hint={t("settings.llm.assistHint")}
          value={settings.assistModel}
          models={models.state === "ok" ? usable : null}
          onChange={(assistModel) => void update({ assistModel })}
        />

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
            disabled={test.state === "running" || settings.translationModel === ""}
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
  label: string;
  hint: string;
  value: string;
  models: ModelInfo[] | null;
  onChange: (model: string) => void;
}) {
  const { t } = useTranslation();

  const list = props.models;
  const field =
    list && list.length > 0 ? (
      <Select value={props.value || undefined} onValueChange={props.onChange}>
        <SelectTrigger className="w-56">
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
        placeholder={t("settings.llm.modelPlaceholder")}
        defaultValue={props.value}
        onBlur={(e) => {
          const v = e.target.value.trim();
          if (v !== props.value) props.onChange(v);
        }}
      />
    );

  return (
    <Row label={props.label} hint={props.hint}>
      {field}
    </Row>
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
