import { Check, Fingerprint, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { speakerColor } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { useMeetingStore } from "@/stores/meeting";

/**
 * Esiti dei profili vocali durante la call: parlanti che somigliano a un
 * profilo ("Sembra Marco?") e nomi assegnati dal profilo ("non è lui").
 */
export function VoiceNotices() {
  const { t } = useTranslation();
  const suggestions = useMeetingStore((s) => s.voiceSuggestions);
  const recognized = useMeetingStore((s) => s.voiceRecognized);
  const accept = useMeetingStore((s) => s.acceptVoice);
  const reject = useMeetingStore((s) => s.rejectVoice);
  const keep = useMeetingStore((s) => s.keepVoice);

  const items = [
    ...Object.entries(suggestions).map(([k, name]) => ({ speaker: Number(k), name, suggested: true })),
    ...Object.entries(recognized).map(([k, name]) => ({ speaker: Number(k), name, suggested: false })),
  ].sort((a, b) => a.speaker - b.speaker);
  if (items.length === 0) return null;

  return (
    <ul className="flex flex-col gap-1.5 border-t px-4 py-2" aria-label={t("speakers.voice.region")}>
      {items.map(({ speaker, name, suggested }) => {
        const label = t("speakers.label", { n: speaker + 1 });
        return (
          <li key={`${speaker}-${suggested}`} className="flex flex-wrap items-center gap-2 text-xs">
            <Fingerprint className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            <span className={cn("rounded-md px-1.5 py-px font-medium", speakerColor("them", speaker))}>{label}</span>
            <span className="text-muted-foreground">
              {suggested ? t("speakers.voice.suggest", { name }) : t("speakers.voice.recognized", { name })}
            </span>
            <span className="ml-auto flex gap-1">
              {suggested ? (
                <>
                  <Button size="xs" variant="outline" onClick={() => accept(speaker)}>
                    <Check /> {t("speakers.voice.accept", { name })}
                  </Button>
                  <Button size="xs" variant="ghost" onClick={() => reject(speaker)}>
                    <X /> {t("speakers.voice.dismiss")}
                  </Button>
                </>
              ) : (
                <>
                  <Button size="xs" variant="ghost" onClick={() => reject(speaker)}>
                    <X /> {t("speakers.voice.notThem", { name })}
                  </Button>
                  <Button size="xs" variant="outline" onClick={() => keep(speaker)}>
                    <Check /> {t("speakers.voice.ok")}
                  </Button>
                </>
              )}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
