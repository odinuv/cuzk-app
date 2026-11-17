# Strážce katastru – desktopová appka (Tauri) – výkop zadání

Datum: 2026-11-17 · Stav: výkop, schváleno uživatelem (2026-11-17)

## Proč

Azure funkce (`function/`) ověřila, že hlídání nemovitostí přes REST API KN funguje. Teď z toho
má být **desktopová appka pro ostatní lidi**: nainstalují si ji, zadají svůj API klíč ČÚZK a své
nemovitosti a appka je pravidelně kontroluje – bez Azure, bez e-mailů, data jen lokálně.

Současný TS kód je **inspirace a PoC**, ne předloha k doslovnému převodu. Bere se z něj ověřené
chování API KN (viz „Poznatky z PoC“), architektura se navrhuje znovu.

## Cíle

- První spuštění začíná zadáním **API klíče** (ověření proti API, uložení).
- Uživatel ručně spravuje **seznam nemovitostí**: parcely, stavby, jednotky; popis; zapnuto/vypnuto.
- **Kontrola** stáhne aktuální stav, porovná s předchozím a **zobrazí výsledek** v appce:
  plomby (zahájená řízení) nahoře, pak ostatní změny, „první kontrola“, chyby.
- Kontrola jde spustit **z UI** i **bez UI** (`strazce --check`) – stejný kód.
- Appka se umí **nastavit jako plánovaná úloha** v OS (zapnout/vypnout, interval).
- Data lokálně v **SQLite**: seznam, historie běhů a snapshotů, cache dohledaných ISKN id.
- **Instalátor** pro běžného uživatele.

## Non-goals (teď)

- E-maily. Import seznamu (CSV, ze strazcekatastru.cz, …) – až později.
- Synchronizace mezi počítači, víc profilů/účtů, cloud.
- Vlastníci – API KN je nevrací.
- Auto-update a podepisování instalátorů – až po první použitelné verzi.

## Anti-goals (selhání, i když to „funguje“)

- **Falešné změny** – výpadek API nebo chyba dohledání nesmí vypadat jako změna/zmizení objektu.
- **Překročení limitu 500 volání/den** na klíč (klíč je uživatelův, limit se sdílí i s jeho jinými nástroji).
- **Tichý plánovaný běh** – když naplánovaná kontrola selže nebo najde změnu, uživatel se to musí
  dozvědět (systémová notifikace), ne jen v logu.
- **Únik API klíče** – nesmí ležet v čitelném souboru vedle databáze.
- Dva běhy současně (plánovaný + ruční) nad stejnou databází.

## Omezení

- API KN: hlavička `ApiKey`, 500 volání/den, odpovědi `{data, zpravy, aktualnostDatK, provedenoVolani}`;
  výjimka `/AplikacniSluzby/StavUctu` je plochý objekt. Swagger: https://api-kn.cuzk.gov.cz/swagger/v1.0/swagger.json
- Stavby a jednotky se nevyhledávají podle k.ú., ale přes parcelu / kód části obce (viz níže).
- Uživatelé nejsou technici – nesmí potřebovat znát kód k.ú. ani terminál.

## Approaches

1. **Jádro v Rustu, UI v TS jen zobrazuje – zvoleno.** `--check` běží jako čistý proces bez webview,
   plánovač OS ho spouští spolehlivě; UI volá totéž jádro přes Tauri commands.
2. Jádro v TS ve webview (převzít PoC) – plánovaný běh by potřeboval skryté okno s celým webview;
   křehké bez přihlášeného uživatele, hůř se supportuje. Zamítnuto.
3. Přibalený Node sidecar s PoC kódem – velký instalátor, další pohyblivá část. Zamítnuto.

## Rozhodnuto

- **Tauri 2**, jeden binár: bez argumentů otevře UI, s `--check` provede kontrolu headless a skončí
  (exit code ≠ 0 při chybě běhu).
- **Rust jádro** jako samostatný crate (`strazce-core`) bez závislosti na Tauri: API klient (low-level
  HTTP, `reqwest`), dohledání ISKN id, normalizace + diff, úložiště (SQLite, `rusqlite`), orchestrace běhu.
  Tauri app a CLI jsou jen tenké vrstvy nad ním. Testy jádra s mockovaným HTTP.
