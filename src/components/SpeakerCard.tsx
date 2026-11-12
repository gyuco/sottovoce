import { BookUser, Loader2 } from "lucide-react";
import { type FormEvent, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
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
import { Switch } from "@/components/ui/switch";
import type { Contact } from "@/lib/ipc";
import { errorMessage } from "@/lib/ipc";
import { CONTACT_LIMITS, findContact, isValidEmail, nameKey, useContactsStore } from "@/stores/contacts";

export const TEXTAREA_CLASS =
  "min-h-16 w-full resize-y rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30";

export interface ContactFields {
  name: string;
  role: string;
  description: string;
  email: string;
}

const EMPTY: ContactFields = { name: "", role: "", description: "", email: "" };

function fieldsOf(contact: Contact | undefined, name: string): ContactFields {
  return contact
    ? { name: contact.name, role: contact.role, description: contact.description, email: contact.email }
    : { ...EMPTY, name };
}

/** Campi della scheda (nome, ruolo, email, descrizione), condivisi con la Rubrica. */
export function ContactFieldsForm(props: {
  value: ContactFields;
  onChange: (next: ContactFields) => void;
  /** Suggerimenti per il nome (rubrica). */
  suggestions?: Contact[];
  /** Altri nomi suggeriti (partecipanti attesi non in rubrica). */
  extraNames?: string[];
  autoFocus?: boolean;
  emailInvalid?: boolean;
}) {
  const { t } = useTranslation();
  const id = useId();
  const { value, onChange } = props;
  const set = (patch: Partial<ContactFields>) => onChange({ ...value, ...patch });
  return (
    <div className="grid gap-3">
      <label className="grid gap-1.5 text-sm">
        <span className="font-medium">{t("contacts.fields.name")}</span>
        <Input
          autoFocus={props.autoFocus}
          required
          value={value.name}
          maxLength={CONTACT_LIMITS.name}
          list={props.suggestions ? `${id}-names` : undefined}
          autoComplete="off"
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => set({ name: e.target.value })}
        />
        {props.suggestions && (
          <datalist id={`${id}-names`}>
            {props.suggestions.map((c) => (
              <option key={c.id} value={c.name}>
                {c.role}
              </option>
            ))}
            {props.extraNames
              ?.filter((n) => !findContact(props.suggestions ?? [], n))
              .map((n) => <option key={`extra-${n}`} value={n} />)}
          </datalist>
        )}
      </label>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="grid gap-1.5 text-sm">
          <span className="font-medium">{t("contacts.fields.role")}</span>
          <Input
            value={value.role}
            maxLength={CONTACT_LIMITS.role}
            placeholder={t("contacts.fields.rolePlaceholder")}
            onChange={(e) => set({ role: e.target.value })}
          />
        </label>
        <label className="grid gap-1.5 text-sm">
          <span className="font-medium">{t("contacts.fields.email")}</span>
          <Input
            type="email"
            value={value.email}
            maxLength={CONTACT_LIMITS.email}
            placeholder={t("contacts.fields.emailPlaceholder")}
            aria-invalid={props.emailInvalid || undefined}
            spellCheck={false}
            onChange={(e) => set({ email: e.target.value })}
          />
        </label>
      </div>
      {props.emailInvalid && (
        <p role="alert" className="-mt-1 text-xs text-destructive">
          {t("contacts.invalidEmail")}
        </p>
      )}
      <label className="grid gap-1.5 text-sm">
        <span className="font-medium">{t("contacts.fields.description")}</span>
        <textarea
          rows={3}
          value={value.description}
          maxLength={CONTACT_LIMITS.description}
          placeholder={t("contacts.fields.descriptionPlaceholder")}
          onChange={(e) => set({ description: e.target.value })}
          className={TEXTAREA_CLASS}
        />
      </label>
    </div>
  );
}

/**
 * Scheda di un parlante di "Loro": nome (con suggerimenti dalla rubrica),
 * ruolo, email, descrizione. Il nome va al parlante; la scheda, se scelto, in
 * rubrica (unita per nome a un contatto già presente). Va montata solo
 * quando è aperta: lo stato iniziale si legge al montaggio.
 */
