# ADR 0003 — Cartella di progetto per lo sviluppo, file unico per l'esportazione

- Stato: accettata (da realizzare nelle Fasi 3-4)
- Data: 2026-10-31
- Decide: Andrea (D4)

## Contesto

Oggi un progetto è un solo file JSON `.evolve` e l'esportazione incorpora solo quello. Mettere `.evui`, temi e font dentro il JSON renderebbe
i file illeggibili e non versionabili su Git, impossibili da aprire in un editor esterno, e toglierebbe senso all'hot reload. I font
finirebbero in base64 dentro un JSON.

## Decisione

- **Sviluppo:** il progetto è una cartella con il `.evolve` più `ui/`, `themes/` e `fonts/`. I percorsi nei file sono relativi alla cartella del progetto.
- **Compatibilità:** un `.evolve` senza cartella continua ad aprirsi come oggi.
- **Esportazione:** il bundle impacchetta progetto e risorse in un unico contenitore (desktop, macOS, web), come oggi fa con il solo JSON.
- **Non serve per la Fase 1:** i font predefiniti si incorporano nell'engine con `include_bytes!`.
- Si realizza quando serve (Fase 3/4), insieme al passaggio del formato da v3 a v4 secondo le regole di CONTRIBUTING.md (fixture congelato per versione).
