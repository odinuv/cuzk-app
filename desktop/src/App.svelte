<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    apiStatus, dueOverview, listItems, results, runCheck,
    type ApiStatus, type DueOverview, type ItemView, type ResultsView,
  } from "./lib/api";
  import { formatDue, formatRunSummary } from "./lib/format";
  import Onboarding from "./screens/Onboarding.svelte";
  import Items from "./screens/Items.svelte";
  import Results from "./screens/Results.svelte";
  import Settings from "./screens/Settings.svelte";

  type Tab = "items" | "results" | "settings";

  let status = $state<ApiStatus | null>(null);
  let statusLoaded = $state(false);
  let items = $state<ItemView[]>([]);
  let res = $state<ResultsView | null>(null);
  let due = $state<DueOverview | null>(null);
  let tab = $state<Tab>("results");
  let checking = $state(false);
  let checkMsg = $state<{ text: string; error: boolean } | null>(null);
  let loadError = $state<string | null>(null);

  const unseen = $derived(res?.findings.length ?? 0);

  // Local DB only: cheap, safe to call on every focus.
  async function refreshLocal() {
    try {
      [items, res] = await Promise.all([listItems(), results()]);
      loadError = null;
    } catch (err) {
      loadError = String(err);
    }
  }

  // Each of these spends one API call (StavUctu): only on start, after a check / key change, or explicit refresh.
  async function refreshRemote() {
    try {
      status = await apiStatus();
    } catch (err) {
      loadError = String(err);
    }
    statusLoaded = true;
    if (status?.hasKey) {
      try {
        due = await dueOverview();
      } catch {
        due = null;
      }
    } else {
      due = null;
    }
  }

  async function refreshAll() {
    await Promise.all([refreshLocal(), refreshRemote()]);
  }

  async function check() {
    checking = true;
    checkMsg = null;
    try {
      const s = await runCheck();
      checkMsg = { text: formatRunSummary(s), error: s.status === "failed" };
      tab = "results";
    } catch (err) {
      checkMsg = { text: String(err), error: true };
    } finally {
      checking = false;
    }
    await refreshAll();
  }

  function keyChanged(s: ApiStatus | null) {
    status = s;
    statusLoaded = true;
    void refreshAll();
  }

  onMount(() => {
    void refreshAll();
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void refreshLocal();
      })
      .then((u) => (unlisten = u))
      .catch(() => {});
    return () => unlisten?.();
  });
</script>

{#if !statusLoaded}
  <p class="center muted">Načítám…</p>
{:else if !status?.hasKey}
  <Onboarding ondone={keyChanged} />
{:else}
  <header>
    <nav class="row">
      <button class:primary={tab === "items"} onclick={() => (tab = "items")}>Nemovitosti</button>
      <button class:primary={tab === "results"} onclick={() => (tab = "results")}>
        Výsledky{#if unseen > 0} ({unseen}){/if}
      </button>
      <button class:primary={tab === "settings"} onclick={() => (tab = "settings")}>Nastavení</button>
    </nav>
    <div class="check">
      <button class="primary" onclick={check} disabled={checking}>{checking ? "Kontroluji…" : "Zkontrolovat"}</button>
    </div>
    <div class="due muted">
      {#if due}{formatDue(due)}{/if}
      <button class="link" onclick={refreshRemote} disabled={checking} title="Načíst znovu (stojí jedno volání API)">Obnovit</button>
    </div>
  </header>

  <main>
    {#if checkMsg}<p class:error={checkMsg.error} class="msg">{checkMsg.text}</p>{/if}
    {#if loadError}<p class="error">{loadError}</p>{/if}
    {#if tab === "items"}
      <Items {items} onchanged={refreshLocal} />
    {:else if tab === "results"}
      <Results data={res} onchanged={refreshLocal} />
    {:else if status}
      <Settings {status} onstatus={keyChanged} />
    {/if}
  </main>
{/if}

<style>
  header {
    display: grid; grid-template-columns: 1fr auto; gap: 0.4em 1em; align-items: center;
    padding: 0.75em 1em; background: var(--surface); border-bottom: 1px solid var(--border);
    position: sticky; top: 0; z-index: 10;
  }
  .due { grid-column: 1 / -1; font-size: 0.9rem; }
  main { max-width: 900px; margin: 1em auto; padding: 0 1em; }
  .center { text-align: center; margin-top: 3em; }
  .msg { font-weight: 600; }
</style>
