import {
  ArrowLeft,
  ArrowRight,
  AudioLines,
  Check,
  Languages,
  Lightbulb,
  ListChecks,
  Loader2,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Logo } from "@/components/Logo";
import { AudioStep, type AudioResult } from "@/components/onboarding/AudioStep";
import { Choice, type Status, StatusIcon } from "@/components/onboarding/parts";
import { AiStep, EngineStep, ModelsStep } from "@/components/onboarding/SetupSteps";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { UI_LANGUAGES, type UiLanguage, setUiLanguage } from "@/i18n";
import { CONCRETE_LANGUAGES, type Language } from "@/lib/ipc";
import { effectiveEngine, llmUsed } from "@/lib/privacy";
import { useSttBlocker } from "@/lib/readiness";
import { cn } from "@/lib/utils";
import { useModelsStore } from "@/stores/models";
import { useSettingsStore } from "@/stores/settings";

/** Modello della ricerca per significato (catalogo `sv-models`). */
const EMBEDDING_MODEL = "embeddinggemma-2-q4";

const STEPS = ["welcome", "languages", "engine", "models", "audio", "ai", "done"] as const;
type Step = (typeof STEPS)[number];

/**
 * Setup guidato del primo avvio: benvenuto, lingue, motore di trascrizione,
 * modelli locali, prova audio con i permessi, modello linguistico. Ogni
 * scelta si salva subito nei settings; alla fine (o saltando) `onboardingDone`.
 */
export function Onboarding() {
  const { t } = useTranslation();
  const update = useSettingsStore((s) => s.update);
  const [step, setStep] = useState<Step>("welcome");
  const [audio, setAudio] = useState<AudioResult>({ mic: false, system: false });
  const [finishing, setFinishing] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const index = STEPS.indexOf(step);

  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 });
  }, [step]);

  const go = (delta: number) => setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, index + delta))] ?? step);
  const finish = async () => {
    setFinishing(true);
    await update({ onboardingDone: true });
    setFinishing(false);
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background">
      {step !== "welcome" && (
        <header className="flex items-center gap-4 border-b px-6 py-3">
          <Logo className="size-7" label={t("app.title")} />
          <ol className="flex flex-1 items-center gap-1.5" aria-label={t("onboarding.progress")}>
            {STEPS.slice(1).map((s, i) => {
              const n = i + 1;
              return (
                <li
                  key={s}
                  aria-current={s === step ? "step" : undefined}
                  className={cn(
                    "h-1.5 flex-1 rounded-full bg-muted transition-colors",
                    n <= index && "bg-primary",
                  )}
                  title={t(`onboarding.steps.${s}`)}
                />
              );
            })}
          </ol>
          <span className="text-xs text-muted-foreground tabular-nums">
            {t("onboarding.stepOf", { n: index, total: STEPS.length - 1 })}
          </span>
          {step !== "done" && (
            <Button variant="ghost" size="sm" onClick={() => void finish()} disabled={finishing}>
              {t("onboarding.skip")}
            </Button>
          )}
        </header>
      )}

      <div ref={scroller} className="flex-1 overflow-y-auto">
        <div className="mx-auto flex min-h-full max-w-2xl flex-col justify-center gap-6 px-6 py-8">
          {step === "welcome" ? (
            <Welcome onStart={() => go(1)} onSkip={() => void finish()} />
          ) : (
            <>
              <div className="flex flex-col gap-1.5">
                <h1 className="text-2xl font-semibold tracking-tight">{t(`onboarding.${step}.title`)}</h1>
                <p className="text-sm text-muted-foreground">{t(`onboarding.${step}.lead`)}</p>
              </div>
              {step === "languages" && <LanguagesStep />}
              {step === "engine" && <EngineStep />}
              {step === "models" && <ModelsStep />}
              {step === "audio" && <AudioStep onResult={setAudio} />}
              {step === "ai" && <AiStep />}
              {step === "done" && <DoneStep audio={audio} />}
            </>
          )}
        </div>
      </div>

      {step !== "welcome" && (
        <footer className="flex items-center justify-between gap-3 border-t px-6 py-3">
          <Button variant="ghost" onClick={() => go(-1)}>
            <ArrowLeft /> {t("onboarding.back")}
          </Button>
          {step === "done" ? (
            <Button size="lg" onClick={() => void finish()} disabled={finishing}>
              {finishing ? <Loader2 className="animate-spin" /> : <Check />}
              {t("onboarding.finish")}
            </Button>
          ) : (
            <Button onClick={() => go(1)}>
              {t("onboarding.next")} <ArrowRight />
            </Button>
          )}
        </footer>
      )}
    </div>
  );
}

const FEATURES: { icon: LucideIcon; key: string }[] = [
  { icon: AudioLines, key: "transcribe" },
  { icon: Languages, key: "translate" },
  { icon: Lightbulb, key: "suggest" },
  { icon: ListChecks, key: "summarize" },
];

