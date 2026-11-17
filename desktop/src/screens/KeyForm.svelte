<script lang="ts">
  import { setApiKey, type ApiStatus } from "../lib/api";

  let { onsaved, label = "Ověřit a uložit" }: { onsaved: (s: ApiStatus) => void; label?: string } = $props();

  let key = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: Event) {
    e.preventDefault();
    if (!key.trim()) {
      error = "Vložte API klíč.";
      return;
    }
    busy = true;
    error = null;
    try {
      const s = await setApiKey(key.trim());
      key = "";
      onsaved(s);
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }
</script>

<form onsubmit={submit} class="row">
  <input type="password" bind:value={key} aria-label="API klíč ČÚZK" placeholder="API klíč ČÚZK" autocomplete="off" style="flex: 1; min-width: 220px" />
  <button class="primary" type="submit" disabled={busy}>{busy ? "Ověřuji…" : label}</button>
</form>
{#if error}<p class="error">{error}</p>{/if}
