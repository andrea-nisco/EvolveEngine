# Evolve — Piano di implementazione: UI & Testo

> Documento operativo per l'agente di sviluppo. Va letto per intero prima di scrivere codice.
> Le caselle `[ ]` vanno spuntate man mano che i task vengono completati, così il documento resta lo stato di avanzamento del lavoro.

---

## 0. Come usare questo documento

1. Leggi tutto il piano, poi esegui la **Fase 0 (ricognizione)**: nessuna riga di codice UI prima di averla completata.
2. Procedi **una fase alla volta**, nell'ordine indicato. Ogni fase ha dei *criteri di accettazione*: una fase è chiusa solo quando sono tutti soddisfatti e i test passano.
3. Se una decisione di questo piano si rivela sbagliata o incompatibile con il codice esistente, **non deviare in silenzio**: scrivi una breve nota di decisione (ADR) in `docs/adr/` spiegando problema, alternativa scelta e motivo, e aggiorna questo piano.
4. I nomi di crate, moduli e file qui proposti sono indicativi: adattali alla struttura reale del workspace scoperta nella Fase 0.
5. Le decisioni prese sono registrate in [docs/adr/](docs/adr/) (0001-0006); la ricognizione è in [docs/ui/RECON.md](docs/ui/RECON.md). Dove questo piano e un ADR divergono, vale l'ADR.

---

## 1. Contesto

- **Evolve** è un editor/engine per videogiochi scritto in **Rust**, pensato per crescere a moduli in base alle necessità.
- Concetti già presenti: oggetti, scena, Play mode, Inspector.
- Rendering GPU tramite **wgpu** (aggiunto da poco; in origine il rendering era su CPU).
- Evolve è **nativamente orientato a SVG** fin dal giorno 0: è un requisito fondamentale, non un'aggiunta.
- Target: Windows, macOS, Linux, **Web, Android, iOS**.
- La logica di gioco **non è vincolata a Rust**: l'utente può scriverla in Python, C++ o altri linguaggi. Il sistema UI non deve quindi mai richiedere closure o tipi Rust per funzionare.
- Un altro sviluppatore sta lavorando in parallelo sul modulo di **fisica** (non ancora pubblicato). Non toccare il modulo; si procede in autonomia e l'unione la farà Andrea (ADR 0004). Annota nel riepilogo di ogni passo le modifiche a file esistenti del motore.
- Oggi la parte **UI/Testo manca completamente**.

---

## 2. Visione e principi

L'obiettivo è una UI di gioco che si costruisce come si costruisce un sito con React, ma pensata per i videogiochi: dichiarativa, a componenti, con un layout prevedibile e un aspetto curato già di default.

1. **La scena e la UI sono due cose separate.** Niente "Canvas" dentro la scena 3D/2D. La UI vive in asset dedicati (*UI Document*, file `.evui`), con un proprio spazio di coordinate e un editor dedicato. La scena si limita a dire quali documenti UI mostrare.
2. **Binari precisi.** Il layout è flexbox/grid (padding, gap, liste orizzontali e verticali, allineamenti), non posizionamento manuale con ancore. Il posizionamento assoluto esiste ma è l'eccezione.
3. **Bello di default.** Un design system integrato (token di colore, spaziatura, tipografia, raggi, ombre) e widget già rifiniti con varianti (`primary`, `secondary`, `ghost`, `danger`…). Personalizzare deve essere possibile, partire da zero non deve essere necessario.
4. **Il file è la fonte di verità.** Il formato `.evui` è testuale, leggibile, versionabile su Git, con sintassi stile JSX. L'editor visuale e l'editor di codice modificano lo stesso file e restano sincronizzati in entrambe le direzioni.
5. **Indipendente dalla risoluzione e dalla piattaforma** fin dal primo giorno: unità logiche, DPI, risoluzione di riferimento, safe area di iOS/Android.
6. **Agnostico rispetto al linguaggio di gioco.** La UI parla con il gioco solo tramite un *data store* (valori con un percorso, es. `player.health`) e *azioni* (eventi con nome, es. `menu.play`). Qualsiasi linguaggio che sappia chiamare un'API C può pilotarla.
7. **SVG come cittadino di prima classe**: icone, forme e decorazioni restano vettoriali e nitide a qualsiasi scala.

---

## 3. Non-obiettivi (per ora)

Da **non** implementare finché non sono esplicitamente richiesti:

- Un motore HTML/CSS completo o un browser embedded.
- Un linguaggio di scripting general purpose dentro `.evui` (le espressioni restano limitate, vedi §4.2).
- UI in world space (barre vita sopra i nemici): è in backlog, Fase 10.
- Rich text avanzato, editor di testo multilinea, IME completo sui mobile: backlog.
- Animazioni complesse a keyframe con timeline: per ora solo transizioni (Fase 9).

---

## 4. Decisioni architetturali

### 4.1 Scena vs UI

- Un asset scena può dichiarare i documenti UI da mostrare all'avvio:
  ```toml
  # nella definizione della scena (adattare al formato esistente)
  ui = ["ui/hud.evui"]
  ```
- A runtime esiste un **Screen Stack** per livelli, nell'ordine di disegno: `hud` → `menu` → `modal` → `debug`. API: `push`, `pop`, `replace`, `show_modal`, `close_modal`.
- **La UI non entra mai nella `Scene` dell'utente** (ADR 0001). A runtime `evolve_ui` costruisce una **scena separata e temporanea**, mai salvata nel progetto e mai visibile nella gerarchia dell'editor. I renderer esistenti (CPU e GPU) la disegnano **dopo la scena di gioco**, con una camera in unità logiche. Non ha mai coordinate nel mondo di gioco.
- L'hit testing si fa sui rettangoli del layout UI, non con il picking della scena.
- In Play mode la UI si sovrappone al viewport di gioco, sia nell'editor sia nella build.

### 4.2 Il formato `.evui` (sintassi stile JSX)

Il markup è un **formato proprio, parsato dall'engine a runtime e nell'editor**. Non è una macro Rust: deve funzionare identico qualunque sia il linguaggio di gioco.

Esempio di riferimento:

```jsx
// ui/main_menu.evui
import { Screen, VStack, Text, Button } from "evolve:ui"
import HealthBar from "./components/HealthBar.evui"

export component MainMenu() {
  <Screen padding="xl" align="center" justify="center">
    <VStack gap="md" align="center" width="320">
      <Text variant="display">Evolve Quest</Text>
      <Button variant="primary" width="fill" onPress={emit("menu.play")}>Gioca</Button>
      <Button variant="secondary" width="fill" onPress={ui.push("ui/settings.evui")}>Impostazioni</Button>
      <Button variant="ghost" width="fill" onPress={emit("menu.quit")}>Esci</Button>
    </VStack>
  </Screen>
}
```

```jsx
// ui/components/HealthBar.evui
import { HStack, Icon, ProgressBar, Text } from "evolve:ui"

export component HealthBar(value: number, max: number = 100) {
  <HStack gap="sm" align="center">
    <Icon src="icons/heart.svg" size="md" color="danger" />
    <ProgressBar
      width="200"
      value={value / max}
      variant={value < max * 0.25 ? "danger" : "success"} />
    <Text variant="caption">{format("{}/{}", value, max)}</Text>
  </HStack>
}
```

```jsx
// ui/hud.evui: uso del data store, condizioni e liste
import { Screen, HStack, VStack, Text, If, For } from "evolve:ui"
import HealthBar from "./components/HealthBar.evui"

export component Hud() {
  <Screen padding="lg">
    <HStack justify="space-between" width="fill">
      <HealthBar value={player.health} max={player.maxHealth} />
      <Text variant="title">{format("Punti: {}", game.score)}</Text>
    </HStack>
    <If cond={player.health <= 0}>
      <Text variant="display" color="danger">Game Over</Text>
    </If>
    <VStack gap="xs">
      <For each={quests.active} as="q" key={q.id}>
        <Text>{q.title}</Text>
      </For>
    </VStack>
  </Screen>
}
```

Regole del linguaggio:

- **Componenti**: `component Nome(prop: tipo = default) { <markup/> }`, con `export` per renderli importabili. I figli passati dall'esterno si inseriscono con `<Slot />`.
- **Controllo di flusso come nodi** (`<If>`, `<Else>`, `<For>`), non come espressioni JS: così compaiono nella gerarchia dell'editor visuale e restano modificabili graficamente.
- **Espressioni** tra `{}`: letterali, percorsi del data store (`player.health`), aritmetica, confronti, logica, ternario, e poche funzioni integrate (`format`, `min`, `max`, `clamp`, `len`, `round`). Niente loop, niente funzioni definite dall'utente, nessun effetto collaterale.
- **Handler di eventi** (`onPress`, `onChange`, `onHover`…): sequenze di istruzioni limitate:
  - `emit("nome.azione", payload?)` invia un'azione al gioco;
  - `ui.push(...)`, `ui.pop()`, `ui.replace(...)`, `ui.showModal(...)` per la navigazione tra schermate senza scrivere codice;
  - assegnamento a **stato locale** del componente.
