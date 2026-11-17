<script lang="ts">
  import { markSeen, type FindingView, type ResultsView } from "../lib/api";
  import { formatChange, formatDate, formatDateTime, formatRizeni, statusLabel } from "../lib/format";

  let { data, onchanged }: { data: ResultsView | null; onchanged: () => void } = $props();

  let error = $state<string | null>(null);

  const hasPlomba = (f: FindingView) => (f.changes?.plombyNove.length ?? 0) > 0;
  const rank = (f: FindingView) =>
    hasPlomba(f) ? 0 : f.status === "Changed" ? 1 : f.status === "Error" ? 2 : f.status === "First" ? 3 : 4;
  // Unseen findings: plomby first, then changes, then the rest (sort is stable within a rank).
  const findings = $derived([...(data?.findings ?? [])].sort((a, b) => rank(a) - rank(b)));

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
    {#if hasPlomba(f)}
      <div class="card plomba">
        <div class="badge">▲ PLOMBA</div>
        <strong>{f.popis}</strong> <span>{f.describe}</span>
        <ul>
          {#each f.changes!.plombyNove as p (p.rizeni.id)}
            <li>
              Řízení <strong>{formatRizeni(p.rizeni)}</strong>
              {#if p.datumPrijeti}, přijato {formatDate(p.datumPrijeti)}{/if}
              {#if p.stav}, stav: {p.stav}{/if}
              {#if p.operace.length}<div class="small">Operace: {p.operace.join(", ")}</div>{/if}
            </li>
          {/each}
        </ul>
        {#if f.changes!.fields.length}
          <ul>{#each f.changes!.fields as c, i (i)}<li>{formatChange(c)}</li>{/each}</ul>
        {/if}
      </div>
    {:else}
      <div class="card" class:err={f.status === "Error"}>
        <span class="tag {f.status}">{statusLabel(f.status)}</span>
        <strong>{f.popis}</strong> <span class="muted">{f.describe}</span>
        {#if f.changes && (f.changes.fields.length > 0 || f.changes.plombyZanikle.length > 0)}
          <ul>
            {#each f.changes.fields as c, i (i)}<li>{formatChange(c)}</li>{/each}
            {#each f.changes.plombyZanikle as r (r.id)}<li>Zaniklo řízení {formatRizeni(r)}</li>{/each}
          </ul>
        {/if}
        <div class="muted small">{formatDateTime(f.runFinishedAt)}</div>
      </div>
    {/if}
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
  .plomba { background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); border-width: 2px; }
  .badge { font-weight: 700; letter-spacing: 0.05em; margin-bottom: 0.3em; }
  .tag { display: inline-block; padding: 0.1em 0.6em; border-radius: 99px; font-size: 0.8rem; background: var(--bg); border: 1px solid var(--border); margin-right: 0.5em; }
  .tag.Changed { background: #e3eefb; border-color: var(--accent); }
  .tag.Error { background: #fde8e6; border-color: var(--danger); color: var(--danger); }
  .card.err { border-color: var(--danger); }
  ul { margin: 0.4em 0 0; padding-left: 1.2em; }
  .small { font-size: 0.85rem; }
  .history { list-style: none; padding: 0; margin: 0; }
  .history li { display: flex; gap: 1em; flex-wrap: wrap; padding: 0.3em 0; border-bottom: 1px solid var(--border); }
  .history li:last-child { border-bottom: none; }
</style>
