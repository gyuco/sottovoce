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
import { type CalendarProvider, type ConsolePage, type Settings, openProviderConsole } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useCalendarStore } from "@/stores/calendar";
import { useSettingsStore } from "@/stores/settings";

/** Passi della guida: chiave del testo e pagina della console da aprire. */
const STEPS: Record<CalendarProvider, { key: string; page?: ConsolePage }[]> = {
  google: [
    { key: "project", page: "googleProject" },
    { key: "api", page: "googleCalendarApi" },
    { key: "branding", page: "googleBranding" },
    { key: "audience", page: "googleAudience" },
    { key: "scopes", page: "googleDataAccess" },
    { key: "client", page: "googleClients" },
    { key: "paste" },
    { key: "warning" },
  ],
  microsoft: [
    { key: "signin", page: "microsoftAppRegistrations" },
    { key: "register", page: "microsoftNewRegistration" },
    { key: "ids" },
    { key: "publicClient" },
    { key: "permissions" },
    { key: "paste" },
    { key: "warning" },
  ],
};

function accountEmail(provider: CalendarProvider, settings: Settings): string {
  return provider === "google" ? settings.googleCalendarEmail : settings.microsoftCalendarEmail;
}

/** Un calendario (Google o Microsoft): client OAuth, collegamento dal browser e guida passo passo. */
export function CalendarSettings({ provider }: { provider: CalendarProvider }) {
  const { t, i18n } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const hasSecret = useSettingsStore((s) => s.keys["google-client-secret"] ?? false);
  const status = useCalendarStore((s) => s.status[provider]);
  const error = useCalendarStore((s) => s.error[provider]);
  const busy = useCalendarStore((s) => s.busy[provider]);
  const { load, connect, cancel, disconnect, sync } = useCalendarStore();
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const p = `settings.calendar.${provider}`;
  const c = "settings.calendar.common";

  useEffect(() => {
    if (provider === "google") void refreshKey("google-client-secret");
    void load(provider);
  }, [provider, refreshKey, load]);

  if (!settings) return null;

  const clientId = provider === "google" ? settings.googleClientId : settings.microsoftClientId;
  const configured = clientId.trim() !== "" && (provider === "microsoft" || hasSecret);
  const connected = status?.connected ?? false;
  const connecting = status?.connecting ?? false;
  const email = accountEmail(provider, settings);
  const uiLanguage = i18n.resolvedLanguage ?? "it";
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
          {t(`${p}.title`)}
        </CardTitle>
        <CardDescription>{t(`${p}.hint`)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {connected ? (
          <div className="flex flex-col gap-3">
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
              <Check className="size-4 text-emerald-600 dark:text-emerald-400" aria-hidden />
              <span>{email ? t(`${c}.connectedAs`, { email }) : t(`${c}.connected`)}</span>
            </p>
            {status?.needsReconnect && (
              <p role="alert" className="flex items-start gap-2 text-sm text-destructive">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden />
                {t(`${p}.needsReconnect`)}
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              {status?.syncing
                ? t(`${c}.syncing`)
                : lastSync
                  ? t(`${c}.lastSync`, {
                      time: lastSync,
                      added: status?.lastReport?.added ?? 0,
                      updated: status?.lastReport?.updated ?? 0,
                      removed: status?.lastReport?.removed ?? 0,
                    })
                  : t(`${c}.neverSynced`)}
            </p>
            <div className="flex flex-wrap gap-2">
              {status?.needsReconnect ? (
                <Button disabled={busy} onClick={() => void connect(provider, uiLanguage)}>
                  {busy && <Loader2 className="animate-spin" />}
                  {t(`${c}.reconnect`)}
                </Button>
              ) : (
                <Button variant="outline" disabled={busy || status?.syncing} onClick={() => void sync(provider)}>
                  <RefreshCw className={cn((busy || status?.syncing) && "animate-spin")} />
                  {t(`${c}.syncNow`)}
                </Button>
              )}
              <Button variant="outline" disabled={busy} onClick={() => setConfirmDisconnect(true)}>
                {t(`${c}.disconnect`)}
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <TextField
              key={`id-${clientId}`}
              label={t(`${c}.clientId`)}
              placeholder={
                provider === "google"
                  ? "1234567890-abc.apps.googleusercontent.com"
                  : "00000000-0000-0000-0000-000000000000"
              }
              saved={clientId}
              onCommit={(value) =>
                void update(provider === "google" ? { googleClientId: value } : { microsoftClientId: value })
              }
            />
            {provider === "google" ? (
              <div className="grid gap-1.5 text-sm">
                <span className="font-medium">{t(`${p}.clientSecret`)}</span>
                <KeyForm provider="google-client-secret" label={t(`${p}.clientSecret`)} />
              </div>
            ) : (
              <TextField
                key={`tenant-${settings.microsoftTenantId}`}
                label={t(`${p}.tenantId`)}
                placeholder="common"
                hint={t(`${p}.tenantHint`)}
                saved={settings.microsoftTenantId}
                onCommit={(microsoftTenantId) => void update({ microsoftTenantId })}
              />
            )}
            {connecting ? (
              <div className="flex flex-wrap items-center gap-3 rounded-xl border bg-muted/30 px-3 py-2 text-sm">
                <Loader2 className="size-4 animate-spin text-primary" aria-hidden />
                <span className="flex-1">{t(`${c}.waitingBrowser`)}</span>
                <Button size="sm" variant="outline" onClick={() => void cancel(provider)}>
                  {t("common.cancel")}
                </Button>
              </div>
            ) : (
              <div>
                <Button disabled={busy || !configured} onClick={() => void connect(provider, uiLanguage)}>
                  {busy && <Loader2 className="animate-spin" />}
                  {t(`${p}.connect`)}
                </Button>
                {!configured && <p className="mt-1.5 text-xs text-muted-foreground">{t(`${p}.needClient`)}</p>}
              </div>
            )}
          </div>
        )}

        {(error || (status?.lastError && !status.needsReconnect)) && (
          <p role="alert" className="text-xs break-words text-destructive">
            {error ?? status?.lastError}
          </p>
        )}

        <details className="rounded-xl border bg-muted/30 px-4 py-3 text-sm" open={!connected && !configured}>
          <summary className="cursor-pointer font-medium select-none">{t(`${p}.guide.title`)}</summary>
          <p className="mt-2 text-xs text-muted-foreground">{t(`${p}.guide.intro`)}</p>
          <ol className="mt-3 flex list-decimal flex-col gap-3 pl-5">
            {STEPS[provider].map(({ key, page }) => (
              <li key={key} className="pl-1">
                <p className="font-medium">{t(`${p}.guide.${key}.title`)}</p>
                <p className="mt-0.5 text-xs whitespace-pre-line text-muted-foreground">
                  {t(`${p}.guide.${key}.body`)}
                </p>
                {page && (
                  <Button
                    size="xs"
                    variant="link"
                    className="h-auto px-0"
                    onClick={() => void openProviderConsole(page)}
                  >
                    <ExternalLink /> {t(`${c}.openConsole`)}
                  </Button>
                )}
              </li>
            ))}
          </ol>
          <p className="mt-3 text-xs text-muted-foreground">{t(`${p}.guide.privacy`)}</p>
        </details>
      </CardContent>

      <AlertDialog open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t(`${p}.disconnectTitle`)}</AlertDialogTitle>
            <AlertDialogDescription>{t(`${p}.disconnectConfirm`)}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className={cn(buttonVariants({ variant: "destructive" }))}
              onClick={() => void disconnect(provider)}
            >
              {t(`${c}.disconnect`)}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}

/** Campo salvato quando perde il fuoco (o con Invio). */
function TextField(props: {
  label: string;
  placeholder: string;
  hint?: string;
  saved: string;
  onCommit: (value: string) => void;
}) {
  const [value, setValue] = useState(props.saved);
  const commit = () => {
    const next = value.trim();
    if (next !== props.saved) props.onCommit(next);
  };
  return (
    <label className="grid gap-1.5 text-sm">
      <span className="font-medium">{props.label}</span>
      <Input
        value={value}
        placeholder={props.placeholder}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
      {props.hint && <span className="text-xs text-muted-foreground">{props.hint}</span>}
    </label>
  );
}