- **Stato locale**: `state selectedTab: number = 0` dentro un componente, per comportamenti puramente UI come tab o pannelli espandibili, senza passare dal codice di gioco.
- **Binding bidirezionale** per i widget di input: `<Slider bind:value="settings.volume" />` aggiorna il data store ed emette anche un evento di cambio.
- **Stati visivi** con prefisso: `hover:background="primary.600"`, `pressed:scale="0.97"`, `focused:border="focus"`, `disabled:opacity="0.5"`.

Implementazione del parser:

- Lexer con `logos`, parser a discesa ricorsiva scritto a mano.
- **Albero sintattico lossless** (CST, es. crate `rowan`) che conserva commenti, spaziature e span. È indispensabile per la sincronizzazione bidirezionale con l'editor visuale (§4.10): una modifica fatta nell'inspector deve produrre una modifica testuale minima, non riscrivere il file.
- Errori con posizione precisa e messaggi chiari (es. `miette` o `ariadne`). Un errore di sintassi non deve mai far crashare editor o gioco: si mostra l'errore e si mantiene l'ultima versione valida.

### 4.3 Layout

- Usare **Taffy** (flexbox + CSS Grid + block). Non reimplementare il layout.
- Proprietà di layout esposte sui nodi: `direction`, `gap`, `padding`, `margin`, `width`, `height`, `minWidth`/`maxWidth`, `minHeight`/`maxHeight`, `grow`, `shrink`, `align`, `justify`, `wrap`, `aspectRatio`, `position="absolute"` con `top`/`right`/`bottom`/`left`, `zIndex`, `overflow` (`visible` | `clip` | `scroll`).
- Dimensionamento in stile Figma auto-layout: `width="hug"` (si adatta al contenuto), `width="fill"` (riempie), valore fisso, percentuale.
- `HStack`, `VStack`, `Grid`, `Spacer` sono semplici scorciatoie sopra `View` con direzione e default preimpostati.
- Il layout si ricalcola solo sui nodi sporchi (dirty flag), mai su tutto l'albero a ogni frame.

### 4.4 Unità e risoluzione

- Unità base: **pixel logici** (indipendenti dal DPI). Supportati anche `%`, `fr` (grid) e i token di spaziatura (`xs`, `sm`, `md`, `lg`, `xl`…).
- Impostazioni di progetto:
  - **risoluzione di riferimento** (default 1920×1080, ADR 0005);
  - **match mode**: `width`, `height`, `min`, `expand` (default `min`: scala sul rapporto minore, la UI entra sempre nello schermo);
  - **scala UI dell'utente** (moltiplicatore, utile per accessibilità e impostazioni di gioco).
- Fattore finale = scala da risoluzione di riferimento × DPI del dispositivo × scala utente.
- **Safe area** (notch, barre di sistema iOS/Android) esposta al layout: il nodo `Screen` la rispetta di default, disattivabile con `safeArea="ignore"`.

### 4.5 Tema e design system

- Un tema è un file dati (es. `themes/default.evtheme` in TOML) con:
  - **token**: palette (`primary.50`…`primary.900`, `neutral.*`, `success`, `warning`, `danger`, `focus`), scala di spaziatura, raggi, ombre, scala tipografica (`display`, `title`, `body`, `caption`, `label`), durate e curve delle transizioni;
  - **stili dei componenti**: per ogni widget, varianti e stati (`default`, `hover`, `pressed`, `focused`, `disabled`).
- I valori nei nodi possono riferirsi ai token per nome (`padding="md"`, `color="danger"`) oppure essere letterali (`padding="12"`, `color="#ff3355"`).
- Il progetto ha un tema attivo; i temi si possono cambiare a runtime (es. tema chiaro e scuro).
- **Il tema di default deve essere curato visivamente**: contrasti accessibili, spaziature coerenti, stati di hover e focus evidenti. È parte del prodotto, non un placeholder.
- Font di default inclusi nell'engine (necessario anche per il Web, dove non ci sono font di sistema): una famiglia sans moderna con licenza OFL (es. Inter) più fallback Noto per gli script non latini. Includere i file di licenza.

