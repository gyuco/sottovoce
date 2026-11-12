import { Check, CircleAlert, Loader2 } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export type Status = "ok" | "pending" | "warn";

/** Opzione a scelta singola, grande e cliccabile. */
export function Choice(props: {
  selected: boolean;
  onSelect: () => void;
  children: ReactNode;
  className?: string;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.selected}
      onClick={props.onSelect}
      className={cn(
        "rounded-xl border bg-card px-3 py-2 text-left text-sm shadow-xs transition-colors outline-none hover:border-primary/50 focus-visible:ring-[3px] focus-visible:ring-ring/50",
        props.selected && "border-primary bg-primary/5 ring-1 ring-primary",
        props.className,
      )}
    >
      {props.children}
    </button>
  );
}

export function StatusIcon({ status }: { status: Status }) {
  if (status === "ok")
    return (
      <span className="flex size-5 shrink-0 items-center justify-center rounded-full bg-emerald-500/15 text-emerald-600 dark:text-emerald-400">
        <Check className="size-3.5" aria-hidden />
      </span>
    );
  if (status === "pending") return <Loader2 className="size-5 shrink-0 animate-spin text-primary" aria-hidden />;
  return <CircleAlert className="size-5 shrink-0 text-amber-500" aria-hidden />;
}
