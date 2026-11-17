import type { DueOverview, FieldChange, ItemStatus, RizeniDef, RunSummary } from "./api";

const DASH = "—";

/** Human-readable value; missing/empty values become "—". */
export function formatValue(v: unknown): string {
  if (v === null || v === undefined || v === "") return DASH;
  if (typeof v === "boolean") return v ? "ano" : "ne";
  if (Array.isArray(v)) return v.length === 0 ? DASH : JSON.stringify(v);
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

export function formatChange(c: FieldChange): string {
  return `${c.path}: ${formatValue(c.before)} → ${formatValue(c.after)}`;
}

/** "V-1234/2026-701" */
export function formatRizeni(r: RizeniDef): string {
  return `${r.typRizeni}-${r.poradoveCislo}/${r.rok}-${r.kodPracoviste}`;
}

const STATUS_LABELS: Record<ItemStatus, string> = {
  First: "první kontrola",
  Unchanged: "beze změn",
  Changed: "změna",
  Error: "chyba",
  Deferred: "čeká na další den",
};

export function statusLabel(s: ItemStatus): string {
  return STATUS_LABELS[s];
}

const dateFmt = new Intl.DateTimeFormat("cs-CZ", {
  day: "numeric",
  month: "numeric",
  year: "numeric",
});

/** "17. 11. 2026"; "—" for missing/invalid. */
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return DASH;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return DASH;
  return dateFmt.format(d);
}

const dateTimeFmt = new Intl.DateTimeFormat("cs-CZ", {
  day: "numeric",
  month: "numeric",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return DASH;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? DASH : dateTimeFmt.format(d);
}

/** "Na řadě: 5 · čeká: 0 · zbývá volání dnes: 412" */
export function formatDue(d: DueOverview): string {
  const calls = d.remainingCalls === null ? DASH : String(d.remainingCalls);
  return `Na řadě: ${d.dueNow} · čeká: ${d.waiting} · zbývá volání dnes: ${calls}`;
}

/** One-line summary of a finished check. */
export function formatRunSummary(s: RunSummary): string {
  if (s.status === "failed") return s.error ?? "Kontrola selhala.";
  const parts = [`zkontrolováno ${s.checked}`];
  if (s.changed > 0) parts.push(`změn ${s.changed}`);
  if (s.plombyNove > 0) parts.push(`nových plomb ${s.plombyNove}`);
  if (s.errors > 0) parts.push(`chyb ${s.errors}`);
  if (s.deferred > 0) parts.push(`odloženo na další den ${s.deferred}`);
  return parts.join(" · ");
}
