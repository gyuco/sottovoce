import { useState } from "react";
import { useTranslation } from "react-i18next";

import { SpeakerCard } from "@/components/SpeakerCard";
import type { ChannelId } from "@/lib/ipc";
import { speakerColor, useSpeakerName } from "@/lib/speakers";
import { cn } from "@/lib/utils";
import { useMeetingStore } from "@/stores/meeting";

/** Etichetta del parlante; per "Loro" con parlante noto apre la scheda (nome, ruolo, email…). */
export function SpeakerLabel(props: { channel: ChannelId; speaker: number | null; editable?: boolean }) {
  const { t } = useTranslation();
  const name = useSpeakerName();
  const rename = useMeetingStore((s) => s.renameSpeaker);
  const given = useMeetingStore((s) =>
    props.speaker === null ? undefined : s.context.speakers[props.speaker],
  );
  const participants = useMeetingStore((s) => s.context.participants);
  const speakers = useMeetingStore((s) => s.context.speakers);
  const [editing, setEditing] = useState(false);
  const { channel, speaker } = props;
  const label = name(channel, speaker);
  const color = speakerColor(channel, speaker);
  const base = "justify-self-start truncate rounded-md px-1.5 py-px text-xs font-medium max-w-full";

  if (channel === "me" || speaker === null || !props.editable) {
    return <span className={cn(base, color)}>{label}</span>;
  }
  return (
    <>
      <button
        type="button"
        className={cn(base, color, "cursor-pointer hover:ring-1 hover:ring-current/40")}
        title={t("speakers.renameHint")}
        aria-label={t("speakers.rename", { name: label })}
        onClick={() => setEditing(true)}
      >
        {label}
      </button>
      {editing && (
        <SpeakerCard
          open
          onOpenChange={setEditing}
          name={given ?? ""}
          fallback={t("speakers.label", { n: speaker + 1 })}
          onSave={(value) => rename(speaker, value)}
          participants={participants.map((p) => p.name)}
          taken={Object.entries(speakers)
            .filter(([k]) => Number(k) !== speaker)
            .map(([, v]) => v)}
        />
      )}
    </>
  );
}
