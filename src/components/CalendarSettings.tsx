import { AlertTriangle, CalendarDays, Check, ExternalLink, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { type GoogleConsolePage, openGoogleConsole } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useCalendarStore } from "@/stores/calendar";
import { useSettingsStore } from "@/stores/settings";

/** Passi della guida: chiave del testo e pagina della console da aprire. */
const STEPS: { key: string; page?: GoogleConsolePage }[] = [
  { key: "project", page: "project" },
  { key: "api", page: "calendarApi" },
  { key: "branding", page: "branding" },
  { key: "audience", page: "audience" },
  { key: "scopes", page: "dataAccess" },
  { key: "client", page: "clients" },
  { key: "paste" },
  { key: "warning" },
];

/** Google Calendar: client OAuth, collegamento dal browser e guida passo passo. */
export function CalendarSettings() {
  const { t, i18n } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const { status, error, busy, load, connect, cancel, disconnect, sync } = useCalendarStore();
  const hasSecret = useSettingsStore((s) => s.keys["google-client-secret"] ?? false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);

  useEffect(() => {
    void refreshKey("google-client-secret");
    void load();
  }, [refreshKey, load]);

  if (!settings) return null;

  const connected = status?.connected ?? false;
  const configured = settings.googleClientId.trim() !== "" && hasSecret;
  const connecting = status?.connecting ?? false;
  const lastSync = status?.lastSyncMs
    ? new Date(status.lastSyncMs).toLocaleString(i18n.language, {
        day: "numeric",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
      })
    : null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <CalendarDays className="size-4 text-muted-foreground" aria-hidden />
          {t("settings.calendar.title")}
        </CardTitle>
        <CardDescription>{t("settings.calendar.hint")}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {connected ? (
          <div className="flex flex-col gap-3">
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
              <Check className="size-4 text-emerald-600 dark:text-emerald-400" aria-hidden />
              <span>
                {settings.googleCalendarEmail
                  ? t("settings.calendar.connectedAs", { email: settings.googleCalendarEmail })
                  : t("settings.calendar.connected")}
              </span>
            </p>
            {status?.needsReconnect && (
              <p role="alert" className="flex items-start gap-2 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden />
                {t("settings.calendar.needsReconnect")}
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              {status?.syncing
                ? t("settings.calendar.syncing")
                : lastSync
                  ? t("settings.calendar.lastSync", {
                      time: lastSync,
                      added: status?.lastReport?.added ?? 0,
                      updated: status?.lastReport?.updated ?? 0,
                      removed: status?.lastReport?.removed ?? 0,
                    })
                  : t("settings.calendar.neverSynced")}
            </p>
            <div className="flex flex-wrap gap-2">
              {status?.needsReconnect ? (
                <Button disabled={busy} onClick={() => void connect(i18n.resolvedLanguage ?? "it")}>
                  {busy && <Loader2 className="animate-spin" />}
                  {t("settings.calendar.reconnect")}
                </Button>
              ) : (
                <Button variant="outline" disabled={busy || status?.syncing} onClick={() => void sync()}>
                  <RefreshCw className={cn((busy || status?.syncing) && "animate-spin")} />
                  {t("settings.calendar.syncNow")}
                </Button>
              )}
              <Button variant="outline" disabled={busy} onClick={() => setConfirmDisconnect(true)}>
                {t("settings.calendar.disconnect")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <ClientIdField key={settings.googleClientId} saved={settings.googleClientId} onCommit={(googleClientId) => void update({ googleClientId })} />
            <div className="grid gap-1.5 text-sm">
              <span className="font-medium">{t("settings.calendar.clientSecret")}</span>
              <KeyForm provider="google-client-secret" label={t("settings.calendar.clientSecret")} />
            </div>
            {connecting ? (
              <div className="flex flex-wrap items-center gap-3 rounded-xl border bg-muted/30 px-3 py-2 text-sm">
                <Loader2 className="size-4 animate-spin text-primary" aria-hidden />
                <span className="flex-1">{t("settings.calendar.waitingBrowser")}</span>
                <Button size="sm" variant="outline" onClick={() => void cancel()}>
                  {t("common.cancel")}
                </Button>
              </div>
            ) : (
              <div>
                <Button
                  disabled={busy || !configured}
                  title={configured ? undefined : t("settings.calendar.needClient")}
                  onClick={() => void connect(i18n.resolvedLanguage ?? "it")}
                >
                  {busy && <Loader2 className="animate-spin" />}
                  {t("settings.calendar.connect")}
                </Button>
                {!configured && (
                  <p className="mt-1.5 text-xs text-muted-foreground">{t("settings.calendar.needClient")}</p>
                )}
              </div>
            )}
          </div>
        )}

        {(error || (status?.lastError && !status.needsReconnect)) && (
          <p role="alert" className="text-xs break-words text-destructive">
            {error ?? status?.lastError}
          </p>
        )}

        <details className="group rounded-xl border bg-muted/30 px-4 py-3 text-sm" open={!connected && !configured}>
          <summary className="cursor-pointer font-medium select-none">{t("settings.calendar.guide.title")}</summary>
          <p className="mt-2 text-xs text-muted-foreground">{t("settings.calendar.guide.intro")}</p>
          <ol className="mt-3 flex list-decimal flex-col gap-3 pl-5">
            {STEPS.map(({ key, page }) => (
              <li key={key} className="pl-1">
                <p className="font-medium">{t(`settings.calendar.guide.${key}.title`)}</p>
                <p className="mt-0.5 text-xs whitespace-pre-line text-muted-foreground">
                  {t(`settings.calendar.guide.${key}.body`)}
                </p>
                {page && (
                  <Button
                    size="xs"
                    variant="link"
                    className="h-auto px-0"
                    onClick={() => void openGoogleConsole(page)}
                  >
                    <ExternalLink /> {t("settings.calendar.guide.open")}
                  </Button>
                )}
              </li>
            ))}
          </ol>
          <p className="mt-3 text-xs text-muted-foreground">{t("settings.calendar.guide.privacy")}</p>
        </details>
      </CardContent>

      <AlertDialog open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.calendar.disconnectTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("settings.calendar.disconnectConfirm")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className={cn(buttonVariants({ variant: "destructive" }))}
              onClick={() => void disconnect()}
            >
              {t("settings.calendar.disconnect")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}

/** ID client: salvato quando il campo perde il fuoco (o con Invio). */
function ClientIdField({ saved, onCommit }: { saved: string; onCommit: (value: string) => void }) {
  const { t } = useTranslation();
  const [value, setValue] = useState(saved);
  const commit = () => {
    const next = value.trim();
    if (next !== saved) onCommit(next);
  };
  return (
    <label className="grid gap-1.5 text-sm">
      <span className="font-medium">{t("settings.calendar.clientId")}</span>
      <Input
        value={value}
        placeholder="1234567890-abc.apps.googleusercontent.com"
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </label>
  );
}
