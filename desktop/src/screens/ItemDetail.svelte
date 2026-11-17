<script lang="ts">
  import { itemDetail, type ItemDetail } from "../lib/api";
  import { detailRows, formatDate } from "../lib/format";

  let { id, onclose }: { id: number; onclose: () => void } = $props();

  let data = $state<ItemDetail | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const wanted = id;
    data = null;
    error = null;
    itemDetail(wanted).then(
      (d) => {
        if (wanted === id) data = d;
      },
      (err) => {
        if (wanted === id) error = String(err);
      },
    );
  });

  const rows = $derived(data?.lastData ? detailRows(data.lastData) : []);
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="backdrop" role="presentation" onclick={onclose}>
  <div
    class="dialog card"
    role="dialog"
    aria-modal="true"
    aria-label="Detail nemovitosti"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={() => {}}
  >
    <div class="row head">
      <h2>{data ? data.item.popis : "Detail nemovitosti"}</h2>
      <button onclick={onclose}>Zavřít</button>
    </div>
    {#if error}
      <p class="error">{error}</p>
    {:else if !data}
      <p class="muted">Načítám…</p>
    {:else}
      <p class="muted">
        {data.item.describe}. Zobrazují se poslední uložená data z katastru (poslední úspěšná kontrola:
        {formatDate(data.item.lastSuccessAt)}); detail nestojí žádné volání API.
      </p>
      {#if rows.length === 0}
        <p class="muted">Nemovitost zatím nebyla úspěšně zkontrolována, data nejsou k dispozici.</p>
      {:else}
        <dl>
          {#each rows as r, i (i)}
            <div class="kv" style="padding-left: {r.depth * 1.2}em">
              <dt class:group={r.value === null}>{r.key}</dt>
              {#if r.value !== null}<dd>{r.value}</dd>{/if}
            </div>
          {/each}
        </dl>
      {/if}
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed; inset: 0; background: rgb(0 0 0 / 35%); display: flex;
    align-items: flex-start; justify-content: center; padding: 2em 1em; z-index: 50; overflow: auto;
  }
  .dialog { width: 100%; max-width: 760px; margin: 0; }
  .head { justify-content: space-between; }
  .head h2 { margin: 0; }
  dl { margin: 0; font-size: 0.9rem; }
  .kv { display: flex; gap: 0.75em; padding: 0.15em 0; border-bottom: 1px solid var(--border); }
  dt { color: var(--muted); min-width: 10em; }
  dt.group { font-weight: 600; color: var(--text); }
  dd { margin: 0; user-select: text; word-break: break-word; }
</style>
