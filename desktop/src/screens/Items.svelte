<script lang="ts">
  import { deleteItem, saveItem, type ItemView } from "../lib/api";
  import { formatDate } from "../lib/format";
  import ItemForm from "./ItemForm.svelte";

  let { items, onchanged }: { items: ItemView[]; onchanged: () => void } = $props();

  let editing = $state<ItemView | "new" | null>(null);
  let confirmId = $state<number | null>(null);
  let errors = $state<Record<number, string>>({});

  async function toggle(it: ItemView) {
    try {
      await saveItem({ id: it.id, popis: it.popis, enabled: !it.enabled, ident: it.ident });
      delete errors[it.id];
      onchanged();
    } catch (err) {
      errors[it.id] = String(err);
    }
  }

  async function remove(it: ItemView) {
    try {
      await deleteItem(it.id);
      confirmId = null;
      onchanged();
    } catch (err) {
      errors[it.id] = String(err);
    }
  }
</script>

<div class="row head">
  <h2>Nemovitosti</h2>
  {#if editing === null}<button class="primary" onclick={() => (editing = "new")}>Přidat nemovitost</button>{/if}
</div>

{#if editing !== null}
  {#key editing === "new" ? "new" : editing.id}
    <ItemForm item={editing === "new" ? null : editing} onsaved={onchanged} oncancel={() => (editing = null)} />
  {/key}
{/if}

{#if items.length === 0 && editing === null}
  <div class="card muted">Zatím tu nic není. Přidejte první parcelu, stavbu nebo jednotku.</div>
{/if}

{#each items as it (it.id)}
  <div class="card item" class:off={!it.enabled}>
    <div class="main">
      <strong>{it.popis}</strong>
      <div class="muted">{it.describe}</div>
      {#if it.resolveError}<div class="error">{it.resolveError}</div>{/if}
      {#if errors[it.id]}<div class="error">{errors[it.id]}</div>{/if}
      <div class="muted small">Poslední kontrola: {formatDate(it.lastSuccessAt)}</div>
    </div>
    <div class="actions">
      <label class="inline"><input type="checkbox" checked={it.enabled} onchange={() => toggle(it)} /> Hlídat</label>
      {#if confirmId === it.id}
        <span>Smazat i s historií?</span>
        <button class="danger" onclick={() => remove(it)}>Ano, smazat</button>
        <button onclick={() => (confirmId = null)}>Ne</button>
      {:else}
        <button onclick={() => (editing = it)}>Upravit</button>
        <button onclick={() => (confirmId = it.id)}>Smazat</button>
      {/if}
    </div>
  </div>
{/each}

<style>
  .head { justify-content: space-between; margin-bottom: 0.75em; }
  .head h2 { margin: 0; }
  .item { display: flex; gap: 1em; justify-content: space-between; flex-wrap: wrap; }
  .item.off { opacity: 0.7; }
  .main { min-width: 220px; flex: 1; }
  .actions { display: flex; gap: 0.5em; align-items: center; flex-wrap: wrap; }
  .small { font-size: 0.85rem; margin-top: 0.3em; }
</style>
