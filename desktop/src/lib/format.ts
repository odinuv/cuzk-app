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

/** History line for runs that found nothing due: "3 prázdné běhy (nic nebylo na řadě)". */
export function formatEmptyRuns(n: number): string {
  const word = n === 1 ? "prázdný běh" : n >= 2 && n <= 4 ? "prázdné běhy" : "prázdných běhů";
  return `${n} ${word} (nic nebylo na řadě)`;
}

/** "Na řadě: 5 · čeká: 0 · zbývá volání dnes: 412" */
export function formatDue(d: DueOverview): string {
  const calls = d.remainingCalls === null ? DASH : String(d.remainingCalls);
  return `Na řadě: ${d.dueNow} · čeká: ${d.waiting} · zbývá volání dnes: ${calls}`;
}

/** One-line summary of a finished check. */
export function formatRunSummary(s: RunSummary): string {
  if (s.status === "failed") return s.error ?? "Kontrola selhala.";
  if (s.checked + s.errors + s.deferred === 0) {
    return s.nextDue
      ? `Nic není na řadě – nejbližší kontrola ${formatDate(s.nextDue)}`
      : "Nic není na řadě – žádná zapnutá nemovitost ke kontrole";
  }
  const parts = [`zkontrolováno ${s.checked}`];
  if (s.changed > 0) parts.push(`změn ${s.changed}`);
  if (s.plombyNove > 0) parts.push(`nových plomb ${s.plombyNove}`);
  if (s.errors > 0) parts.push(`chyb ${s.errors}`);
  if (s.deferred > 0) parts.push(`odloženo na další den ${s.deferred}`);
  return parts.join(" · ");
}

export interface DetailRow {
  depth: number;
  key: string;
  /** null for a container (object/array) header row. */
  value: string | null;
}

/** Flattens JSON into indented key/value rows for a readable nested view. */
export function detailRows(v: unknown, depth = 0): DetailRow[] {
  const rows: DetailRow[] = [];
  const entries: [string, unknown][] = Array.isArray(v)
    ? v.map((x, i) => [`[${i + 1}]`, x])
    : v !== null && typeof v === "object"
      ? Object.entries(v as Record<string, unknown>)
      : [];
  for (const [key, val] of entries) {
    const isContainer = val !== null && typeof val === "object" && Object.keys(val).length > 0;
    if (isContainer) {
      rows.push({ depth, key, value: null });
      rows.push(...detailRows(val, depth + 1));
    } else {
      rows.push({ depth, key, value: formatValue(val) });
    }
  }
  return rows;
}
