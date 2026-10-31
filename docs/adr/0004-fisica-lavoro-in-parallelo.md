# ADR 0004 — Lavoro in parallelo sul modulo fisica

- Stato: accettata
- Data: 2026-10-31
- Decide: Andrea (D3)

## Contesto

Un altro sviluppatore lavora sulla fisica, ma non ha ancora pubblicato nulla: non si può sapere quali file tocca. I file condivisi in cui la UI
potrebbe incrociarlo sono `scene.rs`, `runtime.rs` e `io.rs` (vedi RECON §10).

## Decisione

Si procede come se non ci fosse un lavoro in parallelo. L'unione del codice sarà fatta da Andrea in un secondo momento, risolvendo gli eventuali conflitti.

Per ridurre comunque i conflitti, il codice UI sta in crate nuovi e le modifiche a file esistenti del motore sono piccole e mirate; ciascuna è
annotata nel riepilogo del passo in cui avviene, così Andrea sa dove guardare al momento dell'unione.
