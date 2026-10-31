# ADR 0002 — Il testo è disegnato come percorsi vettoriali

- Stato: accettata
- Data: 2026-10-31
- Decide: Andrea (D2)

## Contesto

Il piano prevedeva un glyph atlas su texture GPU. Con l'ADR 0001 la UI passa per i renderer vettoriali esistenti, e il renderer CPU non ha
pipeline di texture per i glifi. Il motore è vettoriale fin dall'inizio.

## Decisione

- `cosmic-text` per shaping, fallback dei font, a capo e layout.
- I contorni dei glifi vengono da `swash` e sono convertiti in `Path`.
- **Cache dei glifi per (font, glyph id)**, in unità em, riusati come istanze (symbol/`Instance` del motore dove conviene), mai ritessellati a ogni frame.
- La draw list porta *righe di testo* (`TextRun`), non glifi già tessellati: così un futuro backend wgpu può usare un atlas.

## Criterio di accettazione aggiunto alla Fase 1

Testo a 12, 14 e 16 px leggibile e nitido con il renderer CPU a scala 1× e 2×, con immagini di confronto. Il testo piccolo senza hinting può
risultare sfocato.

**Ripiego, se la qualità non basta:** fermarsi e proporlo; il renderer CPU userebbe glifi rasterizzati da `swash` solo per il testo piccolo,
mentre tutto il resto resta vettoriale.
