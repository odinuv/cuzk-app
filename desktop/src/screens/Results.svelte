<script lang="ts">
  import { markSeen, type FindingView, type ResultsView } from "../lib/api";
  import { formatDate, formatDateTime } from "../lib/format";
  import FindingCard from "./FindingCard.svelte";

  let { data, onchanged, ondetail }: {
    data: ResultsView | null;
    onchanged: () => void;
    ondetail: (itemId: number) => void;
  } = $props();

  let error = $state<string | null>(null);

  const hasPlomba = (f: FindingView) => (f.changes?.plombyNove.length ?? 0) > 0;
  const rank = (f: FindingView) =>
    hasPlomba(f) ? 0 : f.status === "Changed" ? 1 : f.status === "Error" ? 2 : f.status === "First" ? 3 : 4;
  const byRank = (list: FindingView[]) => [...list].sort((a, b) => rank(a) - rank(b));

  // Unseen findings: plomby first, then changes (sort is stable within a rank).
  const findings = $derived(byRank(data?.findings ?? []));
  // Last run: noteworthy results (first checks, errors, deferred) not already listed above.
  const lastItems = $derived.by(() => {
    const shown = new Set((data?.findings ?? []).map((f) => f.resultId));
    return (data?.lastRunItems ?? []).filter((f) => !shown.has(f.resultId));
  });
  const noteworthy = $derived(byRank(lastItems.filter((f) => f.status !== "Unchanged")));
  const unchangedCount = $derived(lastItems.filter((f) => f.status === "Unchanged").length);

  const runStatus = (s: string) =>
    s === "ok" ? "OK" : s === "partial" ? "částečně" : s === "running" ? "běží" : "selhala";

  async function seen() {
    error = null;
    try {
      await markSeen();
      onchanged();
    } catch (err) {
      error = String(err);
    }
  }
</script>

{#if !data}
  <p class="muted">Načítám…</p>
{:else}
  <div class="row head">
    <h2>Nové nálezy</h2>
    {#if findings.length > 0}<button onclick={seen}>Označit jako přečtené</button>{/if}
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  {#if findings.length === 0}
    <div class="card muted">Žádné nové nálezy od posledního zobrazení.</div>
  {/if}

  {#each findings as f (f.resultId)}
    <FindingCard {f} {ondetail} />
  {/each}

  <h2>Poslední kontrola</h2>
  {#if data.lastRun}
    {@const r = data.lastRun}
    <div class="card">
      {formatDateTime(r.finishedAt ?? r.startedAt)} · zkontrolováno {r.checked}
      · změn {r.changed} · chyb {r.errors}{#if r.deferred} · odloženo {r.deferred}{/if}
      <div class="muted small">
        Volání API: {r.apiCalls}{#if r.apiUsedToday !== null && r.apiLimit !== null} (dnes {r.apiUsedToday}/{r.apiLimit}){/if}
        {#if r.aktualnostDat} · aktuálnost dat KN: {formatDate(r.aktualnostDat)}{/if}
      </div>
      {#if r.error}<div class="error">{r.error}</div>{/if}
    </div>
    {#each noteworthy as f (f.resultId)}
      <FindingCard {f} {ondetail} />
    {/each}
    {#if unchangedCount > 0}<p class="muted">{unchangedCount} dalších beze změn.</p>{/if}
  {:else}
    <div class="card muted">Kontrola zatím neproběhla. Použijte tlačítko „Zkontrolovat“.</div>
  {/if}

  {#if data.runs.length > 0}
    <h2>Historie kontrol</h2>
    <div class="card">
      <ul class="history">
        {#each data.runs as r (r.id)}
          <li>
            <span>{formatDateTime(r.startedAt)}</span>
            <span class="muted">{r.trigger === "scheduled" ? "plánovaná" : "ruční"}</span>
            <span>zkontrolováno {r.checked}, změn {r.changed}, chyb {r.errors}</span>
            <span class:error={r.status === "failed"}>{runStatus(r.status)}</span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
{/if}

<style>
  .head { justify-content: space-between; margin-bottom: 0.75em; }
  .head h2 { margin: 0; }
  .small { font-size: 0.85rem; }
  .history { list-style: none; padding: 0; margin: 0; }
  .history li { display: flex; gap: 1em; flex-wrap: wrap; padding: 0.3em 0; border-bottom: 1px solid var(--border); }
  .history li:last-child { border-bottom: none; }
</style>
