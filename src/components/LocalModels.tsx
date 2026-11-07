import { Download, HardDrive, Loader2, RotateCw, ShieldCheck, Trash2, X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import type { LocalModel } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useModelsStore } from "@/stores/models";

function formatBytes(n: number, locale: string) {
  const units = ["B", "KB", "MB", "GB"];
  let v = n;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v.toLocaleString(locale, { maximumFractionDigits: v < 10 && i > 0 ? 1 : 0 })} ${units[i]}`;
}

/** Modelli per la trascrizione locale e la diarization: download e stato. */
export function LocalModels() {
  const { t, i18n } = useTranslation();
  const models = useModelsStore((s) => s.models);
  const load = useModelsStore((s) => s.load);
  const listError = useModelsStore((s) => s.errors._list);

  useEffect(() => {
    void load();
  }, [load]);

  const used = models.reduce((sum, m) => sum + m.onDisk, 0);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <HardDrive className="size-4 text-muted-foreground" aria-hidden />
          {t("models.title")}
          <span className="ml-auto text-xs font-normal text-muted-foreground">
            {t("models.used", { size: formatBytes(used, i18n.language) })}
          </span>
        </CardTitle>
        <CardDescription>{t("models.hint")}</CardDescription>
      </CardHeader>
      <CardContent>
        {listError && <p className="text-sm text-destructive">{listError}</p>}
        <ul className="flex flex-col divide-y">
          {models.map((m) => (
            <ModelRow key={m.id} model={m} />
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

function ModelRow({ model }: { model: LocalModel }) {
  const { t, i18n } = useTranslation();
  const progress = useModelsStore((s) => s.progress[model.id]);
  const error = useModelsStore((s) => s.errors[model.id]);
  const busy = useModelsStore((s) => s.busy[model.id] ?? false);
  const { download, cancel, remove, verify } = useModelsStore();
  const fmt = (n: number) => formatBytes(n, i18n.language);

  const downloading = model.downloading || progress !== undefined;
  const state = model.status.state;
  const done = progress?.downloaded ?? (state === "partial" ? model.status.downloaded : 0);
  const pct = model.size > 0 ? Math.min(100, Math.round((done / model.size) * 100)) : 0;

  const badge = downloading
    ? { label: t("models.downloading", { pct }), className: "bg-primary/12 text-primary" }
    : state === "ready"
      ? { label: t("models.ready"), className: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400" }
      : state === "partial"
        ? { label: t("models.partial", { pct }), className: "" }
        : state === "corrupt"
          ? { label: t("models.corrupt"), className: "bg-destructive/12 text-destructive" }
          : { label: t("models.missing"), className: "" };

  return (
    <li className="flex flex-col gap-2 py-3 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        <div className="min-w-0 flex-1">
          <p className="font-medium">{model.name}</p>
          <p className="text-xs text-muted-foreground">
            {t(`models.kind.${model.kind}`)} · {fmt(model.size)}
          </p>
        </div>
        <Badge variant="secondary" className={cn("font-normal", badge.className)}>
          {badge.label}
        </Badge>
        <div className="flex gap-1">
          {downloading ? (
            <Button size="sm" variant="outline" onClick={() => void cancel(model.id)}>
              <X /> {t("models.cancel")}
            </Button>
          ) : state !== "ready" ? (
            <Button size="sm" onClick={() => void download(model.id)} disabled={busy}>
              {state === "partial" ? <RotateCw /> : <Download />}
              {state === "partial"
                ? t("models.resume")
                : state === "corrupt"
                  ? t("models.redownload")
                  : t("models.download")}
            </Button>
          ) : (
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("models.verify")}
              title={t("models.verify")}
              onClick={() => void verify(model.id)}
              disabled={busy}
            >
              {busy ? <Loader2 className="animate-spin" /> : <ShieldCheck />}
            </Button>
          )}
          {!downloading && model.onDisk > 0 && (
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("models.delete")}
              title={t("models.delete")}
              onClick={() => void remove(model.id)}
              disabled={busy}
            >
              <Trash2 />
            </Button>
          )}
        </div>
      </div>
      {downloading && (
        <div className="flex items-center gap-3">
          <div
            role="progressbar"
            aria-label={model.name}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted"
          >
            <div className="h-full bg-primary transition-[width] duration-200" style={{ width: `${pct}%` }} />
          </div>
          <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
            {fmt(done)} / {fmt(model.size)}
          </span>
        </div>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </li>
  );
}
