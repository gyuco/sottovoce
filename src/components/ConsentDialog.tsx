import { Check, Copy, Play } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { CONSENT_LANGUAGES, type ConsentLanguage, consentLanguage, consentMessage } from "@/lib/consent";
import { useCopy } from "@/lib/hooks";
import type { Language } from "@/lib/ipc";
import { isLocalSession } from "@/lib/privacy";
import { useConsentStore } from "@/stores/consent";
import { useSettingsStore } from "@/stores/settings";

/**
 * A ogni nuova call: messaggio da dire o incollare in chat per chiedere ai
 * partecipanti il consenso alla trascrizione. La call parte solo se l'utente
 * conferma che hanno accettato.
 */
export function ConsentDialog() {
  const pending = useConsentStore((s) => s.pending);
  if (!pending) return null;
  // `key`: ogni richiesta riparte dalla lingua della call.
  return <ConsentContent key={pending.id} language={pending.language} />;
}

function ConsentContent({ language }: { language: Language }) {
  const { t, i18n } = useTranslation();
  const answer = useConsentStore((s) => s.answer);
  const settings = useSettingsStore((s) => s.settings);
  const local = settings ? isLocalSession(settings) : false;
  const [lang, setLang] = useState<ConsentLanguage>(() => consentLanguage(language, i18n.language));
  const { copied, copy } = useCopy();
  const message = consentMessage(lang, local);

  return (
    <Dialog open onOpenChange={(open) => !open && answer(false)}>
      <DialogContent closeLabel={t("common.close")} className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("consent.title")}</DialogTitle>
          <DialogDescription>{t("consent.intro")}</DialogDescription>
        </DialogHeader>

        <div className="grid gap-2 rounded-xl border bg-muted/30 p-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium">{t("consent.message")}</span>
            <Select value={lang} onValueChange={(v) => setLang(v as ConsentLanguage)}>
              <SelectTrigger size="sm" className="w-auto bg-background" aria-label={t("consent.language")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CONSENT_LANGUAGES.map((l) => (
                  <SelectItem key={l} value={l}>
                    {t(`settings.language.${l}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <p lang={lang} className="text-sm leading-relaxed select-text">
            {message}
          </p>
          <div>
            <Button type="button" variant="outline" size="sm" className="bg-background" onClick={() => copy(message)}>
              {copied ? <Check /> : <Copy />}
              {copied ? t("common.copied") : t("consent.copy")}
            </Button>
          </div>
        </div>

        {!local && <p className="text-xs text-muted-foreground">{t("consent.cloudNote")}</p>}

        <DialogFooter>
          <Button type="button" variant="outline" onClick={() => answer(false)}>
            {t("common.cancel")}
          </Button>
          <Button type="button" onClick={() => answer(true)}>
            <Play className="fill-current" />
            {t("consent.confirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