- **API klíč** (zdarma, uživatel si ho vyřídí u ČÚZK a vloží do appky) při prvním spuštění: zadání → ověření `StavUctu` → uložení do **úložiště klíčů OS**
  (Windows Credential Manager / macOS Keychain / Secret Service; crate `keyring`). *(můj návrh)*
- **Výsledek** jen v appce; u headless běhu **systémová notifikace** se shrnutím
  („2 změny, 1 plomba“ / „beze změn“ / „chyba“), klik otevře appku na výsledku. *(můj návrh)*

## Návrh (výkop)

### Obrazovky
```
┌ Strážce katastru ───────────────────────────────────────────┐
│ [Nemovitosti] [Výsledky] [Nastavení]        [Zkontrolovat]  │
├─────────────────────────────────────────────────────────────┤
│ Poslední kontrola 17. 11. 2026 · 2 změny · 1 PLOMBA         │
│ ▲ PLOMBA  Kudelova, parc. 3756 – řízení V-1234/2026-701     │
│   změna   Chata, stavba č.e. 3 – způsob využití: … → …      │
│   ok      37 dalších beze změn                              │
└─────────────────────────────────────────────────────────────┘
```
- **Úvod/onboarding:** zadání API klíče, odkaz kde ho získat, ověření (zobrazí limit a čerpání).
- **Nemovitosti:** seznam + formulář přidání/úpravy. Typ (parcela / stavba / jednotka), popis, zapnuto.
  K.ú. **vyhledávané podle názvu** (číselník k.ú. z API, cache v SQLite) – uživatel nezná kódy. *(můj návrh)*
  - parcela: k.ú., stavební/pozemková, kmenové číslo, poddělení
  - stavba: k.ú., č.p./č.e., číslo domovní, parcela pod stavbou
  - jednotka: k.ú., č.p./č.e., číslo domovní, číslo jednotky (část obce dohledat automaticky, ruční přebití při nejednoznačnosti)
  - po uložení hned zkusit dohledat ISKN id → uživatel vidí chybu zadání okamžitě, ne až při kontrole. *(můj návrh)*
- **Výsledky:** nové nálezy od posledního zobrazení (plomby, změny) a poslední běh (plomby nahoře, změny jako `pole: staré → nové`, první kontroly, chyby),
  historie běhů, detail objektu (aktuální data z API).
- **Nastavení:** API klíč (změnit/smazat), stav účtu API, plánovaná kontrola (zapnuto, interval, čas).

### Běh kontroly
**Termín kontroly má každá položka zvlášť** (rozhodnutí uživatele): položka je *na řadě*, když od jejího
posledního úspěšného stažení uplynul interval (nebo ještě nebyla kontrolována). Běh zkontroluje položky
na řadě – nejstarší první – dokud nedojdou dnešní volání API (s malou rezervou), zbytek nechá na další
běh. Díky tomu appka zvládne libovolný počet položek bez zvláštní logiky: 1000 položek se rozloží do
několika dní a pak se samy rozprostřou. Když není nic na řadě, běh nestojí žádné volání.

1. Vybrat zapnuté položky na řadě; `StavUctu` → kolik volání zbývá dnes.
2. Pro každou (dokud je rozpočet): ISKN id z cache (jinak dohledat), `GET /{Parcely|Stavby|Jednotky}/{id}`;
   sekvenčně, retry 429/5xx/síť s backoffem, timeout. Došlá volání = běh končí normálně („zbývá N položek“), ne chyba.
3. Diff proti poslednímu úspěšnému stavu položky: plomby podle id řízení (u nových `GET /Rizeni/{id}`),
   ostatní hluboký diff s normalizací pořadí polí. Chyba stažení → chyba v reportu, starý stav zůstává.
   404 na cachované id → znovu dohledat. Změna identifikace položky → „první kontrola“.
4. Uložit běh a výsledky do SQLite v jedné transakci.

### Plánování
- Zapnutí v Nastavení zaregistruje úlohu v OS, která spouští `strazce --check`:
  Windows Plánovač úloh, macOS launchd (LaunchAgent), Linux systemd user timer (fallback cron). *(můj návrh)*
