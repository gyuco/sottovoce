import { ChevronDown, Loader2, MessageCircleMore, Send } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { useAudioStore } from "@/stores/audio";
import { useChatStore } from "@/stores/chat";

/**
 * Chat compatta con il modello mentre la call è in corso. Il contesto
 * (meeting, trascrizione, codice locale) lo aggiunge il backend: qui si
 * vedono solo domande e risposte.
 */
export function MeetingChat() {
  const { t } = useTranslation();
  const running = useAudioStore((s) => s.status === "running");
  const lines = useChatStore((s) => s.lines);
  const sending = useChatStore((s) => s.sending);
  const error = useChatStore((s) => s.error);
  const ask = useChatStore((s) => s.ask);
  const [openPanel, setOpenPanel] = useState(false);
  const [text, setText] = useState("");
  const list = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = list.current;
    if (openPanel && el) el.scrollTop = el.scrollHeight;
  }, [openPanel, lines, sending]);

  // Solo durante la call: senza trascrizione la chat non ha contesto.
  if (!running) return null;

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const question = text.trim();
    if (question === "" || sending) return;
    setText("");
    void ask(question);
  };

  return (
    <section className="rounded-2xl border bg-card shadow-xs">
      <button
        type="button"
        className="flex w-full items-center gap-2 rounded-2xl px-4 py-3 text-left text-sm outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
        aria-expanded={openPanel}
        aria-controls="meeting-chat"
        onClick={() => setOpenPanel((o) => !o)}
      >
        <MessageCircleMore className="size-4 shrink-0 text-primary" aria-hidden />
        <span className="font-semibold">{t("chat.title")}</span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground">{t("chat.subtitle")}</span>
        <ChevronDown
          className={cn(
            "size-4 shrink-0 text-muted-foreground transition-transform",
            openPanel && "rotate-180",
          )}
          aria-hidden
        />
      </button>

      {openPanel && (
        <div id="meeting-chat" className="flex flex-col gap-2 border-t px-3 pt-2 pb-3">
          <div
            ref={list}
            className="flex max-h-52 min-h-14 flex-col gap-2 overflow-y-auto overscroll-contain"
            aria-live="polite"
          >
            {lines.length === 0 ? (
              <p className="m-auto max-w-72 py-3 text-center text-xs text-muted-foreground">
                {t("chat.empty")}
              </p>
            ) : (
              lines.map((line) => (
                <div
                  key={line.id}
                  className={cn(
                    "max-w-[88%] rounded-xl px-3 py-2 text-sm leading-relaxed whitespace-pre-wrap select-text",
                    line.role === "user"
                      ? "self-end bg-primary/12"
                      : "self-start border bg-muted/40",
                  )}
                >
                  {line.text}
                </div>
              ))
            )}
            {sending && (
              <p className="flex items-center gap-1.5 self-start px-1 text-xs text-muted-foreground">
                <Loader2 className="size-3.5 animate-spin" aria-hidden />
                {t("chat.thinking")}
              </p>
            )}
          </div>

          {error && (
            <p role="alert" className="px-1 text-xs text-destructive">
              {error}
            </p>
          )}

          <form onSubmit={submit} className="flex gap-2">
            <Input
              aria-label={t("chat.placeholder")}
              placeholder={t("chat.placeholder")}
              maxLength={2000}
              value={text}
              onChange={(e) => setText(e.target.value)}
              className="min-w-0"
            />
            <Button
              type="submit"
              size="icon"
              aria-label={t("chat.send")}
              title={t("chat.send")}
              disabled={sending || text.trim() === ""}
            >
              <Send aria-hidden />
            </Button>
          </form>
        </div>
      )}
    </section>
  );
}
