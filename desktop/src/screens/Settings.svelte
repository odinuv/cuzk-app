<script lang="ts">
  import { deleteApiKey, getSettings, saveSettings, type ApiStatus, type SettingsView } from "../lib/api";
  import { formatDate, formatDateTime } from "../lib/format";
  import KeyForm from "./KeyForm.svelte";

  let { status, onstatus }: { status: ApiStatus; onstatus: (s: ApiStatus | null) => void } = $props();

  let settings = $state<SettingsView | null>(null);
  let loadError = $state<string | null>(null);
  let saveMsg = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let changingKey = $state(false);
  let confirmDelete = $state(false);
  let keyError = $state<string | null>(null);

  function load() {
    getSettings().then(
      (s) => (settings = s),
      (err) => (loadError = String(err)),
    );
  }

  $effect(load);

  async function save() {
    if (!settings) return;
    saveMsg = null;
    saveError = null;
    try {
      const { intervalDays, scheduleEnabled, scheduleTime } = settings;
      await saveSettings({ intervalDays, scheduleEnabled, scheduleTime });
      saveMsg = scheduleEnabled ? "Uloženo, plánovaná kontrola je zapnutá." : "Uloženo.";
      load();
    } catch (err) {
      saveError = String(err);
    }
  }

  async function removeKey() {
    keyError = null;
    try {
      await deleteApiKey();
      onstatus(null);
    } catch (err) {
      keyError = String(err);
    }
  }
</script>

<h2>Nastavení</h2>

<div class="card">
  <h3>API klíč</h3>
  <p>
    Klíč je uložen. Denní limit: <strong>{status.limit ?? "—"}</strong> volání, dnes využito:
    <strong>{status.used ?? "—"}</strong>{#if status.expires}, platnost do {formatDate(status.expires)}{/if}.
  </p>
  {#if status.error}<p class="error">{status.error}</p>{/if}
  {#if changingKey}
    <KeyForm onsaved={(s) => { changingKey = false; onstatus(s); }} label="Ověřit a uložit nový klíč" />
    <button class="link" onclick={() => (changingKey = false)}>Zrušit</button>
  {:else}
    <div class="row">
      <button onclick={() => (changingKey = true)}>Změnit klíč</button>
      {#if confirmDelete}
        <span>Opravdu smazat klíč?</span>
        <button class="danger" onclick={removeKey}>Ano, smazat</button>
        <button onclick={() => (confirmDelete = false)}>Ne</button>
      {:else}
        <button onclick={() => (confirmDelete = true)}>Smazat klíč</button>
      {/if}
    </div>
  {/if}
  {#if keyError}<p class="error">{keyError}</p>{/if}
</div>

<div class="card">
  <h3>Plánovaná kontrola</h3>
  {#if loadError}
    <p class="error">{loadError}</p>
  {:else if settings}
    <p class="muted">
      Windows spouští kontrolu každý den v zadaný čas; aplikace zkontroluje jen ty nemovitosti, na které
      už došla řada podle intervalu.
    </p>
    <div class="fields">
      <label class="inline"><input type="checkbox" bind:checked={settings.scheduleEnabled} /> Kontrolovat automaticky</label>
      <label>
        Interval kontroly jedné nemovitosti
        <select bind:value={settings.intervalDays}>
          <option value={7}>každých 7 dní</option>
          <option value={14}>každých 14 dní</option>
          <option value={30}>každých 30 dní</option>
        </select>
      </label>
      <label>
        Čas spuštění
        <input type="time" bind:value={settings.scheduleTime} />
      </label>
    </div>
    {#if settings.scheduleEnabled && settings.scheduleTaskPresent === false}
      <p class="error">
        Plánovaná úloha ve Windows chybí, automatická kontrola proto neběží. Uložte nastavení znovu, aby se
        úloha vytvořila.
      </p>
    {:else if !settings.scheduleEnabled && settings.scheduleTaskPresent === true}
      <p class="error">
        V Plánovači úloh Windows zůstala úloha StrazceKatastru, přestože je plánovaná kontrola vypnutá. Uložte
        nastavení znovu, aby se úloha smazala.
      </p>
    {/if}
    {#if settings.scheduleEnabled}
      <p class="small">
        Plánovaná kontrola naposledy proběhla:
        <strong>{settings.lastScheduledRun ? formatDateTime(settings.lastScheduledRun) : "zatím nikdy"}</strong>
      </p>
      {#if settings.scheduleStale}
        <p class="error">
          Plánovaná kontrola neproběhla déle než 2 dny. Zkontrolujte, že je počítač v nastavený čas zapnutý
          a vy přihlášeni, případně nastavení uložte znovu.
        </p>
      {/if}
    {/if}
    <button class="primary" onclick={save}>Uložit nastavení</button>
    {#if saveMsg}<p class="ok">{saveMsg}</p>{/if}
    {#if saveError}<p class="error">{saveError}</p>{/if}
  {:else}
    <p class="muted">Načítám…</p>
  {/if}
</div>

<style>
  .small { font-size: 0.9rem; }
  .fields { display: flex; flex-direction: column; gap: 0.8em; margin: 1em 0; max-width: 360px; }
</style>