### 4.6 Integrazione con la logica di gioco (agnostica dal linguaggio)

Il gioco **non costruisce la UI**: la UI è dichiarata nei file `.evui`. Il gioco fa solo due cose:

1. **Scrive valori nel data store**, identificati da un percorso: `player.health = 72`, `inventory.items = [...]`.
2. **Legge le azioni** emesse dalla UI: `menu.play`, `inventory.use { id: 42 }`.

Specifiche:

- Tipi del data store: `number`, `bool`, `string`, `list`, `record`, `asset` (riferimento a immagine o SVG). Liste e record si possono impostare anche in blocco da JSON.
- **Primo linguaggio di binding: Rhai** (ADR 0005), nello stile delle funzioni esistenti (`snake_case`, nodi e valori semplici, errori in console, mai panic):
  ```text
  ui_set("player.health", 72)        // scrive nel data store
  ui_get("settings.volume")          // legge dal data store
  fn on_ui_action(name, payload) { } // callback come on_input / on_collision
  ```
  Gli script scrivono i valori e ricevono le azioni; non costruiscono mai la UI.
- API nativa Rust per l'uso interno e per i giochi in Rust.
- L'**API C** (`evolve_ui.h` via `cbindgen`) e le binding per Python/C++ sono nel backlog (Fase 10).

### 4.7 Reattività e riconciliazione

- Ogni espressione di binding registra quali percorsi del data store e quale stato locale legge (**tracciamento delle dipendenze**, stile signals). Quando un valore cambia, solo i nodi che ne dipendono vengono ricalcolati.
- Quando la struttura cambia (`If`, `For`, hot reload di un file), l'albero si **riconcilia** tramite diff con chiavi (`key`), come in React: lo stato interno dei nodi (focus, posizione di scroll, transizioni in corso, stato locale) si preserva dove possibile.
- Pipeline per frame: input → handler → aggiornamento data store → rivalutazione binding sporchi → layout dei nodi sporchi → generazione delle primitive di disegno → render.

### 4.8 Rendering: draw list neutra e backend (ADR 0001)

- `evolve_ui_core` produce una **draw list neutra** (nessun tipo del motore): rettangolo arrotondato (raggi per angolo, bordo, riempimento solido o gradiente lineare, ombra), percorso, riga di testo (`TextRun`), immagine/SVG, `PushClip`/`PopClip`.
- Un **backend** (crate `evolve_ui_scene`) converte la draw list in una scena temporanea del motore: rettangoli e percorsi diventano forme, i clip diventano gruppi con maschera, i glifi diventano istanze di symbol. È l'**unico** codice UI che conosce i tipi di scena del motore.
- Una futura pipeline wgpu (opzione B) leggerà la stessa draw list senza toccare il resto.
- **Ombre:** approssimate con forme sovrapposte e gradienti (nessun blur). Il tema di default si disegna tenendo conto di questo limite.
- **SVG:** si importano già come percorsi nativi (`io::import_svg`): un'icona è un gruppo di forme, nitida a ogni scala. Nessuna rasterizzazione di default.
- **Revisione:** la UI si integra con gli stamp di revisione del motore: una UI invariata non forza un nuovo disegno.
- **Compatibilità:** il percorso passa per i renderer esistenti, quindi funziona su CPU, GPU, player, editor e Web. Nessuna pipeline nuova.

### 4.9 Testo (ADR 0002)

- Shaping, layout e fallback dei font con **`cosmic-text`** (shaping, fallback, bidirezionalità; utile poi per `TextInput`).
- **Contorni dei glifi da `swash`**, convertiti in percorsi (in unità em) e tenuti in **cache per (font, glyph id)**; ogni glifo è riusato come istanza del suo symbol, mai ritessellato a ogni frame.
- Funzionalità minime: dimensione, peso, stile, colore, allineamento orizzontale, a capo automatico, troncamento con ellissi, interlinea, testo multilinea.
- **Rischio da misurare subito:** senza hinting il testo piccolo (12-14 px) può risultare sfocato sul renderer CPU a 1×. Criterio di accettazione della Fase 1. Se la qualità non basta: **fermarsi e proporre il ripiego**, cioè glifi rasterizzati da `swash` solo per il testo piccolo sul percorso CPU.
- MSDF o atlas solo in futuro (testo in world space o backend wgpu).

