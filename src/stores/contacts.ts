import { create } from "zustand";

import {
  type Contact,
  type ContactInput,
  deleteContact,
  errorMessage,
  listContacts,
  saveContact,
} from "@/lib/ipc";

interface ContactsState {
  contacts: Contact[];
  loaded: boolean;
  error: string | null;
  load: () => Promise<void>;
  /** Crea o aggiorna; lancia l'errore (lo mostra il form). */
  save: (input: ContactInput) => Promise<Contact>;
  remove: (id: number) => Promise<void>;
}

/** Rubrica: schede delle persone, legate ai parlanti per nome. */
export const useContactsStore = create<ContactsState>((set, get) => ({
  contacts: [],
  loaded: false,
  error: null,

  load: async () => {
    try {
      set({ contacts: await listContacts(), loaded: true, error: null });
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  save: async (input) => {
    const contact = await saveContact(input);
    await get().load();
    return contact;
  },

  remove: async (id) => {
    try {
      await deleteContact(id);
      await get().load();
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },
}));

/** Chiave di confronto dei nomi, come `name_key` in `sv-store`. */
export function nameKey(name: string): string {
  return name
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
}

/** Contatto con lo stesso nome (senza maiuscole/accenti), se c'è. */
export function findContact(contacts: Contact[], name: string): Contact | undefined {
  const key = nameKey(name);
  return key ? contacts.find((c) => nameKey(c.name) === key) : undefined;
}

/** Controllo essenziale, allineato a `is_valid_email` in `sv-store`. */
export function isValidEmail(email: string): boolean {
  if (email.length > 254 || /[\s<>()[\],;:"\\]/.test(email)) return false;
  const at = email.indexOf("@");
  if (at <= 0 || email.indexOf("@", at + 1) !== -1) return false;
  const domain = email.slice(at + 1);
  return domain.includes(".") && !domain.startsWith(".") && !domain.endsWith(".") && !domain.includes("..");
}

export const CONTACT_LIMITS = { name: 60, role: 100, description: 1000, email: 254 } as const;
