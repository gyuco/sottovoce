import { Check, ExternalLink, Loader2, Mail, Send } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { TEXTAREA_CLASS } from "@/components/SpeakerCard";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { UI_LANGUAGES } from "@/i18n";
import { meetingPeopleNames, parseAddresses, summaryEmail, summarySubject } from "@/lib/email";
import {
  type MeetingDetails,
  type Summary,
  errorMessage,
  openEmailDraft,
  sendEmail,
} from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { findContact, isValidEmail, nameKey, useContactsStore } from "@/stores/contacts";
import { useSettingsStore } from "@/stores/settings";

type Status =
  | { kind: "idle" }
  | { kind: "busy"; action: "send" | "draft" }
  | { kind: "sent"; count: number }
  | { kind: "draft" }
  | { kind: "error"; message: string };

/**
 * Invio del riepilogo: destinatari dai partecipanti con email in rubrica più
 * indirizzi liberi, oggetto, messaggio personale e anteprima dell'email.
 * "Invia" usa il server SMTP delle impostazioni; "Apri nel client di posta"
 * crea una bozza da rivedere e spedire da lì. Va montato solo quando è
 * aperto: lo stato iniziale si legge al montaggio.
 */
export function SendSummaryDialog(props: {
  onOpenChange: (open: boolean) => void;
  details: MeetingDetails;
  summary: Summary;
}) {
  const { t, i18n } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const smtpReady = useSettingsStore(
    (s) => !!s.settings?.emailFrom.trim() && !!s.settings.smtpHost.trim() && !!s.keys.smtp,
  );
  const refreshKey = useSettingsStore((s) => s.refreshKey);
  const { contacts, load } = useContactsStore();
  const { details, summary } = props;

  // Etichette dell'email nella lingua del riepilogo, se l'interfaccia la ha.
  const emailLanguage = UI_LANGUAGES.some((l) => l.code === summary.language)
    ? summary.language
    : (i18n.resolvedLanguage ?? "it");
  const te = useMemo(() => i18n.getFixedT(emailLanguage), [i18n, emailLanguage]);

  // `null` = scelta iniziale: tutti i partecipanti con un'email.
  const [picked, setPicked] = useState<string[] | null>(null);
  const [extra, setExtra] = useState("");
  const [subject, setSubject] = useState(() => summarySubject(details, te));
  const [note, setNote] = useState("");
  const [preview, setPreview] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  useEffect(() => {
    void load();
    void refreshKey("smtp");
  }, [load, refreshKey]);

  /** Partecipanti attesi e parlanti nominati, con l'email della scheda in rubrica. */
  const people = useMemo(() => {
    const seen = new Set<string>();
    return meetingPeopleNames(details).flatMap((name) => {
      const contact = findContact(contacts, name);
      const key = nameKey(contact?.name ?? name);
      if (seen.has(key)) return [];
      seen.add(key);
      return [{ name: contact?.name ?? name, role: contact?.role ?? "", email: contact?.email ?? "" }];
    });
  }, [details, contacts]);

  const selected = picked ?? people.map((p) => p.email).filter(Boolean);

  const extraList = parseAddresses(extra);
  const invalid = extraList.filter((a) => !isValidEmail(a));
  const recipients = [...new Set([...selected, ...extraList].map((a) => a.trim()).filter(Boolean))];

  const message = useMemo(
    () =>
      summaryEmail(
        {
          details,
          summary,
          contacts,
          note,
          senderName: settings?.emailFromName ?? "",
          t: te,
          locale: emailLanguage,
        },
        recipients,
        subject.trim() || summarySubject(details, te),
      ),
    // `recipients` cambia a ogni render: l'anteprima dipende solo dal contenuto.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [details, summary, contacts, note, settings?.emailFromName, te, emailLanguage, subject],
  );

  const busy = status.kind === "busy";
  const blocked = busy || recipients.length === 0 || invalid.length > 0;

  const run = async (action: "send" | "draft") => {
    setStatus({ kind: "busy", action });
    const email = { ...message, to: recipients };
    try {
      if (action === "send") {
        const count = await sendEmail(email);
        setStatus({ kind: "sent", count });
      } else {
        await openEmailDraft(email);
        setStatus({ kind: "draft" });
      }
    } catch (e) {
      setStatus({ kind: "error", message: errorMessage(e) });
    }
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!blocked) void run(smtpReady ? "send" : "draft");
  };

  const toggle = (email: string, on: boolean) =>
    setPicked(on ? [...selected, email] : selected.filter((x) => x !== email));

  return (
    <Dialog open onOpenChange={props.onOpenChange}>
      <DialogContent closeLabel={t("common.close")} className="sm:max-w-2xl">
        <form onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Mail className="size-4 text-primary" aria-hidden />
              {t("email.dialog.title")}
            </DialogTitle>
            <DialogDescription>
              {details.meeting.name || t("history.untitled")}
            </DialogDescription>
          </DialogHeader>

          <fieldset className="grid gap-2">
            <legend className="mb-1.5 text-sm font-medium">{t("email.dialog.to")}</legend>
            {people.length > 0 ? (
              <ul className="flex flex-col divide-y rounded-md border">
                {people.map((p) => (
                  <li key={p.name}>
                    <label
                      className={cn(
                        "flex items-center gap-3 px-3 py-2 text-sm",
                        p.email ? "cursor-pointer" : "text-muted-foreground",
                      )}
                    >
                      <input
                        type="checkbox"
                        className="size-4 accent-primary"
                        disabled={!p.email}
                        checked={!!p.email && selected.includes(p.email)}
                        onChange={(e) => toggle(p.email, e.target.checked)}
                      />
                      <span className="min-w-0 flex-1 truncate">
                        <span className="font-medium text-foreground">{p.name}</span>
                        {p.role && <span className="text-muted-foreground"> · {p.role}</span>}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {p.email || t("email.dialog.noEmail")}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-xs text-muted-foreground">{t("email.dialog.noParticipants")}</p>
            )}
            <Input
              value={extra}
              placeholder={t("email.dialog.extraPlaceholder")}
              aria-label={t("email.dialog.extra")}
              aria-invalid={invalid.length > 0 || undefined}
              spellCheck={false}
              onChange={(e) => setExtra(e.target.value)}
            />
            {invalid.length > 0 && (
              <p role="alert" className="text-xs text-destructive">
                {t("email.dialog.invalid", { address: invalid.join(", ") })}
              </p>
            )}
          </fieldset>

          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("email.dialog.subject")}</span>
            <Input value={subject} maxLength={300} onChange={(e) => setSubject(e.target.value)} />
          </label>

          <label className="grid gap-1.5 text-sm">
            <span className="font-medium">{t("email.dialog.note")}</span>
            <textarea
              rows={3}
              value={note}
              maxLength={4000}
              placeholder={t("email.dialog.notePlaceholder")}
              onChange={(e) => setNote(e.target.value)}
              className={TEXTAREA_CLASS}
            />
          </label>

          <div className="grid gap-2">
            <Button
              type="button"
              size="xs"
              variant="outline"
              className="justify-self-start"
              aria-expanded={preview}
              onClick={() => setPreview((p) => !p)}
            >
              {preview ? t("email.dialog.hidePreview") : t("email.dialog.preview")}
            </Button>
            {preview && (
              <iframe
                title={t("email.dialog.preview")}
                sandbox=""
                srcDoc={message.html}
                className="h-96 w-full rounded-md border bg-white"
              />
            )}
          </div>

          {!smtpReady && (
            <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">
              {t("email.dialog.smtpMissing")}
            </p>
          )}
          {status.kind === "sent" && (
            <p role="status" className="flex items-center gap-1.5 text-sm text-emerald-700 dark:text-emerald-400">
              <Check className="size-4" aria-hidden /> {t("email.dialog.sent", { count: status.count })}
            </p>
          )}
          {status.kind === "draft" && (
            <p role="status" className="flex items-center gap-1.5 text-sm text-emerald-700 dark:text-emerald-400">
              <Check className="size-4" aria-hidden /> {t("email.dialog.draftOpened")}
            </p>
          )}
          {status.kind === "error" && (
            <p role="alert" className="text-sm text-destructive">
              {status.message}
            </p>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={blocked}
              onClick={() => void run("draft")}
            >
              {busy && status.action === "draft" ? <Loader2 className="animate-spin" /> : <ExternalLink />}
              {t("email.dialog.draft")}
            </Button>
            <Button type="submit" disabled={blocked || !smtpReady}>
              {busy && status.action === "send" ? <Loader2 className="animate-spin" /> : <Send />}
              {t("email.dialog.send")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
