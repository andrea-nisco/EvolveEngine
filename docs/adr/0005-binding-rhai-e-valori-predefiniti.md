# ADR 0005 — Binding con Rhai, risoluzione di riferimento e nomi

- Stato: accettata
- Data: 2026-10-31
- Decide: Andrea (D5)

## Decisione

- **Binding:** il primo linguaggio è Rhai, perché è l'unico che esiste. Nello stile delle funzioni esistenti: `ui_set(path, valore)`,
  `ui_get(path)` e una callback `on_ui_action(name, payload)` coerente con `on_input` e `on_collision`. L'API C (`evolve_ui.h`, `cbindgen`) passa nel backlog.
- **Risoluzione di riferimento:** 1920×1080. Match mode predefinito: scala sul rapporto minore (`min`), così la UI entra sempre nello schermo.
- **Nomi:** `.evui`, `.evtheme` e modulo standard `evolve:ui`.

## Conseguenze sul piano

La Fase 7 non crea più `evolve_ui_ffi` né l'esempio in C; espone invece le funzioni Rhai. §4.6 e §9 sono aggiornati.