- Výchozí interval 14 dní, uživatel může změnit (týdně / 14 dní / měsíčně). Zmeškaný běh (PC vypnuté)
  doběhne při příštím spuštění, pokud to OS umí. *(můj návrh)*
- Zámek proti souběhu dvou běhů (ruční + plánovaný).

### Upřesnění (moje rozhodnutí po kontrole zadání)
- **Plomby:** pole `rizeniPlomby` v odpovědi parcely/stavby/jednotky; nová = řízení, které minule nebylo,
  zaniklá = řízení, které zmizelo (diff podle `id`). Referenční chování je v PoC: `function/src/diff.ts`,
  `resolve.ts`, `run.ts` – brát jako popis chování, ne jako kód k převodu.
- **Platformy v1:** povinně Windows; plánování a úložiště klíčů za rozhraním, macOS/Linux až později.
  Plánovaný běh jen pod přihlášeným uživatelem (kvůli úložišti klíčů a notifikacím) – přijatelné.
- **Plánovač spouští denně, appka rozhoduje:** úloha v OS běží 1× denně a `--check` vezme položky na řadě
  (viz Běh kontroly). Vynechaný den nevadí; položky, které selhaly nebo se nevešly do limitu, zůstanou
  na řadě a zkusí se další den. Den bez položek na řadě stojí 0 volání API.
- **Notifikace** po headless běhu jen otevře appku (na záložce Výsledky), bez deep-linku na konkrétní běh.
- **Souběh:** zámek souborem v datovém adresáři; plánovaný běh proběhne i při otevřeném UI,
  UI si po dokončení obnoví data (sleduje změnu DB / znovunačtení při fokusu). Ruční kontrola při
  běžícím plánovaném běhu ukáže „kontrola už běží“.
- **Identita položky:** položka má stabilní interní id; úprava identifikace (k.ú., čísla) = nové dohledání
  a „první kontrola“, historie zůstává u položky s vyznačeným předělem. Smazání položky smaže
  i její historii (po potvrzení).
- **Rozpočet volání:** UI ukazuje zbývající volání dnes, kolik položek je na řadě a kolik jich čeká na
  další dny; dohledání při uložení položky stojí 1–3 volání.

### Data (SQLite, v datovém adresáři appky)
Orientačně: `items` (definice nemovitosti), `resolved` (item → endpoint + ISKN id + otisk identifikace),
`ku_cache` (číselník k.ú. a částí obcí), `runs` (čas, stav, počty, čerpání API, aktuálnost dat KN),
`item_results` (běh × položka: stav, data JSON, změny JSON, chyba). Schéma s migracemi.

### Distribuce
Tauri bundler: Windows (NSIS/MSI) jako první platforma, macOS (dmg) a Linux (AppImage/deb) podle zájmu. *(můj návrh)*

## Poznatky z PoC (ověřeno proti živému API)

- Parcela: `GET /Parcely/Vyhledani` (k.ú., typ PKN, druh číslování 1=stavební/2=pozemková, kmenové, poddělení).
- Stavba: přes parcelu pod ní → `data.stavba.id`; ověřit, že `cislaDomovni` obsahuje číslo domovní.
- Jednotka: `GET /CiselnikyUzemnichJednotek/KatastralniUzemi/{kod}` → `kodObce`; `GET /CiselnikyUzemnichJednotek/CastiObci`
  (celý číselník, ~15 tis. záznamů – cachovat) → část obce se stejným názvem jako k.ú. v téže obci → `GET /Jednotky/Vyhledani`.
- Typ stavby 1 = č.p., 2 = č.e. První kontrola 39 položek ≈ 82 volání, další ≈ 40.
- `provedenoVolani` ve `StavUctu` nezahrnuje právě provedené volání `StavUctu`.

## Na builderovi

- Frontendový framework (React/Svelte/…), vzhled, komponenty.
- Přesné schéma SQLite a migrace, struktura repozitáře/crates.
- Podoba notifikací, ikonky, texty.

## Otevřené (doladí se později)

- Import seznamu (formát, zdroje).
- Podepisování instalátorů, auto-update, macOS notarizace.
- Jazyk UI (čeština jen, nebo i angličtina).
- Retence historie (kolik běhů/snapshotů držet).
