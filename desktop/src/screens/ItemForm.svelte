<script lang="ts">
  import {
    castiObciForKu, saveItem, searchKu,
    type CastObceOption, type ItemKind, type ItemView, type KuOption,
  } from "../lib/api";
  import { emptyIdentForm, identFromForm, identToForm, type IdentForm } from "../lib/ident";

  let { item = null, onsaved, oncancel }: {
    item?: ItemView | null;
    onsaved: () => void;
    oncancel: () => void;
  } = $props();

  // The form is re-mounted per edit session by the parent, so reading the prop once is intended.
  // svelte-ignore state_referenced_locally
  const initial = item;
  let popis = $state(initial?.popis ?? "");
  let enabled = $state(initial?.enabled ?? true);
  let f = $state<IdentForm>(initial ? identToForm(initial.ident) : emptyIdentForm());
  let editingId = $state<number | undefined>(initial?.id);
  let resolveError = $state<string | null>(initial?.resolveError ?? null);

  let kuText = $state(initial ? initial.describe : "");
  let kuOptions = $state<KuOption[]>([]);
  let kuOpen = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0; // guards against out-of-order searchKu responses

  let casti = $state<CastObceOption[]>([]);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const kinds: { id: ItemKind; label: string }[] = [
    { id: "parcela", label: "Parcela" },
    { id: "stavba", label: "Stavba" },
    { id: "jednotka", label: "Jednotka" },
  ];

  function onKuInput() {
    f.ku = null;
    kuOpen = true;
    seq++;
    clearTimeout(timer);
    const q = kuText.trim();
    if (q.length < 2) {
      kuOptions = [];
      return;
    }
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const found = await searchKu(q);
        if (mine === seq) kuOptions = found;
      } catch (err) {
        if (mine === seq) {
          kuOptions = [];
          error = String(err);
        }
      }
    }, 250);
  }

  function pickKu(o: KuOption) {
    seq++;
    f.ku = o.kod;
    kuText = `${o.nazev} (${o.obec})`;
    kuOptions = [];
    kuOpen = false;
    f.kodCastiObce = null;
  }

  // část obce picker: only for a jednotka whose last resolve failed
  const showCasti = $derived(f.kind === "jednotka" && resolveError !== null && f.ku !== null);
  $effect(() => {
    const ku = f.ku;
    if (!showCasti || ku === null) return;
    castiObciForKu(ku).then(
      (c) => (casti = c),
      (err) => (error = String(err)),
    );
  });

  async function submit(e: Event) {
    e.preventDefault();
    error = null;
    let ident;
    try {
      if (!popis.trim()) throw new Error("Vyplňte popis nemovitosti.");
      ident = identFromForm(f);
    } catch (err) {
      error = (err as Error).message;
      return;
    }
    busy = true;
    try {
      const saved = await saveItem({ id: editingId, popis: popis.trim(), enabled, ident });
      editingId = saved.id;
      resolveError = saved.resolveError;
      onsaved();
      // Saved but the lookup failed: stay open so the user can fix it (or pick a část obce).
      if (!saved.resolveError) oncancel();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }
</script>

<form class="card" onsubmit={submit}>
  <h2>{editingId === undefined ? "Přidat nemovitost" : "Upravit nemovitost"}</h2>

  <div class="row" role="group" aria-label="Typ nemovitosti">
    {#each kinds as k (k.id)}
      <button type="button" class:primary={f.kind === k.id} onclick={() => (f.kind = k.id)}>{k.label}</button>
    {/each}
  </div>

  <div class="fields">
    <label>
      Popis
      <input type="text" bind:value={popis} placeholder="např. Chata, Kudelova" />
    </label>

    <label class="ku">
      Katastrální území
      <input
        type="text"
        bind:value={kuText}
        oninput={onKuInput}
        onfocus={() => (kuOpen = true)}
        onkeydown={(e) => e.key === "Escape" && (kuOpen = false)}
        onblur={() => setTimeout(() => (kuOpen = false), 150)}
        placeholder="začněte psát název, např. Ponava"
        autocomplete="off"
      />
      {#if kuOpen && kuOptions.length > 0}
        <ul class="options">
          {#each kuOptions as o (o.kod)}
            <li><button type="button" class="link" onclick={() => pickKu(o)}>{o.nazev} ({o.obec})</button></li>
          {/each}
        </ul>
      {/if}
      {#if f.ku === null && kuText.trim().length >= 2 && kuOptions.length === 0}
        <span class="muted">Vyberte území ze seznamu návrhů.</span>
      {/if}
    </label>

    {#if f.kind !== "parcela"}
      <div class="grid">
        <label>
          Typ čísla
          <select bind:value={f.typ}>
            <option value="Cp">č.p. (popisné)</option>
            <option value="Ce">č.e. (evidenční)</option>
          </select>
        </label>
        <label>
          Číslo domovní
          <input type="text" inputmode="numeric" bind:value={f.cisloDomovni} />
        </label>
        {#if f.kind === "jednotka"}
          <label>
            Číslo jednotky
            <input type="text" inputmode="numeric" bind:value={f.cisloJednotky} />
          </label>
        {/if}
      </div>
    {/if}

    {#if f.kind !== "jednotka"}
      <div class="grid">
        <label>
          {f.kind === "stavba" ? "Parcela pod stavbou: číslování" : "Číslování"}
          <select bind:value={f.parcela.druh}>
            <option value="Pozemkova">pozemková</option>
            <option value="Stavebni">stavební</option>
          </select>
        </label>
        <label>
          Kmenové číslo
          <input type="text" inputmode="numeric" bind:value={f.parcela.kmenove} />
        </label>
        <label>
          Poddělení (nepovinné)
          <input type="text" inputmode="numeric" bind:value={f.parcela.poddeleni} />
        </label>
      </div>
    {/if}

    {#if showCasti}
      <label>
        Část obce (nepovinné; vyberte, pokud se jednotku nepodařilo dohledat automaticky)
        <select bind:value={f.kodCastiObce}>
          <option value={null}>určit automaticky</option>
          {#each casti as c (c.kod)}
            <option value={c.kod}>{c.nazev}</option>
          {/each}
        </select>
      </label>
    {/if}

    <label class="inline"><input type="checkbox" bind:checked={enabled} /> Hlídat (zapnuto)</label>
  </div>

  {#if resolveError}<p class="error">Nemovitost je uložena, ale nepodařilo se ji dohledat: {resolveError}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}

  <div class="row">
    <button class="primary" type="submit" disabled={busy}>{busy ? "Ukládám a dohledávám…" : "Uložit"}</button>
    <button type="button" onclick={oncancel}>{resolveError ? "Zavřít" : "Zrušit"}</button>
  </div>
</form>

<style>
  .fields { display: flex; flex-direction: column; gap: 0.8em; margin: 1em 0; }
  .ku { position: relative; }
  .options {
    position: absolute; top: 100%; left: 0; right: 0; z-index: 5; margin: 2px 0 0; padding: 0.25em 0;
    list-style: none; background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--radius); max-height: 220px; overflow: auto; box-shadow: 0 4px 12px rgb(0 0 0 / 12%);
  }
  .options button { width: 100%; text-align: left; color: var(--text); }
  .options button:hover { background: var(--bg); }
</style>
