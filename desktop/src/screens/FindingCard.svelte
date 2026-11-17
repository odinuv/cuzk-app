<script lang="ts">
  import type { FindingView } from "../lib/api";
  import { formatChange, formatDate, formatDateTime, formatRizeni, statusLabel } from "../lib/format";

  let { f, ondetail }: { f: FindingView; ondetail: (itemId: number) => void } = $props();

  const nove = $derived(f.changes?.plombyNove ?? []);
  const zanikle = $derived(f.changes?.plombyZanikle ?? []);
  const fields = $derived(f.changes?.fields ?? []);
  const plomba = $derived(nove.length > 0);
</script>

<div class="card" class:plomba class:err={f.status === "Error"}>
  {#if plomba}<div class="badge">▲ PLOMBA</div>{/if}
  <span class="tag {f.status}">{statusLabel(f.status)}</span>
  <strong>{f.popis}</strong> <span class:muted={!plomba}>{f.describe}</span>
  {#if f.error}<div class="error">{f.error}</div>{/if}
  {#if plomba && f.status === "First"}
    <div class="small">Plomba byla na nemovitosti už při první kontrole.</div>
  {/if}
  {#if nove.length > 0}
    <ul>
      {#each nove as p (p.rizeni.id)}
        <li>
          Nové řízení <strong>{formatRizeni(p.rizeni)}</strong>
          {#if p.datumPrijeti}, přijato {formatDate(p.datumPrijeti)}{/if}
          {#if p.stav}, stav: {p.stav}{/if}
          {#if p.operace.length}<div class="small">Operace: {p.operace.join(", ")}</div>{/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if zanikle.length > 0}
    <ul>{#each zanikle as r (r.id)}<li>Zaniklo řízení {formatRizeni(r)}</li>{/each}</ul>
  {/if}
  {#if fields.length > 0}
    <ul>{#each fields as c, i (i)}<li>{formatChange(c)}</li>{/each}</ul>
  {/if}
  <div class="row foot">
    <span class="small muted">{formatDateTime(f.runFinishedAt)}</span>
    <button class="link" onclick={() => ondetail(f.itemId)}>Detail</button>
  </div>
</div>

<style>
  .plomba { background: var(--warn-bg); border-color: var(--warn-border); color: var(--warn-text); border-width: 2px; }
  .badge { font-weight: 700; letter-spacing: 0.05em; margin-bottom: 0.3em; }
  .tag { display: inline-block; padding: 0.1em 0.6em; border-radius: 99px; font-size: 0.8rem; background: var(--bg); border: 1px solid var(--border); margin-right: 0.5em; color: var(--text); }
  .tag.Changed { background: #e3eefb; border-color: var(--accent); }
  .tag.Error { background: #fde8e6; border-color: var(--danger); color: var(--danger); }
  .card.err { border-color: var(--danger); }
  ul { margin: 0.4em 0 0; padding-left: 1.2em; }
  .small { font-size: 0.85rem; }
  .foot { justify-content: space-between; margin-top: 0.3em; }
</style>