export function SpeakerCard(props: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Nome attuale ("" = ancora "Speaker N"). */
  name: string;
  /** Etichetta di default, es. "Speaker 2". */
  fallback: string;
  onSave: (name: string) => Promise<void> | void;
  /** Partecipanti attesi: scelta rapida in cima alla scheda. */
  participants?: string[];
  /** Nomi già dati ad altri parlanti (esclusi dalla scelta rapida). */
  taken?: string[];
}) {
  const { t } = useTranslation();
  const { contacts, load, save } = useContactsStore();
  const quick = (props.participants ?? []).filter(
    (n) => !(props.taken ?? []).some((x) => nameKey(x) === nameKey(n)),
  );
  // Montata a ogni apertura: parte dalla scheda in rubrica del nome attuale.
  const [fields, setFields] = useState<ContactFields>(() =>
    fieldsOf(findContact(useContactsStore.getState().contacts, props.name), props.name),
  );
  const [toContacts, setToContacts] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [emailInvalid, setEmailInvalid] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  const linked = findContact(contacts, fields.name);

  const onChange = (next: ContactFields) => {
    setEmailInvalid(false);
    // Nome scelto dalla rubrica: si compila il resto della scheda.
    const match = next.name !== fields.name ? findContact(contacts, next.name) : undefined;
    if (match && findContact(contacts, fields.name)?.id !== match.id) {
      setFields({ ...fieldsOf(match, match.name), name: next.name });
    } else {
      setFields(next);
    }
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const name = fields.name.trim();
    const email = fields.email.trim();
    if (email && !isValidEmail(email)) {
      setEmailInvalid(true);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      let finalName = name;
      if (toContacts && name) {
        const contact = await save({ ...fields, name, email, id: linked?.id ?? null });
        finalName = contact.name;
      }
      await props.onSave(finalName);
      props.onOpenChange(false);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent closeLabel={t("common.close")}>
        <form onSubmit={(e) => void submit(e)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              {t("speakers.card.title", { name: props.name || props.fallback })}
              {linked && (
                <Badge variant="secondary" className="gap-1 font-normal">
                  <BookUser className="size-3" aria-hidden />
                  {t("speakers.card.inContacts")}
                </Badge>
              )}
            </DialogTitle>
            <DialogDescription>{t("speakers.card.hint")}</DialogDescription>
          </DialogHeader>

          {quick.length > 0 && (
            <div className="grid gap-1.5">
              <span className="text-sm font-medium">{t("speakers.card.whoIs")}</span>
              <div className="flex flex-wrap gap-1.5">
                {quick.map((n) => (
                  <Button
                    key={n}
                    type="button"
                    size="xs"
                    variant={nameKey(n) === nameKey(fields.name) ? "default" : "outline"}
                    className="rounded-full"
                    onClick={() => onChange({ ...fields, name: n })}
                  >
                    {n}
                  </Button>
                ))}
              </div>
            </div>
          )}

          <ContactFieldsForm
            autoFocus
            value={fields}
            onChange={onChange}
            suggestions={contacts}
            extraNames={props.participants}
            emailInvalid={emailInvalid}
          />

          <label className="flex items-center justify-between gap-4 text-sm">
            <span>
              {t("speakers.card.saveToContacts")}
              <span className="block text-xs text-muted-foreground">
                {linked ? t("speakers.card.updateHint") : t("speakers.card.saveHint")}
              </span>
            </span>
            <Switch checked={toContacts} onCheckedChange={setToContacts} />
          </label>

          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}

          <DialogFooter>
            {props.name && (
              <Button
                type="button"
                variant="ghost"
                className="sm:mr-auto"
                disabled={saving}
                onClick={() => {
                  void Promise.resolve(props.onSave("")).then(() => props.onOpenChange(false));
                }}
              >
                {t("speakers.card.clear")}
              </Button>
            )}
            <Button type="button" variant="outline" onClick={() => props.onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={saving || !fields.name.trim()}>
              {saving && <Loader2 className="animate-spin" />}
              {t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
