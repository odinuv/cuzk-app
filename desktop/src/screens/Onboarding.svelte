<script lang="ts">
  import { formatDate } from "../lib/format";
  import type { ApiStatus } from "../lib/api";
  import KeyForm from "./KeyForm.svelte";

  let { ondone }: { ondone: (s: ApiStatus) => void } = $props();

  const KEY_URL = "https://www.cuzk.gov.cz";
  let verified = $state<ApiStatus | null>(null);
  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(KEY_URL);
      copied = true;
    } catch {
      copied = false;
    }
  }
</script>

<div class="wrap">
  <h1>Strážce katastru</h1>
  <p>
    Aplikace hlídá změny na vašich nemovitostech v katastru nemovitostí a upozorní vás na plomby
    (zahájená řízení). Data zůstávají jen ve vašem počítači.
  </p>

  <div class="card">
    <h2>Vložte svůj API klíč ČÚZK</h2>
    <p class="muted">
      Klíč je zdarma, vyřídíte si ho u Českého úřadu zeměměřického a katastrálního (stránka
      <span class="selectable">{KEY_URL}</span>, sekce o API Katastru nemovitostí).
      <button class="link" type="button" onclick={copy}>{copied ? "Zkopírováno" : "Kopírovat adresu"}</button>
    </p>
    {#if !verified}
      <KeyForm onsaved={(s) => (verified = s)} />
    {:else}
      <p class="ok">
        Klíč je platný a uložený. Denní limit: <strong>{verified.limit ?? "—"}</strong> volání, dnes využito:
        <strong>{verified.used ?? "—"}</strong>{#if verified.expires}, platnost do {formatDate(verified.expires)}{/if}.
      </p>
      {#if verified.error}<p class="error">{verified.error}</p>{/if}
      <button class="primary" onclick={() => ondone(verified!)}>Pokračovat</button>
    {/if}
  </div>
</div>

<style>
  .wrap { max-width: 640px; margin: 2em auto; padding: 0 1em; }
  .selectable { user-select: text; font-family: Consolas, monospace; }
</style>
