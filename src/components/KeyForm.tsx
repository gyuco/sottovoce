import { Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { KeyProvider } from "@/lib/ipc";
import { useSettingsStore } from "@/stores/settings";

/** Salva/sostituisce/elimina una API key nel Keychain. Il valore non torna mai indietro. */
export function KeyForm({ provider, label }: { provider: KeyProvider; label: string }) {
  const { t } = useTranslation();
  const hasKey = useSettingsStore((s) => s.keys[provider] ?? false);
  const saveKey = useSettingsStore((s) => s.saveKey);
  const deleteKey = useSettingsStore((s) => s.deleteKey);
  const [key, setKey] = useState("");
  const [saving, setSaving] = useState(false);

  const onSave = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    if (await saveKey(provider, key)) setKey("");
    setSaving(false);
  };

  return (
    <form onSubmit={(e) => void onSave(e)} className="flex gap-2">
      <Input
        type="password"
        autoComplete="off"
        spellCheck={false}
        placeholder={hasKey ? t("settings.key.replacePlaceholder") : t("settings.key.placeholder")}
        aria-label={label}
        value={key}
        onChange={(e) => setKey(e.target.value)}
      />
      <Button type="submit" disabled={saving || key.trim() === ""}>
        {t("settings.save")}
      </Button>
      {hasKey && (
        <Button
          type="button"
          variant="outline"
          size="icon"
          aria-label={t("settings.key.delete")}
          title={t("settings.key.delete")}
          onClick={() => void deleteKey(provider)}
        >
          <Trash2 />
        </Button>
      )}
    </form>
  );
}
