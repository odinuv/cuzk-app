# Strážce katastru (desktopová aplikace)

Windows aplikace, která hlídá vybrané nemovitosti (parcely, stavby, jednotky) v katastru nemovitostí přes API ČÚZK a upozorní na změny a na plomby (rozepsaná řízení).

## Pro uživatele

### Co aplikace dělá
- Sleduje vámi zadané parcely, stavby a jednotky. Při každé kontrole porovná aktuální data s posledním uloženým stavem a ukáže, co se změnilo.
- Plomby (nově zapsaná nebo zaniklá řízení) jsou ve výsledcích zvýrazněné nahoře.
- Položky se kontrolují postupně, každá nejvýše jednou za zvolený interval (7, 14 nebo 30 dní, výchozí 14). Denní limit API se nikdy nepřekročí: co se do limitu nevejde, počká na další den.
- Výsledky jsou jen v aplikaci, u plánované kontroly navíc přijde systémová notifikace se shrnutím.

### API klíč ČÚZK
Aplikace potřebuje vlastní klíč k API ČÚZK. Klíč je zdarma a umožňuje 500 volání za den. Získáte ho na https://www.cuzk.gov.cz (přesný odkaz se při prvním spuštění zobrazí v aplikaci s tlačítkem „Kopírovat adresu“). Klíč vložíte při prvním spuštění; aplikace ho ověří a ukáže limit, aktuální čerpání a platnost. Později ho změníte nebo smažete v Nastavení.

Klíč se ukládá do Správce pověření systému Windows (Credential Manager) pod názvem `strazce-katastru`, ne do souboru.

### Plánovaná kontrola
V Nastavení lze zapnout denní kontrolu v zadaný čas. Aplikace k tomu vytvoří úlohu `StrazceKatastru` v Plánovači úloh Windows, která spouští `strazce.exe --check` bez okna. Úloha běží jen pod přihlášeným uživatelem (kvůli úložišti klíčů a notifikacím), takže počítač musí být v daný čas zapnutý a vy přihlášeni. Vypnutím plánu se úloha smaže.

### Kde jsou data
- Databáze: `%APPDATA%\cz.strazce-katastru.app\strazce.db`
- Log chyb: `%APPDATA%\cz.strazce-katastru.app\strazce.log`
- API klíč: Správce pověření Windows, položka `strazce-katastru`

### Odinstalace a smazání dat
1. V Nastavení aplikace vypněte plánovanou kontrolu (smaže se úloha) a smažte API klíč.
2. Odinstalujte aplikaci (Nastavení Windows, Aplikace). Instalace je pro jednoho uživatele a nevyžaduje práva správce.
3. Chcete-li smazat i data, odstraňte složku `%APPDATA%\cz.strazce-katastru.app`. Případný zbylý klíč odstraníte ve Správci pověření (`strazce-katastru`) a zbylou úlohu příkazem `schtasks /Delete /F /TN StrazceKatastru`.

Instalátor používá zástupnou ikonu (bez finální grafiky).

## Pro vývojáře

### Požadavky
- Rust stable s MSVC toolchainem (`x86_64-pc-windows-msvc`)
- Node.js a npm
- Visual Studio Build Tools (C++ workload) a WebView2 (je součástí Windows 11)

Nebuildujte ve složce pod OneDrive: Windows Defender (Controlled Folder Access) blokuje zápis build výstupů. Pracujte v repozitáři mimo OneDrive.

### Příkazy (ve složce `desktop/`)
```
npm install
npm run tauri dev          # vývojový běh
npm test                   # testy frontendu (vitest)
npm run check              # svelte-check
cargo test --workspace     # testy Rustu
npm run tauri build        # instalátor
```
Instalátor (NSIS, instalace pro aktuálního uživatele) vznikne v `desktop/target/release/bundle/nsis/` jako `Strážce katastru_<verze>_x64-setup.exe`. První build stáhne nástroje NSIS.

### Architektura
- `core/` (crate `strazce-core`): veškerá logika bez závislosti na UI: klient API ČÚZK (`api/`), překlad zadání na identifikátory (`resolve.rs`), porovnání snapshotů (`diff.rs`), SQLite (`db/`), běh kontroly s rozpočtem volání (`run.rs`), zámek proti souběžným během (`lock.rs`), úložiště klíče (`secrets.rs`), plánovač (`schedule.rs`), headless běh a notifikace (`headless.rs`).
- `src-tauri/` (crate `strazce`): tenká Tauri 2 vrstva; příkazy pro UI (`commands.rs`, `ipc.rs`) a `main.rs`.
- `src/`: frontend ve Svelte 5 a TypeScriptu, s Tauri API volaným jen přes `lib/api.ts`.
- Cesty k datům počítá jediné místo, `core/src/paths.rs`.
- Klíč se nikdy neposílá do frontendu ani do logu.

### Headless režim
`strazce.exe --check` provede jednu plánovanou kontrolu bez okna a vrátí kód:

| Kód | Význam |
|-----|--------|
| 0 | kontrola doběhla (i částečně) |
| 1 | selhání (databáze, API, kontrola nedoběhla) |
| 2 | chybí nebo je neplatný API klíč |
| 3 | jiná kontrola právě běží |

### Ruční ověření instalátoru a plánu
Na čistém Windows:
1. Spusťte instalátor a nainstalujte aplikaci.
2. V Nastavení zapněte plánovanou kontrolu; `schtasks /Query /TN StrazceKatastru` musí úlohu zobrazit.
3. `schtasks /Run /TN StrazceKatastru` musí spustit kontrolu bez konzolového okna a měla by se objevit notifikace.
4. Vypněte plán v Nastavení; úloha musí zmizet (`schtasks /Query /TN StrazceKatastru` selže).
