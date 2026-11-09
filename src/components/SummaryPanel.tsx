import { Check, Copy, Loader2, RefreshCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useCopy } from "@/lib/hooks";
import {
  type ActionItem,
  type Language,
  type Summary,
  errorMessage,
  updateSummaryChecklist,
} from "@/lib/ipc";
import { summaryMarkdown } from "@/lib/markdown";
import { cn } from "@/lib/utils";

/** Riepilogo finale: overview, key points, checklist spuntabile e copia. */
export function SummaryPanel(props: {
  summary: Summary;
  onChange: (summary: Summary) => void;
  onRegenerate?: (language: Language) => Promise<Summary>;
}) {
  const { t } = useTranslation();
  const { copied, copy } = useCopy();
  const [busy, setBusy] = useState(false);
  const [language, setLanguage] = useState<Language>(props.summary.language);
  const [error, setError] = useState<string | null>(null);

  const toggle = async (index: number) => {
    const items: ActionItem[] = props.summary.actionItems.map((item, i) =>
      i === index ? { ...item, done: !item.done } : item,
    );
    try {
      props.onChange(await updateSummaryChecklist(props.summary.meetingId, items));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const regenerate = async () => {
    if (!props.onRegenerate) return;
    setBusy(true);
    setError(null);
    try {
      props.onChange(await props.onRegenerate(language));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const copyAll = () => copy(summaryMarkdown(props.summary, t));

  return (
    <section className="flex flex-col gap-3 rounded-2xl border bg-card p-4 shadow-xs">
      <header className="flex flex-wrap items-center gap-2">
        <h2 className="min-w-0 flex-1 text-sm font-semibold">{t("summary.title")}</h2>
        {props.onRegenerate && (
          <>
            <Select value={language} onValueChange={(v) => setLanguage(v as Language)}>
              <SelectTrigger size="sm" className="w-24" aria-label={t("summary.language")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="it">{t("settings.language.it")}</SelectItem>
                <SelectItem value="en">{t("settings.language.en")}</SelectItem>
              </SelectContent>
            </Select>
            <Button
              size="xs"
              variant="outline"
              disabled={busy}
              onClick={() => void regenerate()}
              title={t("summary.regenerate")}
            >
              {busy ? <Loader2 className="animate-spin" /> : <RefreshCw />}
              {t("summary.regenerate")}
            </Button>
          </>
        )}
        <Button size="xs" variant="ghost" onClick={copyAll} title={t("summary.copy")}>
          {copied ? <Check className="text-emerald-600" /> : <Copy />}
          {copied ? t("common.copied") : t("summary.copy")}
        </Button>
      </header>

      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}

      {props.summary.overview && (
        <p className="text-sm leading-relaxed select-text">{props.summary.overview}</p>
      )}

      {props.summary.keyPoints.length > 0 && (
        <div className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
            {t("summary.keyPoints")}
          </h3>
          {props.summary.keyPoints.map((topic) => (
            <div key={topic.topic}>
              <p className="text-sm font-medium">{topic.topic}</p>
              <ul className="ml-4 list-disc text-sm text-muted-foreground">
                {topic.points.map((point) => (
                  <li key={point} className="select-text">
                    {point}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}

      {props.summary.actionItems.length > 0 && (
        <div className="flex flex-col gap-1">
          <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
            {t("summary.actions")}
          </h3>
          <ul className="flex flex-col gap-1">
            {props.summary.actionItems.map((item, index) => (
              <li key={`${index}-${item.text}`}>
                <label className="flex items-start gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={item.done}
                    className="mt-0.5 size-4 shrink-0 accent-primary"
                    onChange={() => void toggle(index)}
                  />
                  <span
                    className={cn(
                      "select-text leading-snug",
                      item.done && "text-muted-foreground line-through",
                    )}
                  >
                    {item.text}
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
