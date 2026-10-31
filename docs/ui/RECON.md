# Evolve UI & Testo — Ricognizione (Fase 0)

> Risultato della Fase 0 di [EVOLVE_UI_PLAN.md](../../EVOLVE_UI_PLAN.md). Data: 2026-10-31. Nessun codice UI è stato scritto.
> Ogni affermazione viene dalla lettura del codice o dei file di configurazione del repository; dove non ho potuto verificare lo dico.

## 1. In sintesi: cosa cambia rispetto alle ipotesi del piano

Il piano parte da cinque presupposti che il codice **non** conferma. I primi due cambiano l'architettura del renderer e vanno decisi prima della Fase 1.

| # | Il piano assume | Nel codice |
|---|---|---|
| 1 | La UI si disegna in "un pass dedicato dopo il pass di scena" (§4.1, §4.8) | Il player usa solo il renderer CPU (tiny-skia): rasterizza in un `Pixmap` e lo mostra come texture egui. Il renderer wgpu esiste solo nell'editor, è opzionale (default CPU) e disegna in una texture mostrata da egui con un *paint callback*. Nessuno dei due possiede una surface o un frame: li possiede eframe/egui. |
| 2 | Un renderer UI con pipeline `ui_quad`/`ui_glyph`/`ui_image` su wgpu | Un secondo renderer GPU nel solo player non esisterebbe senza portare wgpu nel player (oggi eframe con `glow`, scelto apposta per restare piccolo e girare nel browser). Servirebbe anche un'implementazione CPU, altrimenti la UI sparirebbe con il renderer CPU. |
| 3 | SVG: "riusa la pipeline SVG, rasterizza e metti in cache per dimensione" (§4.8) | Gli SVG **non sono rasterizzati**: `usvg` li converte in `Path` nativi (Bézier) del motore, che entrambi i renderer disegnano come qualsiasi forma. Sono già nitidi a ogni scala. `usvg` è compilato senza feature (`default-features = false`), quindi il testo dentro gli SVG non è supportato. |
| 4 | Esiste un sistema di asset e di hot reload | No. Un progetto è **un solo file JSON** `.evolve` con le scene dentro. Non ci sono file di asset esterni, né ricaricamento a caldo. |
| 5 | Il gioco comunica con l'engine via binding/FFI | Il gioco è scritto in **Rhai** (script per nodo, dentro il motore). Non esiste API C. |

Il resto del piano (Taffy, `.evui`, data store, tema, editor dedicato) è compatibile, con gli adattamenti elencati in §8.

## 2. Workspace

`Cargo.toml` radice: membri `evolve_engine`, `evolve_editor`, `evolve_player`, `evolve_gpu`, `tools/web-canary`.

| Crate | Ruolo | Dipendenze principali |
|---|---|---|
| `evolve_engine` | Core: scena, percorsi Bézier, stili, camera, render CPU (tiny-skia), progetto/IO, fisica, runtime, scripting Rhai, animazione, rig | glam, rhai, tiny-skia, usvg, i_overlay, serde |
| `evolve_gpu` | Renderer wgpu: tessellazione (lyon), draw list, layer, cache; test headless e confronto con il CPU. Compila anche per il browser | wgpu 30.0.1, lyon, bytemuck; feature `egui` → egui-wgpu 0.36.2 |
| `evolve_player` (lib `evolve_player_core`) | Player desktop/web e pacchettizzazione (`bundle`) | eframe 0.36.2 con `glow` (non wgpu) |
| `evolve_editor` | Editor (egui + egui_dock, layout in RON) | eframe, egui_dock 0.21.1, rfd, ron |
| `tools/web-canary` | Pagina di test del renderer GPU su WebGL2/WebGPU | wgpu (webgl, webgpu) |

Punti d'ingresso: editor `crates/evolve_editor/src/main.rs` (stato in `app.rs`, `EditorApp`); player `crates/evolve_player/src/main.rs` e `app.rs` (`PlayerApp`); runtime `evolve_engine::runtime::Runtime`, usato sia dal Play mode dell'editor sia dal player.

Non esistono né la cartella `docs/` (creata ora con questo file) né `docs/adr/`. Esistono già molti documenti nella radice (piani, CHANGELOG, MANUAL_TESTS, OPEN_POINTS): conviene seguire quella convenzione per i registri di avanzamento.

## 3. Rendering

