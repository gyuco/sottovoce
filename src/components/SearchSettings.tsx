import { Loader2, RotateCw, Sparkles } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { ModelRow } from "@/components/LocalModels";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useModelsStore } from "@/stores/models";
import { useSearchIndexStore } from "@/stores/searchIndex";
import { useSettingsStore } from "@/stores/settings";

const MODEL_ID = "embeddinggemma-2-q4";

/** Ricerca per significato nello storico: interruttore, modello e indice. */
export function SearchSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const status = useSearchIndexStore((s) => s.status);
  const statusError = useSearchIndexStore((s) => s.error);
  const load = useSearchIndexStore((s) => s.load);
  const reindex = useSearchIndexStore((s) => s.reindex);
  const model = useModelsStore((s) => s.models.find((m) => m.id === MODEL_ID));
  const loadModels = useModelsStore((s) => s.load);

  useEffect(() => {
    void load();
    void loadModels();
  }, [load, loadModels]);

  // Il modello appena scaricato (o eliminato) cambia lo stato dell'indice.
  const modelState = model?.status.state;
  useEffect(() => {
    void load();
  }, [load, modelState]);

  if (!settings) return null;
  const ready = model?.status.state === "ready";
  const working = (status?.pending ?? 0) > 0;
  const error = status?.error ?? statusError;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Sparkles className="size-4 text-muted-foreground" aria-hidden />
          {t("settings.search.title")}
        </CardTitle>
        <CardDescription>{t("settings.search.hint")}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <label className="flex items-center justify-between gap-4 text-sm">
          <span>
            {t("settings.search.toggle")}
            <span className="block text-xs text-muted-foreground">{t("settings.search.toggleHint")}</span>
          </span>
          <Switch
            checked={settings.semanticSearch}
            onCheckedChange={(semanticSearch) => void update({ semanticSearch })}
          />
        </label>

        {model && (
          <ul>
            <ModelRow model={model} />
          </ul>
        )}
        {!ready && <p className="text-xs text-muted-foreground">{t("settings.search.downloadHint")}</p>}

        {ready && settings.semanticSearch && status && (
          <div className="flex items-center justify-between gap-3 text-sm">
            <span className="flex items-center gap-2 text-muted-foreground">
              {working && <Loader2 className="size-3.5 animate-spin" aria-hidden />}
              {working
                ? t("settings.search.indexing", { indexed: status.indexed, total: status.total, pending: status.pending })
                : t("settings.search.indexed", { indexed: status.indexed, total: status.total })}
            </span>
            <Button size="xs" variant="outline" disabled={working} onClick={() => void reindex()}>
              <RotateCw /> {t("settings.search.reindex")}
            </Button>
          </div>
        )}
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
