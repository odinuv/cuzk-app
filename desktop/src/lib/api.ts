// Typed wrappers over the Tauri commands (src-tauri/src/ipc.rs, bodies in commands.rs).
//
// Serde naming: structs defined in the Tauri layer are camelCase on the wire and here.
// Core types passed through unchanged keep their own shape: `ItemIdent` (PascalCase
// variant tag, snake_case fields), `FieldChange`, `RizeniDef` (camelCase) and `KuOption`.
// Enums use their default Rust representation: `ItemStatus` is PascalCase ("Changed"),
// run statuses are lowercase strings. Dates are RFC 3339 strings.
// Command arguments are camelCase. Every command rejects with a Czech message (string).

import { invoke } from "@tauri-apps/api/core";

// ---- core types passed as-is ----

export type DruhCislovani = "Stavebni" | "Pozemkova";
export type TypStavby = "Cp" | "Ce";

export interface ParcelaRef {
  druh: DruhCislovani;
  kmenove: number;
  poddeleni: number | null;
}

export type ItemIdent =
  | { Parcela: { ku: number; parcela: ParcelaRef } }
  | {
      Stavba: {
        ku: number;
        typ: TypStavby;
        cislo_domovni: number;
        na_parcele: ParcelaRef;
      };
    }
  | {
      Jednotka: {
        ku: number;
        typ: TypStavby;
        cislo_domovni: number;
        cislo_jednotky: number;
        kod_casti_obce: number | null;
      };
    };

export interface FieldChange {
  path: string;
  before: unknown | null;
  after: unknown | null;
}

/** Only `id` is guaranteed; the display parts may be missing in the API data. */
export interface RizeniDef {
  id: number;
  typRizeni: string | null;
  poradoveCislo: number | null;
  rok: number | null;
  kodPracoviste: number | null;
}

export interface KuOption {
  kod: number;
  nazev: string;
  obec: string;
}

export type ItemStatus = "First" | "Unchanged" | "Changed" | "Error" | "Deferred";
export type RunStatus = "ok" | "partial" | "failed";

// ---- Tauri-layer view types (camelCase) ----

export interface ApiStatus {
  hasKey: boolean;
  limit: number | null;
  used: number | null;
  expires: string | null;
  /** Set when a key is stored but the account call failed. */
  error: string | null;
}

export type ItemKind = "parcela" | "stavba" | "jednotka";

export interface ItemView {
  id: number;
  popis: string;
  enabled: boolean;
  kind: ItemKind;
  describe: string;
  ident: ItemIdent;
  resolveError: string | null;
  lastSuccessAt: string | null;
}

export interface ItemInput {
  id?: number;
  popis: string;
  enabled: boolean;
  ident: ItemIdent;
}

export interface CastObceOption {
  kod: number;
  nazev: string;
}

export interface RunSummary {
  runId: number;
  status: RunStatus;
  checked: number;
  changed: number;
  plombyNove: number;
  errors: number;
  deferred: number;
  apiCalls: number;
  apiUsedToday: number;
  apiLimit: number;
  aktualnostDat: string | null;
  error: string | null;
  /** When the next item becomes due; null without checked, enabled items. */
  nextDue: string | null;
}

export interface PlombaView {
  rizeni: RizeniDef;
  datumPrijeti: string | null;
  stav: string | null;
  operace: string[];
}

export interface Changes {
  fields: FieldChange[];
  plombyNove: PlombaView[];
  plombyZanikle: RizeniDef[];
}

export interface FindingView {
  resultId: number;
  runId: number;
  itemId: number;
  popis: string;
  describe: string;
  status: ItemStatus;
  changes: Changes | null;
  /** Czech error text of a failed item. */
  error: string | null;
  runFinishedAt: string | null;
}

export interface RunView {
  id: number;
  startedAt: string;
  finishedAt: string | null;
  status: RunStatus | "running";
  trigger: "manual" | "scheduled";
  checked: number;
  changed: number;
  errors: number;
  deferred: number;
  apiCalls: number;
  apiUsedToday: number | null;
  apiLimit: number | null;
  aktualnostDat: string | null;
  error: string | null;
}

export interface ResultsView {
  /** Unseen findings. */
  findings: FindingView[];
  /** Per-item results of `lastRun` (all statuses). */
  lastRunItems: FindingView[];
  /** Newest run with a result (items checked, failed or deferred, or the run failed). */
  lastRun: RunView | null;
  /** History without the runs that had nothing due. */
  runs: RunView[];
  /** Runs with nothing due since `lastRun`. */
  emptyRuns: number;
}

export interface DueOverview {
  dueNow: number;
  waiting: number;
  remainingCalls: number | null;
}

export interface Settings {
  intervalDays: 7 | 14 | 30;
  scheduleEnabled: boolean;
  /** "HH:MM" */
  scheduleTime: string;
}

/** `get_settings` result: the editable settings plus the schedule heartbeat. */
export interface SettingsView extends Settings {
  /** Start of the newest scheduled (`--check`) run. */
  lastScheduledRun: string | null;
  /** The schedule is on but no scheduled run happened in the last two days. */
  scheduleStale: boolean;
  /** Whether the Windows task exists right now; null when unknown. */
  scheduleTaskPresent: boolean | null;
}

export interface ItemDetail {
  item: ItemView;
  /** Last stored API snapshot; null before the first successful check. */
  lastData: Record<string, unknown> | null;
}

// ---- commands ----

export const apiStatus = () => invoke<ApiStatus>("api_status");
export const setApiKey = (key: string) => invoke<ApiStatus>("set_api_key", { key });
export const deleteApiKey = () => invoke<void>("delete_api_key");

export const listItems = () => invoke<ItemView[]>("list_items");
export const saveItem = (input: ItemInput) => invoke<ItemView>("save_item", { input });
export const deleteItem = (id: number) => invoke<void>("delete_item", { id });
export const searchKu = (q: string) => invoke<KuOption[]>("search_ku", { q });
export const castiObciForKu = (ku: number) =>
  invoke<CastObceOption[]>("casti_obci_for_ku", { ku });

export const runCheck = () => invoke<RunSummary>("run_check");
export const results = () => invoke<ResultsView>("results");
/** Marks exactly these findings (the ones displayed) as seen. */
export const markSeen = (resultIds: number[]) => invoke<void>("mark_seen", { resultIds });
export const itemDetail = (id: number) => invoke<ItemDetail>("item_detail", { id });
export const dueOverview = () => invoke<DueOverview>("due_overview");

export const getSettings = () => invoke<SettingsView>("get_settings");
export const saveSettings = (settings: Settings) => invoke<void>("save_settings", { settings });