**Render loop.** Entrambi i renderer disegnano "una scena con una matrice di vista" in una destinazione di dimensione data:
- CPU: `evolve_engine::render::render(&scene, view, &mut pixmap)`. Il player lo chiama ogni frame, poi carica il pixmap come texture egui (`player/src/app.rs`).
- GPU: `evolve_gpu::renderer::Renderer::render(scene, view, size, options)` registra più pass offscreen (uno per ogni layer con opacità/clip, l'ultimo "scene") con MSAA, depth e stencil per le maschere. Lo mostra `egui_glue.rs` (`GpuContext`, `GpuView`) con un callback `egui_wgpu`. Il renderer "non sa nulla di egui".
- Un frame invariato non viene ridisegnato (stamp di revisione); molto lavoro è stato fatto per questo (GPU_RENDERER_PLAN fasi 1-5).

**Scelta del renderer.** `renderers.rs`: `Auto | Cpu | Gpu`, default **Cpu** finché non si chiude la Fase 7 del piano GPU (P5 in OPEN_POINTS). Se la GPU non può disegnare la scena o il device si perde, l'editor ricade sul CPU con un avviso. Il player non ha scelta: solo CPU.

**Vincoli già rispettati da `evolve_gpu`**, utili alla UI: nessun compute shader, nessuno storage buffer (modalità di test `WebGl2` con i limiti WebGL2); `GPU_SAFETY.md` è vincolante per ogni lavoro GPU (errori registrati, budget di memoria, test limitati a 1 minuto, nessun processo GPU lasciato aperto).

**Conseguenza per il piano:** disegnare la UI "dopo la scena" è facile per il renderer GPU (basta aggiungere un pass al suo encoder) ma lascia fuori il CPU e il player. Vedi domanda D1.

## 4. Pipeline SVG

`evolve_engine::io::import_svg` → `usvg::Tree` → nodi `Group`/`Shape` con `Path` (anchor e maniglie Bézier), `Paint` solido o gradiente lineare/radiale, `Stroke`. Il risultato vive nella scena come qualsiasi altra forma ed è disegnato dal CPU (tiny-skia) o dal GPU (tessellazione lyon per livello di zoom, in cache).

Limiti noti (OPEN_POINTS P10): dash diventano linee piene; clip path, maschere, filtri e pattern (`Paint::Pattern` → `None`) sono scartati senza avviso; il testo SVG non è supportato.

Per la UI: un'icona SVG si può già rappresentare come `Group` di `Shape`, senza cache di raster. È una semplificazione importante rispetto a §4.8, ma costa un numero di forme per icona (tessellazione) invece di un solo quad texturizzato.

## 5. Testo

Non c'è nessun supporto al testo nel motore: nessun tipo "text", nessun font, nessuna libreria (`cosmic-text`, `swash`, `fontdb` non compaiono). Il testo dell'editor è quello di egui/epaint. Il testo dentro agli SVG viene scartato (`usvg` senza feature).

La scelta della libreria testo è già un punto aperto di GPU_RENDERER_PLAN (Q5, OPEN_POINTS P6 "decidere in 10.1"). Questo piano la anticipa: `cosmic-text` + `swash`, come nel §4.9.

Implicazione di design da decidere (D2): il motore è "vector-first" e ha due renderer. Un glyph atlas (§4.9) funziona solo dove c'è una pipeline di texture; i contorni dei glifi (`swash` li fornisce come percorsi) possono invece diventare `Path` e passare per entrambi i renderer, nitidi a qualsiasi scala, ma con più triangoli per il testo lungo.

## 6. Formato di scena, asset, undo, editor

**Scena.** `Scene { name, nodes: Vec<Node>, symbols, clips, ik, ... }`; `Node` ha `NodeKind = Group | Shape | Instance | Camera`, trasformazione, opacità, `body` (fisica), `script` (testo Rhai). Un `Project` è un elenco ordinato di scene più l'indice della scena iniziale. Formato `.evolve`: JSON, `FORMAT_VERSION = 3` (`io.rs`).

**Regole di stabilità del formato** (CONTRIBUTING.md, "Changing the project file format"): ogni modifica a ciò che l'editor scrive, anche un solo campo, è una nuova versione; serve un fixture congelato per versione (`crates/evolve_engine/tests/fixtures`, test `format_compat.rs`) e una riga in README/CHANGELOG. Aggiungere `ui = ["ui/hud.evui"]` alla scena (§4.1) è quindi un cambio di versione (3 → 4).

**Asset.** Non c'è un asset system generale. Le uniche "risorse" sono i *symbol* (prefab) dentro la scena. Il pacchetto di esportazione (`bundle.rs`) aggiunge **solo il JSON del progetto** in coda all'eseguibile (desktop), in `Contents/Resources/game.evolve` (macOS) o nel `index.html` (web). I file `.evui`, `.evtheme` e i font dovranno quindi essere incorporati nel pacchetto: o dentro il JSON, o in un contenitore nuovo. Non esiste hot reload.

**Undo/redo.** `history.rs`: istantanee dell'intera `Scene`, rilevate tramite gli stamp di revisione (`revision.rs`), massimo 100 passi, un solo passo per un trascinamento. Funziona per ciò che sta dentro `Scene`. I documenti `.evui` sono file di testo esterni: avranno bisogno di una cronologia propria (testo/CST), da agganciare ai comandi Annulla/Ripeti dell'editor (Fase 8).

**Editor.** È egui (eframe 0.36.2) con pannelli ancorabili in `egui_dock`. Un pannello nuovo = una variante in `enum Pane` (`layout.rs`; ora 8 varianti, con `Pane::ALL: [Pane; 8]` e `title()` da aggiornare), il codice del pannello come `impl EditorApp` in un file nuovo, e la voce nel menu *Window* (`panels.rs`). Il layout è salvato in RON; un layout non leggibile ricade sul predefinito, quindi aggiungere una variante non rompe i layout esistenti. Tema scuro in `theme.rs`.

L'editor non verrà costruito con il sistema UI (domanda aperta §9 del piano, D6).

## 7. Come il gioco parla oggi con l'engine

Script **Rhai** per nodo (`Node.script`), eseguiti dal `Runtime` con callback `on_init(me)`, `on_update(me, dt)`, `on_input(me, key)`, `on_collision`, `on_trigger`. I nodi sono interi (`i64`), le funzioni in `snake_case`, gli errori finiscono nella console di gioco (`LogLine`), non fanno panic. Esiste già uno stato globale tra scene: `set_global("score", 10)` / `get_global("score")`.

Non c'è API C né binding per altri linguaggi. Di conseguenza, per ora, "il gioco" sono gli script Rhai; l'API C del §4.6 è nuova e il suo stile (nomi, ownership delle stringhe, gestione errori) non ha un precedente nel repository da allineare. Un punto di partenza naturale: il data store potrebbe esporre a Rhai `ui_set(path, valore)` e `ui_poll_action()`, nello stile delle funzioni esistenti, prima di costruire l'FFI.

Input: `runtime::Input` (tasti, mouse nel mondo, rotella). Per far "consumare" l'input alla UI (Fase 5) il punto di contatto è la costruzione di `Input` in `evolve_player_core::input::collect_input_over` e nel Game view dell'editor (`game_view.rs`, `game.rs`).

## 8. Piattaforme

| Piattaforma | Stato verificato nel repository |
|---|---|
| Windows, macOS, Linux | Build e test in CI (`ci.yml`); release con `release.yml` |
| Web | CI compila il player (`wasm32`) e controlla `evolve_gpu` + `web_canary` per il browser. Il player web usa `glow` (WebGL) e il renderer CPU; il renderer GPU è provato nel browser solo dal *web canary* |
| Android, iOS | **Nessuna** build, CI o target. Citati solo come "wrapper" futuri in TODO.md e come Fase 11 di GPU_RENDERER_PLAN (11.3: Android Vulkan/GLES, iOS Metal). Safe area, touch e tastiera virtuale del piano non hanno oggi dove appoggiarsi |

Il test in CI per `wasm32` richiesto dal piano (§7) esiste già per player e GPU; Android e iOS andranno aggiunti con il primo lavoro di porting, non con questo piano.

## 9. Test e CI oggi

~565 test (`#[test]`) nel workspace. Strumenti che la UI può riusare:
- confronto immagini CPU↔GPU con tolleranza (`evolve_gpu::compare`, `picture_tests.rs`), device senza finestra e modalità `WebGl2` (`Gpu::for_each_test_mode`, `EVOLVE_REQUIRE_GPU`). Sono i golden test del §7 già pronti.
- `gpu-required` su Linux con Vulkan software (lavapipe); su Windows il runner ha solo WARP (nota del 2026-10-28: soglie diverse per adapter software).
- clippy con `-D warnings`; `cargo test --workspace` con timeout 10 minuti.

Regole da rispettare: GPU_SAFETY.md per ogni test che apre la GPU; clippy pulito; i test GPU non devono superare 1 minuto.

## 10. Punti di contatto con la fisica

`physics.rs` è dentro `evolve_engine` (505 righe) e la usano `scene.rs` (campo `Node.body`), `runtime.rs` (simulazione), `revision.rs` e i test. Il repository ha un solo autore nei commit (`andrea nisco`), quindi non vedo rami o file dell'altro sviluppatore: **non posso verificare dove sta lavorando**. I file condivisi in cui il lavoro UI potrebbe incrociarlo sono `scene.rs` (se si aggiunge un campo `ui` alla scena), `runtime.rs` (ciclo `update`, input) e `io.rs` (versione del formato). Proposta: tutto il codice UI in crate nuovi; toccare quei tre file solo con un accordo preventivo.

## 11. Parti del piano da adattare

1. **§4.1/§4.8 pass di UI:** dipende dalla risposta a D1.
2. **§4.8 `ui_image`:** gli SVG sono già percorsi vettoriali; la rasterizzazione con cache è un'ottimizzazione facoltativa, non il percorso predefinito.
3. **§4.9 glyph atlas:** da riconsiderare se la UI deve funzionare sul renderer CPU (D2).
4. **§4.1 `ui = [...]` nella scena:** bump `FORMAT_VERSION` 3 → 4, fixture e documentazione (CONTRIBUTING).
5. **§4.2 import / risoluzione dei file:** serve un modello di asset (cartella di progetto o contenitore) che oggi non esiste; il bundle di esportazione va esteso (D4).
6. **§4.6 API C:** nessun precedente da allineare; partire dall'API Rhai.
7. **§4.10 crate:** `evolve_ui_core` senza wgpu è già coerente con `evolve_engine`, che non dipende da wgpu. Nome crate/lib: `evolve_player` esporta `evolve_player_core`; evitare ambiguità simili.
8. **§8 "commit e PR piccoli":** in questo progetto l'uso di git è del proprietario (vedi note di lavoro): nessun commit da parte mia, solo modifiche ai file e checklist aggiornate.
9. **§7 Rendering/golden:** si può appoggiare a `evolve_gpu::compare`, ma per il percorso CPU servono golden separati (tiny-skia).
10. **Fase 4 "UI Preview":** un pannello in sola lettura è fattibile con la sezione §6 (nuovo `Pane`).

## 12. Domande aperte per il team

> **Risolte il 2026-10-31:** tutte le domande qui sotto hanno una risposta, registrata negli ADR 0001-0006 (`docs/adr/`). D1 = opzione A con draw list neutra; D2 = percorsi vettoriali; D3 = si procede in autonomia; D4 = cartella di progetto per lo sviluppo, file unico per l'esportazione; D5 = Rhai, 1920×1080, `min`; D6, D7 = editor su egui, mobile fuori piano. Il testo originale resta come storia.

- **D1 (blocca Fase 1-2). Dove e con quale renderer si disegna la UI?**
  - **A. Come nodi della scena:** la UI produce forme vettoriali (rettangoli arrotondati, glifi come percorsi, icone SVG) disegnate dai due renderer esistenti, in un livello sopra la scena con una camera propria in unità logiche. Funziona identica su CPU, GPU, player ed editor, sul Web e sui mobile futuri, senza nuove pipeline. Costi: ombre sfocate e SDF vanno approssimate con forme/gradienti; molto testo = molti triangoli.
  - **B. Pipeline wgpu dedicata (come nel piano):** massima qualità e velocità, ma serve portare wgpu nel player (oggi glow), mantenere un secondo percorso per il CPU, o rinunciare alla UI col renderer CPU.
  - **C. Via egui:** il player ha già egui; disegnare la UI di gioco con epaint sarebbe rapido, ma lega il gioco a egui e al suo testo, contro il principio di "agnostico".
  - *Raccomandazione:* partire da **A** per avere subito lo slice verticale su tutte le piattaforme e misurare; B resta un'ottimizzazione successiva e isolata dentro `evolve_ui_render`.
- **D2. Testo come glifi in atlas (§4.9) o come percorsi vettoriali?** Dipende da D1: con A, percorsi.
- **D3. Dove vive la fisica dell'altro sviluppatore e quali file sta toccando?** Per concordare i punti di §10.
- **D4. Modello di asset e di esportazione.** Un progetto sarà ancora un solo `.evolve` con dentro `.evui`/`.evtheme`/font (semplice, già compatibile con bundle e web), oppure una cartella di progetto? Cambia come funziona l'hot reload (Fase 4).
- **D5. Le domande già elencate nel §9 del piano:** lingua per le prime binding; risoluzione di riferimento e match mode predefiniti; nomi finali delle estensioni e del modulo `evolve:ui`.
- **D6. Il dogfooding dell'editor** (editor costruito col sistema UI) o egui per sempre? Il piano lo lascia aperto; oggi non cambia nulla per le fasi 1-8.
- **D7. Mobile:** conferma che Android/iOS restano fuori da questo piano (nessun target oggi) e che safe area e touch si progettano ma si provano solo su desktop/Web.
