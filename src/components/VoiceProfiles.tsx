import { Check, Fingerprint, Pencil, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

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
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type { VoiceProfile } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { MEETING_LIMITS } from "@/stores/meeting";
import { useSettingsStore } from "@/stores/settings";
import { useVoicesStore } from "@/stores/voices";

/** Profili vocali (opt-in): interruttore con spiegazione, elenco, rinomina, elimina. */
export function VoiceProfiles() {
  const { t, i18n } = useTranslation();
  const settings = useSettingsStore((s) => s.settings);
  const update = useSettingsStore((s) => s.update);
  const { profiles, error, load, rename, remove, removeAll } = useVoicesStore();
  const [confirm, setConfirm] = useState<"off" | "all" | null>(null);

  // Ricarica anche quando cambia l'interruttore (e dopo ogni sessione riaprendo).
  const enabled = settings?.voiceProfiles;
  useEffect(() => {
    void load();
  }, [load, enabled]);

  if (!settings) return null;

  const toggle = (on: boolean) => {
    // Spegnere con profili salvati: si chiede se eliminarli.
    if (!on && profiles.length > 0) setConfirm("off");
    else void update({ voiceProfiles: on });
  };

  const date = new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium" });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Fingerprint className="size-4 text-muted-foreground" aria-hidden />
          {t("settings.voices.title")}
        </CardTitle>
        <CardDescription>{t("settings.voices.hint")}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <label className="flex items-center justify-between gap-4 text-sm">
          <span>
            {t("settings.voices.toggle")}
            <span className="block text-xs text-muted-foreground">{t("settings.voices.toggleHint")}</span>
          </span>
          <Switch checked={settings.voiceProfiles} onCheckedChange={toggle} />
        </label>
        {settings.voiceProfiles && !settings.diarization && (
          <p className="text-xs text-amber-700 dark:text-amber-400">{t("settings.voices.needsDiarization")}</p>
        )}

        {profiles.length > 0 ? (
          <>
            <ul className="flex flex-col divide-y rounded-md border">
              {profiles.map((p) => (
                <ProfileRow
                  key={p.id}
                  profile={p}
                  lastSeen={date.format(new Date(p.lastSeenMs))}
                  onRename={(name) => rename(p.id, name)}
                  onDelete={() => remove(p.id)}
                />
              ))}
            </ul>
            <Button size="xs" variant="outline" className="self-end" onClick={() => setConfirm("all")}>
              <Trash2 /> {t("settings.voices.deleteAll")}
            </Button>
          </>
        ) : (
          settings.voiceProfiles && <p className="text-xs text-muted-foreground">{t("settings.voices.empty")}</p>
        )}

        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </CardContent>

      <AlertDialog open={confirm !== null} onOpenChange={(open) => !open && setConfirm(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirm === "off" ? t("settings.voices.offTitle") : t("settings.voices.deleteAllTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm === "off" ? t("settings.voices.offConfirm") : t("settings.voices.deleteAllConfirm")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            {confirm === "off" ? (
              <>
                <AlertDialogCancel onClick={() => void update({ voiceProfiles: false })}>
                  {t("settings.voices.offKeep")}
                </AlertDialogCancel>
                <AlertDialogAction
                  className={cn(buttonVariants({ variant: "destructive" }))}
                  onClick={() => void update({ voiceProfiles: false }).then(removeAll)}
                >
                  {t("settings.voices.offDelete")}
                </AlertDialogAction>
              </>
            ) : (
              <>
                <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
                <AlertDialogAction
                  className={cn(buttonVariants({ variant: "destructive" }))}
                  onClick={() => void removeAll()}
                >
                  {t("settings.voices.deleteAll")}
                </AlertDialogAction>
              </>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}

function ProfileRow(props: {
  profile: VoiceProfile;
  lastSeen: string;
  onRename: (name: string) => Promise<void>;
  onDelete: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const { profile } = props;
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(profile.name);

  const save = async () => {
    const trimmed = name.trim();
    if (trimmed && trimmed !== profile.name) await props.onRename(trimmed);
    setEditing(false);
  };

  return (
    <li className="flex items-center gap-3 px-3 py-2 text-sm">
      {editing ? (
        <Input
          autoFocus
          value={name}
          maxLength={MEETING_LIMITS.speakerName}
          aria-label={t("settings.voices.nameLabel")}
          className="h-7 flex-1"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
            if (e.key === "Escape") {
              setName(profile.name);
              setEditing(false);
            }
          }}
        />
      ) : (
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{profile.name}</span>
          <span className="block text-xs text-muted-foreground">
            {t("settings.voices.meta", { date: props.lastSeen, meetings: profile.meetings })}
          </span>
        </span>
      )}
      {editing ? (
        <>
          <Button size="icon-xs" variant="ghost" aria-label={t("common.save")} onClick={() => void save()}>
            <Check />
          </Button>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label={t("common.cancel")}
            onClick={() => {
              setName(profile.name);
              setEditing(false);
            }}
          >
            <X />
          </Button>
        </>
      ) : (
        <>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label={t("settings.voices.rename", { name: profile.name })}
            onClick={() => setEditing(true)}
          >
            <Pencil />
          </Button>
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label={t("settings.voices.delete", { name: profile.name })}
            onClick={() => void props.onDelete()}
          >
            <Trash2 />
          </Button>
        </>
      )}
    </li>
  );
}
