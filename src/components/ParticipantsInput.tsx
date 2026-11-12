import { UserPlus, X } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { ContactDialog } from "@/components/Contacts";
import type { Contact, Participant } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { findContact, nameKey, useContactsStore } from "@/stores/contacts";
import { MEETING_LIMITS } from "@/stores/meeting";

function fromContact(c: Contact): Participant {
  return { name: c.name, role: c.role, description: c.description };
}

/**
 * Partecipanti attesi, sempre dalla rubrica: un nome già in rubrica si
 * aggiunge subito; un nome nuovo apre la scheda (ruolo, email, descrizione)
 * da salvare in rubrica, e solo allora diventa partecipante. Clic su un
 * partecipante = completa o modifica la sua scheda.
 */
export function ParticipantsInput(props: {
  value: Participant[];
  onChange: (next: Participant[]) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const id = useId();
  const contacts = useContactsStore((s) => s.contacts);
  const [draft, setDraft] = useState("");
  // Nomi nuovi in attesa della scheda, uno alla volta.
  const [queue, setQueue] = useState<string[]>([]);
  // Partecipante di cui è aperta la scheda.
  const [editing, setEditing] = useState<Participant | null>(null);
  const { value, onChange } = props;
  const full = value.length >= MEETING_LIMITS.participants;

  const has = (list: Participant[], name: string) => list.some((p) => nameKey(p.name) === nameKey(name));

  /** Aggiunge i nomi in rubrica; per gli altri, se `ask`, chiede la scheda. */
  const add = (raw: string, ask: boolean) => {
    const names = raw
      .split(/[,;\n]/)
      .map((n) => n.trim().replace(/\s+/g, " ").slice(0, MEETING_LIMITS.speakerName))
      .filter(Boolean);
    if (names.length === 0) return;
    const next = [...value];
    const unknown: string[] = [];
    for (const name of names) {
      if (has(next, name)) continue;
      const contact = findContact(contacts, name);
      if (contact) {
        if (next.length < MEETING_LIMITS.participants) next.push(fromContact(contact));
      } else if (ask && !unknown.some((n) => nameKey(n) === nameKey(name))) {
        unknown.push(name);
      }
    }
    if (next.length !== value.length) onChange(next);
    if (unknown.length > 0) {
      setQueue((q) => [...q, ...unknown.filter((n) => !q.some((x) => nameKey(x) === nameKey(n)))]);
      setDraft("");
    } else if (ask || next.length !== value.length) {
      setDraft("");
    }
  };

  const saved = (contact: Contact) => {
    const participant = fromContact(contact);
    if (editing) {
      // Scheda completata o rinominata: sostituisce il partecipante.
      const others = value.filter((p) => p !== editing && nameKey(p.name) !== nameKey(contact.name));
      const index = value.indexOf(editing);
      others.splice(Math.max(0, Math.min(index, others.length)), 0, participant);
      onChange(others);
    } else if (!has(value, contact.name) && value.length < MEETING_LIMITS.participants) {
      onChange([...value, participant]);
    }
  };

  const suggestions = contacts.filter((c) => !has(value, c.name));
  const pending = queue[0];

  return (
    <>
      <div
        className={cn(
          "flex min-h-9 w-full flex-wrap items-center gap-1.5 rounded-md border border-input bg-transparent px-2 py-1.5 shadow-xs focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50 dark:bg-input/30",
          props.className,
        )}
      >
        {value.map((p, index) => {
          const inContacts = !!findContact(contacts, p.name);
          return (
            <span
              key={p.name}
              className={cn(
                "inline-flex max-w-full items-center rounded-full text-xs",
                inContacts
                  ? "bg-them/12 text-them"
                  : "border border-dashed border-amber-500/60 text-amber-700 dark:text-amber-400",
              )}
            >
              <button
                type="button"
                className="flex min-w-0 items-center gap-1 rounded-full py-0.5 pl-2.5 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring/50"
                title={inContacts ? p.description || t("participants.edit") : t("participants.notInContacts")}
                onClick={() => setEditing(p)}
              >
                {!inContacts && <UserPlus className="size-3 shrink-0" aria-hidden />}
                <span className="truncate font-medium">{p.name}</span>
                {p.role && <span className="truncate opacity-75">· {p.role}</span>}
              </button>
              <button
                type="button"
                className="mr-1 ml-0.5 rounded-full p-0.5 outline-none hover:bg-current/15 focus-visible:ring-2 focus-visible:ring-ring/50"
                aria-label={t("participants.remove", { name: p.name })}
                onClick={() => onChange(value.filter((_, i) => i !== index))}
              >
                <X className="size-3" aria-hidden />
              </button>
            </span>
          );
        })}
        <input
          value={draft}
          disabled={full}
          list={`${id}-contacts`}
          autoComplete="off"
          aria-label={t("participants.label")}
          placeholder={
            full
              ? t("participants.full", { max: MEETING_LIMITS.participants })
              : value.length > 0
                ? t("participants.more")
                : t("participants.placeholder")
          }
          className="min-w-32 flex-1 bg-transparent px-1 py-0.5 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
          onChange={(e) => {
            const text = e.target.value;
            const native = e.nativeEvent as InputEvent;
            if (/[,;]/.test(text)) add(text, true);
            // Scelta dalla lista della rubrica: aggiunge subito.
            else if (
              native.inputType === "insertReplacementText" ||
              (!native.inputType && findContact(contacts, text))
            )
              add(text, false);
            else setDraft(text);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add(draft, true);
            } else if (e.key === "Backspace" && draft === "" && value.length > 0) {
              onChange(value.slice(0, -1));
            }
          }}
          // Uscendo dal campo si aggiunge solo un nome già in rubrica: la
          // scheda di un nome nuovo si apre con Invio, mai di sorpresa.
          onBlur={() => add(draft, false)}
        />
        <datalist id={`${id}-contacts`}>
          {suggestions.map((c) => (
            <option key={c.id} value={c.name}>
              {c.role}
            </option>
          ))}
        </datalist>
      </div>
      {draft.trim() && !findContact(contacts, draft) && (
        <p className="text-xs text-muted-foreground">{t("participants.newHint", { name: draft.trim() })}</p>
      )}

      {pending && (
        <ContactDialog
          key={pending}
          contact={null}
          initialName={pending}
          title={t("participants.newTitle", { name: pending })}
          description={t("participants.newDescription")}
          onSaved={saved}
          onClose={() => setQueue((q) => q.slice(1))}
        />
      )}
      {editing && !pending && (
        <ContactDialog
          contact={findContact(contacts, editing.name) ?? null}
          initialName={editing.name}
          title={
            findContact(contacts, editing.name)
              ? t("contacts.edit", { name: editing.name })
              : t("participants.newTitle", { name: editing.name })
          }
          description={t("participants.newDescription")}
          onSaved={saved}
          onClose={() => setEditing(null)}
        />
      )}
    </>
  );
}
