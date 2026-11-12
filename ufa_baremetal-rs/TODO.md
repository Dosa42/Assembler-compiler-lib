# TODO — C → UAGL → AArch64 in één offline playground

Maak één `ufa-baremetal-playground.html` waarin je C, UAGL, UAGLBR of AArch64-assembly kunt openen, zelf stappen kiest en die met **Run pipeline** uitvoert. Elke stap kan ook los worden uitgevoerd.

## 1. Bestaande projecten samenbrengen

- [x] Gebruik `c-uagl-rust` voor C → UAGL, terugvertaling naar C en fidelity. Gebruik `ufa-baremetal-rs` voor UAGL → AArch64 en K01–K06.
- [x] Breid de bestaande playground-build uit. Eén buildcommando compileert de drie Rust-Wasm-modules en sluit ze in één HTML-bestand in.
- [x] Verbind de bestaande Rust-ingangen met JavaScript. De HTML werkt offline vanaf schijf. De build blijft bruikbaar met de vastgelegde Cargo-lockbestanden.
- [x] Houd de bestaande native CLI-commando’s en uitvoerformaten bruikbaar. De browser en de CLI delen de Rust-implementaties.

## 2. Functies beschikbaar maken als stappen

| Stap | Invoer | Uitvoer | Rust/Wasm-ingang |
| --- | --- | --- | --- |
| C → UAGL | C-bron en bronbestandsnaam | `.uagl`, convertervalidatie en `.gaps.json` | `convert_c` |
| UAGL voor de assembler controleren | `.uagl` of `.uaglbr` | Brondiagnoses per regel | `validate` |
| UAGL → AArch64 | `.uagl` of `.uaglbr` | AArch64-`.s` | `assemble` |
| K01–K06 uitvoeren | Bestaande of gegenereerde `.s` en de bestandsnaam | Uitspraken voor de geselecteerde controles | `check_selected` |
| UAGL → C | UAGL-tekst | Terugvertaald `.c` | `uagl_to_c` |
| Fidelity meten | Originele C en UAGL | Structureel vergelijkingsoverzicht | `fidelity_report` |
| Verify | Bestaande of gegenereerde `.s` en referentie-`.s` | Match of regelverschillen | Bestaande vergelijking in de playground |

- [x] Laat gebruikers bron- en referentiebestanden openen, tekst bewerken en elke beschikbare uitvoer downloaden.
- [x] Toon per stap de gekozen invoer, status, meldingen en uitvoer. Laat resultaten op K-code of functienaam filteren waar die gegevens beschikbaar zijn.
- [x] Laat de gebruiker kiezen of de bestaande convertervalidatie bij **C → UAGL** meeloopt. Geef die meldingen een eigen naam naast de broncontrole van de assembler.

## 3. Volledig instelbare pipeline

- [x] In **Custom** kun je stappen toevoegen, herhalen, verwijderen en verplaatsen. Je kunt per stap kiezen tussen een geopend bestand en uitvoer van een eerdere stap.
- [x] Controleer vóór **Run pipeline** of de gekozen invoer past bij de stap. Toon welk bestand ontbreekt en waar je het kunt koppelen.
- [x] Bewaar oorspronkelijke invoer en tussentijdse uitvoer apart. Een geopende `.s` blijft beschikbaar als later een andere `.s` wordt gegenereerd.
- [x] Maak **Doorgaan bij meldingen** de standaard voor de nieuwe geïntegreerde pipeline. Houd waarschuwingen zichtbaar. Voer vervolgstappen uit zodra hun invoer beschikbaar is. Sla alleen stappen over waarvoor vereiste invoer ontbreekt.
- [x] Bied **Stop bij fouten** als keuze per pipeline. In die modus stopt de run bij de eerste mislukte stap en blijft eerdere uitvoer beschikbaar.
- [x] Laat pipelines exporteren en importeren als JSON met stapvolgorde, invoerkoppelingen, K-selectie en foutbeleid. Houd bronbestanden en referentiebestanden apart van de pipelineconfiguratie.
- [x] Markeer resultaten als verouderd wanneer hun invoer of instellingen wijzigen. Bewaar de laatste uitvoer voor vergelijking en download.

| Startpunt | Voorinstelling |
| --- | --- |
| C-bron | C → UAGL → UAGL-controle → Assembler → K-checks → optioneel Verify |
| UAGL of UAGLBR | UAGL-controle → Assembler → K-checks → optioneel Verify |
| Bestaande `.s` plus UAGL | K-checks op bestaande `.s` → Assembler → Verify van de nieuwe `.s` |
| C en UAGL vergelijken | C → UAGL → Fidelity en/of UAGL → C |

## 4. Auditbevindingen koppelen

- [x] Neem convertergaten en assemblerdiagnoses op in één overzicht. Toon per melding de herkomst, categorie, UAGL-regel en C-bronlocatie zodra die beschikbaar is.
- [x] Behoud de bestaande converter-`.gaps.json`. Bied een aparte export van het gecombineerde overzicht aan.
- [x] Gebruik gerichte gevallen voor prefixcasts, `INT128`, geneste initializers, `SIZEOF` en K03. Leg per geval vast wat de converter schrijft, wat de assembler ondersteunt en welke controle het verschil kan zien.
- [x] Volg K03-meldingen terug tot de werkelijke operandbreedtes. Verander een `MUL` pas als een gerichte test aantoont welke instructie nodig is.
- [x] Houd de status van **Verify** bij de vergelijking van twee `.s`-bestanden. Plan een aparte AArch64-uitvoeringstest voor gedragsvergelijking met de originele C. Die stap assembleert, linkt en voert de uitvoer uit.

## 5. Acceptatiechecklist

- [ ] Open het enige gegenereerde HTML-bestand offline in een browser. Test de drie Wasm-modules en controleer dat de pagina geen externe bestanden ophaalt.
- [x] Test elke functie los en alle voorinstellingen met één klik. Test ook een aangepaste volgorde met een bestaande `.s`, hergebruik van uitvoer en export/import van de configuratie.
- [x] Test beide foutmodi met dezelfde bekende diagnose. In de ene modus blijft de beschikbare `.s` bruikbaar. In de andere stopt de pipeline bij de eerste mislukte stap.
- [x] Vergelijk de browserconversie van de aangeleverde Ed25519-bron byte voor byte met de huidige native Rust-referentie voor UAGL, gaps en fidelity. Controleer dat de bekende diagnoses zichtbaar blijven en beschikbare uitvoer downloadbaar is.
- [x] Controleer dat geselecteerde K-checks, verschillende bestandsnamen voor K05, referentieverschillen en het opnieuw uitvoeren van gewijzigde invoer werken.

## Verificatie van deze build

- De drie echte Wasm-modules en de bediening zijn getest in een DOM-simulatie zonder externe bestanden. De directe test in een grafische browser blijft open.
- De Ed25519-uitvoer van de converter, de gap-JSON, de fidelity-uitvoer en de terugvertaalde C kwamen byte voor byte overeen met de native Rust-referentie.
- De aparte AArch64-uitvoeringstest draaide onder QEMU. De volledige Ed25519-assembly kon tot objectbestand worden geassembleerd. Het linken stopte op ongedefinieerde symbolen. Zie de auditgevallen in `README.md`.