### 4.10 Organizzazione del codice (indicativa)

```
crates/
  evolve_ui_core/     draw list neutra, albero dei nodi, stile, tema, data store, reattività, layout (Taffy)
  evolve_ui_text/     font, shaping, contorni dei glifi, cache (nessun tipo del motore)
  evolve_ui_scene/    backend: draw list -> scena temporanea del motore (l'unico che dipende da evolve_engine)
  evolve_ui_markup/   lexer, parser, CST, valutatore delle espressioni, formatter
  evolve_ui_widgets/  widget base e loro comportamento
  evolve_ui_editor/   finestra UI Editor
```

`evolve_ui_core` e `evolve_ui_text` non devono dipendere né da wgpu né dai tipi di scena del motore: devono poter calcolare layout, testo e stato in modo headless per i test. `evolve_ui_ffi` (API C) è nel backlog.

---

## 5. Fasi di lavoro

### Fase 0: Ricognizione (nessun codice UI)

- [x] Mappare la struttura del workspace: crate, moduli, punti d'ingresso di editor e runtime.
- [x] Capire come funzionano oggi il render loop wgpu, i pass e la gestione delle surface.
- [x] Individuare la pipeline SVG esistente (libreria usata: `resvg`, `lyon`, custom?) e come produce texture o mesh.
- [x] Capire il formato degli asset e della scena, il sistema di asset/hot reload (se esiste), il sistema di undo/redo dell'editor.
- [x] Capire come è costruita l'interfaccia dell'editor stesso (egui? custom?) e come si aggiunge una nuova finestra o pannello.
- [x] Capire come il codice di gioco comunica oggi con l'engine (binding, FFI, scripting).
- [x] Verificare lo stato delle build Web, Android e iOS.
- [x] Individuare i punti di contatto con il lavoro sulla fisica, per evitare conflitti. (parziale: non vedo il lavoro dell'altro sviluppatore, vedi docs/ui/RECON.md §10 e D3)

**Accettazione:** un file `docs/ui/RECON.md` con i risultati, l'elenco delle parti di questo piano da adattare e le eventuali domande aperte per il team.

### Fase 1: Pipeline del testo

- [x] `evolve_ui_core`: draw list neutra (rect arrotondato, path, text run, push/pop clip), senza tipi del motore.
- [x] `evolve_ui_text`: font da memoria e da file; font predefiniti incorporati con `include_bytes!` e relative licenze (OFL).
- [x] Integrazione `cosmic-text`: shaping, fallback, a capo, allineamento, ellissi.
- [x] Contorni dei glifi da `swash` come percorsi, in cache per (font, glyph id).
- [x] Backend `evolve_ui_scene` minimo: righe di testo → scena temporanea (glifi come istanze), disegnata dai renderer esistenti.
- [x] Gestione del cambio di fattore di scala (il layout dipende dalla scala, `TextSystem::layout(.., scale)`; i contorni sono vettoriali e non vanno rigenerati).

**Accettazione:** una scena di test mostra testo in italiano con accenti, più una riga in arabo e una in giapponese tramite fallback, a 1× e 2× di scala, con i renderer CPU e GPU (e sul Web, tramite il player). In più, **testo a 12, 14 e 16 px leggibile e nitido con il renderer CPU a 1× e 2×, con immagini di confronto**. Se la qualità non basta: fermarsi e proporre il ripiego di §4.9.

**Stato dell'accettazione (2026-10-31):** italiano, arabo e giapponese a 1× e 2× su CPU ✓, su GPU (Vulkan e limiti WebGL2) ✓ con test di parità CPU↔GPU. Web: i crate compilano per `wasm32` (CI) ma **non sono stati eseguiti in un browser**. Testo a 12/14/16 px: immagini prodotte con `text_sheet`; a 14 e 16 px nitido, a 12 px a 1× leggibile ma un po' morbido (niente hinting). **Giudizio di Andrea (2026-10-31): va bene** (testo e demo della Fase 2 provati: «tutto perfetto»). Snapping attivo, ripiego con glifi rasterizzati non necessario; si riapre solo se emergono problemi.

### Fase 2: Primitive di rendering UI

- [x] Rettangoli arrotondati (anche con raggi diversi per angolo), bordi e gradiente lineare nel backend.
- [x] Ombre approssimate con forme (10 rettangoli sovrapposti che seguono il profilo gaussiano entro pochi punti percentuali; nessun blur). Regole per il tema: nessuna ombra con blur > 1000 px, una o due ombre per elemento (ognuna costa 10 forme).
- [x] Immagini e SVG come symbol (via `io::import_svg`), un'istanza per uso, con tinta opzionale; il symbol è fatto una volta per asset.
- [x] Clipping: `PushClip`/`PopClip` come gruppi con maschera, anche annidati (test CPU e parità GPU).
- [x] La scena UI temporanea si disegna dopo la scena di gioco, con vista in unità logiche (player: `PlayerApp::with_ui`; editor: Game view, CPU e GPU). Regola di scala minima: riferimento 1920×1080, rapporto minore.
- [x] Integrazione con gli stamp di revisione: `UiLayer` non tocca la scena se la draw list è uguale; test dell'editor: 5 frame fermi = 0 disegni GPU.

**Accettazione:** una demo statica disegna card con ombre, bordi, icone SVG e testo, sopra una scena in esecuzione, con renderer CPU e GPU.

### Fase 3: Albero dei nodi, layout, risoluzione

- [x] Albero dei nodi in `evolve_ui_core` (`UiTree`, `Style` con le parole del piano: hug, fill, gap, align, justify, grow, absolute, z-index, overflow, grid) con stile Taffy calcolato per nodo.
- [x] Integrazione Taffy con dirty flag e ricalcolo incrementale (un albero invariato non si ricalcola né si ridisegna; cambiare un testo misura solo quel testo; `TreeStats` lo rende visibile).
- [x] Unità logiche, risoluzione di riferimento, match mode (`width`/`height`/`min`/`expand`), scala utente (`ScreenMetrics`). Il DPI è già nei pixel fisici: nessun fattore a parte (vedi `units.rs`).
- [x] Safe area esposta al layout (`ScreenMetrics::safe_area`) e rispettata da `Style::screen()` (disattivabile con `safe_area: false`).
- [x] Screen Stack con livelli (`hud`, `menu`, `modal`, `debug`): `push`, `pop`, `replace`, `show_modal`, `close_modal`; hud e menu mostrano la schermata in cima, modal e debug tutte.
- [ ] Integrazione con la scena: lista di documenti UI iniziali. **Rimandata alla Fase 4** (serve il formato v4 e i file `.evui`; ADR 0003): senza documenti non c'è nulla da elencare.

**Accettazione:** test headless di layout (snapshot dei rettangoli calcolati) per HStack, VStack, Grid, padding, gap, hug/fill, assoluto. La stessa UI di prova appare proporzionata a 1280×720, 1920×1080, 2560×1080 e su un viewport verticale da telefono.

**Stato (2026-10-31):** 55 test headless in `evolve_ui_core` (compositore di testo finto a metriche fisse, rettangoli calcolati a mano) e 4 sulla demo, che ora è costruita dal layout e non ha più coordinate; le quattro schermate sono in `local/ui_sheets/ui_demo_*.png`.

### Fase 4: Linguaggio `.evui` e anteprima

- [ ] Lexer (`logos`) e parser con CST lossless.
- [ ] Componenti, props con default, `import`/`export`, `<Slot />`.
- [ ] Valutatore delle espressioni (sandbox, nessun effetto collaterale).
- [ ] Nodi `<If>`, `<Else>`, `<For>` con `key`.
- [ ] Formatter canonico (stile `prettier`), così i file restano uniformi.
- [ ] Errori con posizione e messaggio chiaro; in caso di errore si mantiene l'ultima versione valida.
- [ ] **Hot reload**: salvare un file `.evui` aggiorna la UI in esecuzione preservando lo stato.
- [ ] **Finestra "UI Preview"** nell'editor, in sola lettura: apre un `.evui` e lo mostra, con selezione del dispositivo/risoluzione e **safe area simulata** (ADR 0006). Serve a vedere i risultati presto, prima dell'editor completo della Fase 8.

**Accettazione:** i tre file di esempio della §4.2 si caricano e si visualizzano (con valori finti nel data store); `parse → print` restituisce il file identico (test di round-trip su un corpus di file).

### Fase 5: Input ed eventi

- [ ] Hit testing rispettando clipping e `zIndex`.
- [ ] Eventi: press, release, click, hover enter/leave, drag, scroll (rotella e touch).
- [ ] Bubbling e cattura del puntatore; la UI consuma l'input in modo che il gioco non lo riceva due volte.
- [ ] Stati visivi `hover`, `pressed`, `focused`, `disabled` con i prefissi del markup.
- [ ] Touch multipunto di base per mobile.

**Accettazione:** cliccare un bottone sopra il viewport non attiva l'input di gioco sottostante; gli stati visivi cambiano correttamente con mouse e touch.

### Fase 6: Tema e widget base

- [ ] Caricamento del tema, risoluzione dei token, cambio tema a runtime.
- [ ] Tema di default curato (vedi §4.5).
- [ ] Widget: `Screen`, `View`, `HStack`, `VStack`, `Grid`, `Spacer`, `Text`, `Icon` (SVG), `Image`, `Button`, `IconButton`, `Toggle`, `Checkbox`, `Slider`, `ProgressBar`, `ScrollView`, `Panel`/`Card`, `Divider`, `Modal`.
- [ ] Varianti per i widget principali (`primary`, `secondary`, `ghost`, `danger`) e taglie (`sm`, `md`, `lg`).
- [ ] Pagina di "galleria" `.evui` che mostra tutti i widget in tutte le varianti e stati.

**Accettazione:** la galleria è visivamente coerente in tema chiaro e scuro, senza alcuno stile personalizzato.

### Fase 7: Data store, reattività, API per il gioco

- [ ] Data store con percorsi tipizzati e impostazione da JSON.
- [ ] Tracciamento delle dipendenze e aggiornamento solo dei nodi interessati.
- [ ] Riconciliazione con chiavi che preserva lo stato dei nodi.
- [ ] Stato locale nei componenti (`state`) e handler (`emit`, `ui.*`, assegnamenti).
- [ ] Binding bidirezionale `bind:value`.
- [ ] Coda delle azioni verso il gioco.
- [ ] API Rust e funzioni Rhai `ui_set`, `ui_get` e callback `on_ui_action` (ADR 0005). L'API C è nel backlog.

**Accettazione:** la **slice verticale** (§6) funziona su desktop e Web.

### Fase 8: UI Editor (finestra dedicata)

- [ ] Nuova finestra/tab **UI Editor** che apre i file `.evui`, distinta dall'editor di scena.
- [ ] **Canvas centrale** con cornici dispositivo (preset telefono, tablet, 16:9, 21:9, personalizzato), rotazione, overlay della safe area, zoom e pan.
- [ ] **Gerarchia** dei nodi (inclusi `If`/`For`/componenti) con drag & drop per riordinare e annidare.
- [ ] **Palette** di widget e componenti del progetto da trascinare nel canvas o nella gerarchia.
- [ ] **Inspector stile auto-layout di Figma**: direzione, gap, box visuale del padding, griglia 3×3 per l'allineamento, dimensionamento hug/fill/fisso, selettori dei token di tema.
- [ ] Anteprima degli stati (forza `hover`, `pressed`, `disabled` su un nodo selezionato).
- [ ] **Pannello "dati di prova"**: valori finti per i percorsi usati nel documento (es. `player.health = 30`) per vedere la UI in ogni situazione senza avviare il gioco.
- [ ] **Vista divisa codice/visuale sincronizzata**: le modifiche nell'inspector diventano modifiche testuali minime sul CST, e viceversa.
- [ ] Guide, righelli, snapping alla scala di spaziatura.
- [ ] Integrazione con undo/redo dell'editor.
- [ ] Collegamento al Play mode: ispezionare la UI viva del gioco in esecuzione (albero, valori del data store, azioni emesse).

**Accettazione:** si può ricostruire da zero il menu principale della slice verticale usando solo l'editor visuale; il file risultante è pulito e leggibile, e una modifica a mano nel codice si riflette subito nel canvas.

### Fase 9: Navigazione e animazioni

- [ ] Focus e navigazione con tastiera e gamepad (focus graph automatico basato sul layout, sovrascrivibile con props).
- [ ] Indicatore di focus visibile dal tema.
- [ ] Transizioni di proprietà (colore, opacità, scala, traslazione, dimensioni) con durate e curve dei token.
- [ ] Transizioni di ingresso e uscita per le schermate nello Screen Stack.

**Accettazione:** il menu della slice verticale è completamente utilizzabile con un gamepad; le transizioni non causano ricalcoli di layout a ogni frame quando non serve.

### Fase 10: Backlog (dopo, in ordine indicativo)

- [ ] `TextInput` con selezione, copia/incolla, IME e tastiera virtuale su mobile.
- [ ] Localizzazione con `fluent` e testo da destra a sinistra.
- [ ] Varianti responsive per orientamento e breakpoint (layout diversi per telefono verticale e desktop).
- [ ] Accessibilità con `AccessKit` (screen reader, navigazione).
- [ ] UI in world space (barre vita, nomi sopra i personaggi), eventualmente con testo MSDF.
- [ ] Liste virtualizzate per inventari molto lunghi.
- [ ] Rich text (grassetto, colori, icone inline nel testo).
- [ ] Editor di temi visuale.
- [ ] API C (`evolve_ui.h` via `cbindgen`) e binding ufficiali per Python e C++.
- [ ] Backend wgpu (opzione B) che legge la draw list, se servisse più velocità.

---

## 6. Slice verticale di riferimento

Prima milestone completa, da raggiungere alla fine della Fase 7:

1. All'avvio la scena mostra `main_menu.evui` con titolo e tre bottoni.
2. "Gioca" emette `menu.play`; il gioco sostituisce il menu con `hud.evui`.
3. L'HUD mostra `HealthBar` collegata a `player.health`, che il gioco aggiorna nel tempo; sotto il 25% la barra diventa rossa.
4. "Impostazioni" apre una schermata con uno `Slider` collegato a `settings.volume` e un bottone "Indietro" (`ui.pop()`), senza codice di gioco.
5. A `player.health <= 0` appare "Game Over".
6. Tutto funziona su desktop e Web, e risulta proporzionato su risoluzioni diverse.

---

## 7. Test e qualità

- **Layout**: test headless con snapshot dei rettangoli calcolati.
- **Parser**: round-trip `parse → print` identico, test sugli errori (messaggio e posizione attesi), fuzzing di base.
- **Espressioni**: test unitari del valutatore, inclusi casi di errore (percorso mancante, tipo sbagliato) che non devono mai mandare in panic.
- **Rendering**: golden image con tolleranza, render offscreen con wgpu.
- **Piattaforme**: in CI almeno la compilazione per `wasm32`, Android e iOS; esecuzione della slice verticale su Web.
- **Prestazioni**: un HUD tipico non deve ricalcolare il layout a ogni frame se nessun valore è cambiato; misurare il tempo per frame del sistema UI e tenerlo visibile nel pannello debug.

---

## 8. Regole di lavoro per l'agente

- Passi piccoli e coerenti, una funzionalità per volta, ciascuno con i suoi test. **Git lo gestisce Andrea:** nessun commit, branch o push da parte dell'agente; alla fine di ogni passo si scrive un breve riepilogo di cosa è cambiato, così lui può fare il commit.
- Alla fine di ogni fase: demo eseguibile, test verdi, caselle aggiornate in questo documento.
- Non aggiungere dipendenze pesanti senza motivazione scritta nel commit o in un ADR.
- Mantenere la compatibilità con tutti i renderer e con il Web: la UI passa solo per i renderer esistenti. Per ogni test che apre la GPU vale GPU_SAFETY.md.
- Non modificare il modulo di fisica; per le modifiche a file condivisi del motore, annotarle nel riepilogo del passo.
- Documentare l'API pubblica (Rust e C) e il formato `.evui` in `docs/ui/`, con esempi.
- Se un requisito è ambiguo, fermarsi e chiedere invece di indovinare.

---

## 9. Decisioni prese (2026-10-31)

Registrate in [docs/adr/](docs/adr/):

- **D1** UI come scena vettoriale separata, con draw list neutra → ADR 0001.
- **D2** Testo come percorsi vettoriali, cache per (font, glyph) → ADR 0002.
- **D3** Fisica in parallelo: si va avanti, l'unione la fa Andrea → ADR 0004.
- **D4** Cartella di progetto per lo sviluppo, file unico per l'esportazione → ADR 0003.
- **D5** Binding Rhai (`ui_set`, `ui_get`, `on_ui_action`); 1920×1080, match `min`; `.evui`, `.evtheme`, `evolve:ui` → ADR 0005.
- **D6, D7** Editor su egui; mobile fuori piano, safe area simulata nell'anteprima → ADR 0006.

Domande ancora aperte: nessuna al momento.
