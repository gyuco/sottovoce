import { Check, Loader2, Mail } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { KeyForm } from "@/components/KeyForm";
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
import { type Settings, type SmtpSecurity, errorMessage, testSmtp } from "@/lib/ipc";
import { isValidEmail } from "@/stores/contacts";
import { useSettingsStore } from "@/stores/settings";

/** Porta standard per tipo di cifratura (proposta al cambio). */
const DEFAULT_PORTS: Record<SmtpSecurity, number> = { starttls: 587, tls: 465 };

type TextKey = "emailFromName" | "emailFrom" | "smtpHost" | "smtpUser";

/** Mittente e server SMTP per l'invio del riepilogo; password nel Keychain. */
export function EmailSettings() {
  const { t } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const hasPassword = useSettingsStore((s) => s.keys.smtp ?? false);
  const [test, setTest] = useState<{ state: "idle" | "busy" | "ok" } | { state: "error"; message: string }>({
    state: "idle",
  });

  useEffect(() => {
    void refreshKey("smtp");
  }, [refreshKey]);

  if (!settings) return null;

  const fromInvalid = settings.emailFrom.trim() !== "" && !isValidEmail(settings.emailFrom.trim());

  const runTest = async () => {
    setTest({ state: "busy" });
    try {
      await testSmtp();
      setTest({ state: "ok" });
    } catch (e) {
      setTest({ state: "error", message: errorMessage(e) });
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Mail className="size-4 text-muted-foreground" aria-hidden />
          {t("settings.email.title")}
        </CardTitle>
        <CardDescription>{t("settings.email.hint")}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            key={settings.emailFromName}
            settings={settings}
            field="emailFromName"
            label={t("settings.email.fromName")}
            placeholder={t("settings.email.fromNamePlaceholder")}
          />
          <TextField
            key={settings.emailFrom}
            settings={settings}
            field="emailFrom"
            type="email"
            label={t("settings.email.from")}
            placeholder="nome@azienda.it"
            invalid={fromInvalid}
          />
        </div>
        {fromInvalid && (
          <p role="alert" className="-mt-2 text-xs text-destructive">
            {t("contacts.invalidEmail")}
          </p>
        )}

        <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_6rem_10rem]">
          <TextField
            key={settings.smtpHost}
            settings={settings}
            field="smtpHost"
            label={t("settings.email.host")}
            placeholder="smtp.gmail.com"
          />
          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("settings.email.port")}</span>
            <PortInput key={settings.smtpPort} value={settings.smtpPort} onCommit={(smtpPort) => void update({ smtpPort })} />
          </label>
          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("settings.email.security")}</span>
            <Select
              value={settings.smtpSecurity}
              onValueChange={(v) => {
                const smtpSecurity = v as SmtpSecurity;
                const standard = Object.values(DEFAULT_PORTS).includes(settings.smtpPort);
                void update(
                  standard ? { smtpSecurity, smtpPort: DEFAULT_PORTS[smtpSecurity] } : { smtpSecurity },
                );
              }}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="starttls">{t("settings.email.starttls")}</SelectItem>
                <SelectItem value="tls">{t("settings.email.tls")}</SelectItem>
              </SelectContent>
            </Select>
          </label>
        </div>

        <TextField
          key={settings.smtpUser}
          settings={settings}
          field="smtpUser"
          label={t("settings.email.user")}
          placeholder={settings.emailFrom || t("settings.email.userPlaceholder")}
        />

        <div className="grid gap-1.5 text-sm">
          <span className="font-medium">{t("settings.email.password")}</span>
          <KeyForm provider="smtp" label={t("settings.email.password")} />
          <span className="text-xs text-muted-foreground">{t("settings.email.passwordHint")}</span>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            size="sm"
            variant="outline"
            disabled={
              test.state === "busy" || !settings.smtpHost.trim() || !settings.emailFrom.trim() || !hasPassword
            }
            onClick={() => void runTest()}
          >
            {test.state === "busy" && <Loader2 className="animate-spin" />}
            {t("settings.email.test")}
          </Button>
          {test.state === "ok" && (
            <span role="status" className="flex items-center gap-1 text-xs text-emerald-700 dark:text-emerald-400">
              <Check className="size-3.5" aria-hidden /> {t("settings.email.testOk")}
            </span>
          )}
          {test.state === "error" && (
            <span role="alert" className="min-w-0 flex-1 text-xs break-words text-destructive">
              {test.message}
            </span>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

/** Campo di testo salvato quando perde il fuoco (o con Invio). */
function TextField(props: {
  settings: Settings;
  field: TextKey;
  label: string;
  placeholder?: string;
  type?: string;
  invalid?: boolean;
}) {
  const update = useSettingsStore((s) => s.update);
  const saved = props.settings[props.field];
  // Il genitore rimonta il campo (key) quando il valore salvato cambia.
  const [value, setValue] = useState(saved);
  const commit = () => {
    const next = value.trim();
    if (next !== saved) void update({ [props.field]: next });
  };
  return (
    <label className="grid gap-1.5 text-sm">
      <span className="font-medium">{props.label}</span>
      <Input
        type={props.type ?? "text"}
        value={value}
        placeholder={props.placeholder}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={props.invalid || undefined}
        onChange={(e) => setValue(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </label>
  );
}

function PortInput(props: { value: number; onCommit: (port: number) => void }) {
  const [value, setValue] = useState(String(props.value));
  const commit = () => {
    const port = Number.parseInt(value, 10);
    if (Number.isInteger(port) && port > 0 && port < 65536) {
      if (port !== props.value) props.onCommit(port);
    } else {
      setValue(String(props.value));
    }
  };
  return (
    <Input
      inputMode="numeric"
      value={value}
      onChange={(e) => setValue(e.target.value.replace(/\D/g, ""))}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}