function Welcome({ onStart, onSkip }: { onStart: () => void; onSkip: () => void }) {
  const { t, i18n } = useTranslation();
  const current = UI_LANGUAGES.find((l) => l.code === i18n.resolvedLanguage)?.code ?? "it";

  return (
    <div className="flex flex-col items-center gap-8 text-center">
      <Logo className="size-24 drop-shadow-lg" label={t("app.title")} />
      <div className="flex flex-col gap-2">
        <h1 className="text-3xl font-semibold tracking-tight">{t("onboarding.welcome.title")}</h1>
        <p className="mx-auto max-w-md text-muted-foreground">{t("onboarding.welcome.lead")}</p>
      </div>
      <ul className="grid w-full grid-cols-2 gap-3 text-left">
        {FEATURES.map(({ icon: Icon, key }) => (
          <li key={key} className="flex gap-3 rounded-xl border bg-card p-3 shadow-xs">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Icon className="size-4.5" aria-hidden />
            </span>
            <span className="flex flex-col">
              <span className="text-sm font-medium">{t(`onboarding.welcome.features.${key}.title`)}</span>
              <span className="text-xs text-muted-foreground">{t(`onboarding.welcome.features.${key}.text`)}</span>
            </span>
          </li>
        ))}
      </ul>
      <p className="text-sm text-muted-foreground">{t("onboarding.welcome.time")}</p>
      <div className="flex flex-col items-center gap-3">
        <Button size="lg" className="min-w-48" onClick={onStart}>
          {t("onboarding.welcome.start")} <ArrowRight />
        </Button>
        <div className="flex items-center gap-3 text-sm">
          <Select value={current} onValueChange={(v) => void setUiLanguage(v as UiLanguage)}>
            <SelectTrigger size="sm" className="w-36" aria-label={t("settings.ui.language")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {UI_LANGUAGES.map((l) => (
                <SelectItem key={l.code} value={l.code}>
                  {l.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <button
            type="button"
            className="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
            onClick={onSkip}
          >
            {t("onboarding.welcome.skip")}
          </button>
        </div>
      </div>
    </div>
  );
}

function LanguagesStep() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  if (!settings) return null;

  const group = (
    label: string,
    hint: string,
    value: Language,
    options: readonly Language[],
    onChange: (l: Language) => void,
  ) => (
    <section className="flex flex-col gap-2">
      <div>
        <h2 className="text-sm font-medium">{label}</h2>
        <p className="text-xs text-muted-foreground">{hint}</p>
      </div>
      <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-2">
        {options.map((l) => (
          <Choice key={l} selected={value === l} onSelect={() => onChange(l)}>
            {t(`settings.language.${l}`)}
          </Choice>
        ))}
      </div>
    </section>
  );

  return (
    <div className="flex flex-col gap-6">
      {group(
        t("onboarding.languages.mine"),
        t("onboarding.languages.mineHint"),
        settings.myLanguage,
        CONCRETE_LANGUAGES,
        (myLanguage) => void update({ myLanguage }),
      )}
      {group(
        t("onboarding.languages.theirs"),
        t("onboarding.languages.theirsHint"),
        settings.theirLanguage,
        ["multi", ...CONCRETE_LANGUAGES],
        (theirLanguage) => void update({ theirLanguage }),
      )}
    </div>
  );
}

function DoneStep({ audio }: { audio: AudioResult }) {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const models = useModelsStore((s) => s.models);
  const blocker = useSttBlocker();
  if (!settings) return null;

  const downloading = models.some((m) => m.downloading);
  const engine = effectiveEngine(settings);
  const stt: [Status, string] =
    blocker === null
      ? ["ok", t(`onboarding.done.stt.${engine}`)]
      : blocker === "needKey"
        ? ["warn", t("onboarding.done.stt.needKey")]
        : downloading
          ? ["pending", t("onboarding.done.stt.downloading")]
          : ["warn", t("onboarding.done.stt.needModels")];

  const embedding = models.find((m) => m.id === EMBEDDING_MODEL);
  const search: [Status, string] = !settings.semanticSearch
    ? ["off", t("onboarding.done.search.off")]
    : embedding?.status.state === "ready"
      ? ["ok", t("onboarding.done.search.ready")]
      : embedding?.downloading
        ? ["pending", t("onboarding.done.search.downloading")]
        : ["warn", t("onboarding.done.search.missing")];

  const rows: { label: string; status: Status; text: string }[] = [
    { label: t("onboarding.done.rows.stt"), status: stt[0], text: stt[1] },
    {
      label: t("onboarding.done.rows.mic"),
      status: audio.mic ? "ok" : "warn",
      text: audio.mic ? t("onboarding.done.heard") : t("onboarding.done.notChecked"),
    },
    {
      label: t("onboarding.done.rows.system"),
      status: audio.system ? "ok" : "warn",
      text: audio.system ? t("onboarding.done.heard") : t("onboarding.done.notChecked"),
    },
    {
      label: t("onboarding.done.rows.ai"),
      status: llmUsed(settings) ? "ok" : "warn",
      text: llmUsed(settings)
        ? settings.translationModel || settings.assistModel
        : t("onboarding.done.aiLater"),
    },
    { label: t("onboarding.done.rows.search"), status: search[0], text: search[1] },
  ];

  return (
    <div className="flex flex-col gap-4">
      <ul className="flex flex-col divide-y rounded-xl border bg-card shadow-xs">
        {rows.map((r) => (
          <li key={r.label} className="flex items-center gap-3 px-4 py-3 text-sm">
            <StatusIcon status={r.status} />
            <span className="font-medium">{r.label}</span>
            <span className="ml-auto truncate text-right text-muted-foreground">{r.text}</span>
          </li>
        ))}
      </ul>
      <p className="text-sm text-muted-foreground">{t("onboarding.done.hint")}</p>
    </div>
  );
}
