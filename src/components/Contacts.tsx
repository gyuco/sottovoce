import { Loader2, Mail, Pencil, Plus, Search, Trash2, UserRound } from "lucide-react";
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { type ContactFields, ContactFieldsForm } from "@/components/SpeakerCard";
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { type Contact, errorMessage } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { isValidEmail, nameKey, useContactsStore } from "@/stores/contacts";

const EMPTY: ContactFields = { name: "", role: "", description: "", email: "" };

/** Rubrica: le persone incontrate nei meeting, con ruolo, email e descrizione. */
export function Contacts() {
  const { t } = useTranslation();
  const { contacts, loaded, error, load, remove } = useContactsStore();
  const [query, setQuery] = useState("");
  // `null` = chiuso; `{}` senza id = nuovo contatto.
  const [editing, setEditing] = useState<Contact | "new" | null>(null);
  const [deleting, setDeleting] = useState<Contact | null>(null);

  useEffect(() => {
    void load();
  }, [load]);

  const shown = useMemo(() => {
    const q = nameKey(query);
    if (!q) return contacts;
    return contacts.filter((c) =>
      [c.name, c.role, c.email, c.description].some((field) => nameKey(field).includes(q)),
    );
  }, [contacts, query]);

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-6 py-5">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{t("contacts.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("contacts.hint")}</p>
        </div>
        <Button onClick={() => setEditing("new")}>
          <Plus /> {t("contacts.add")}
        </Button>
      </header>

      <div className="relative">
        <Search
          className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden
        />
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("contacts.search")}
          aria-label={t("contacts.search")}
          className="pl-8"
        />
      </div>

      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}

      {!loaded ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" aria-hidden /> {t("history.loading")}
        </p>
      ) : shown.length === 0 ? (
        <p className="rounded-2xl border border-dashed px-6 py-10 text-center text-sm text-muted-foreground">
          {query ? t("contacts.noResults") : t("contacts.empty")}
        </p>
      ) : (
        <ul className="flex flex-col divide-y rounded-2xl border bg-card shadow-xs">
          {shown.map((c) => (
            <li key={c.id} className="flex items-start gap-3 px-4 py-3">
              <span className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-full bg-them/12 text-sm font-semibold text-them">
                {initials(c.name) || <UserRound className="size-4" aria-hidden />}
              </span>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <p className="truncate text-sm">
                  <span className="font-medium">{c.name}</span>
                  {c.role && <span className="text-muted-foreground"> · {c.role}</span>}
                </p>
                {c.email && (
                  <p className="flex items-center gap-1 truncate text-xs text-muted-foreground select-text">
                    <Mail className="size-3 shrink-0" aria-hidden /> {c.email}
                  </p>
                )}
                {c.description && (
                  <p className="line-clamp-2 text-xs whitespace-pre-line text-muted-foreground">
                    {c.description}
                  </p>
                )}
              </div>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={t("contacts.edit", { name: c.name })}
                title={t("contacts.edit", { name: c.name })}
                onClick={() => setEditing(c)}
              >
                <Pencil />
              </Button>
              <Button
                size="icon-sm"
                variant="ghost"
                className="text-destructive hover:text-destructive"
                aria-label={t("contacts.delete", { name: c.name })}
                title={t("contacts.delete", { name: c.name })}
                onClick={() => setDeleting(c)}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}

      {editing !== null && (
        <ContactDialog contact={editing === "new" ? null : editing} onClose={() => setEditing(null)} />
      )}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("contacts.deleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("contacts.deleteConfirm", { name: deleting?.name ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className={cn(buttonVariants({ variant: "destructive" }))}
              onClick={() => deleting && void remove(deleting.id)}
            >
              {t("contacts.deleteAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/**
 * Nuovo contatto o modifica di una scheda. `initialName` precompila il nome di
 * un contatto nuovo (es. partecipante scritto a mano); `onSaved` riceve la
 * scheda salvata.
 */
export function ContactDialog(props: {
  contact: Contact | null;
  initialName?: string;
  title?: string;
  description?: string;
  onClose: () => void;
  onSaved?: (contact: Contact) => void;
}) {
  const { t } = useTranslation();
  const save = useContactsStore((s) => s.save);
  const { contact } = props;
  const [fields, setFields] = useState<ContactFields>(
    contact
      ? { name: contact.name, role: contact.role, description: contact.description, email: contact.email }
      : { ...EMPTY, name: props.initialName ?? "" },
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [emailInvalid, setEmailInvalid] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    // Il dialogo può stare dentro un altro form (avvio della call): il submit
    // risalirebbe l'albero React attraverso il portal.
    e.stopPropagation();
    const email = fields.email.trim();
    if (email && !isValidEmail(email)) {
      setEmailInvalid(true);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const saved = await save({ ...fields, email, id: contact?.id ?? null });
      props.onSaved?.(saved);
      props.onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && props.onClose()}>
      <DialogContent closeLabel={t("common.close")}>
        <form onSubmit={(e) => void submit(e)} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>
              {props.title ?? (contact ? t("contacts.edit", { name: contact.name }) : t("contacts.add"))}
            </DialogTitle>
            <DialogDescription>{props.description ?? t("contacts.formHint")}</DialogDescription>
          </DialogHeader>
          <ContactFieldsForm
            autoFocus
            value={fields}
            emailInvalid={emailInvalid}
            onChange={(next) => {
              setEmailInvalid(false);
              setFields(next);
            }}
          />
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={props.onClose}>
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

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
}
