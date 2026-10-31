# ADR 0001 — La UI è disegnata come scena vettoriale separata (opzione A)

- Stato: accettata
- Data: 2026-10-31
- Decide: Andrea (D1 di [docs/ui/RECON.md](../ui/RECON.md) §12)

## Contesto

Il piano ipotizzava un renderer UI dedicato su wgpu (pipeline `ui_quad`, `ui_glyph`, `ui_image`) in un pass dopo la scena. Nel codice il
player usa solo il renderer CPU (tiny-skia, con egui/`glow`); il renderer wgpu esiste solo nell'editor ed è opzionale. Un pass wgpu avrebbe
lasciato fuori player, renderer CPU e Web, o richiesto di portare wgpu nel player.

## Decisione

Si sceglie l'**opzione A**: la UI diventa forme vettoriali disegnate dai due renderer esistenti. Con due vincoli.

1. **La UI non entra mai nella `Scene` dell'utente.** `evolve_ui` costruisce a runtime una scena separata e temporanea. Non viene salvata nel
   progetto e non compare nella gerarchia dell'editor. I renderer esistenti la disegnano dopo la scena di gioco, con una camera in unità
   logiche. L'hit testing si fa sui rettangoli del layout UI, non con il picking della scena.
2. **Strato intermedio obbligatorio.** `evolve_ui_core` produce una *draw list neutra*: rettangolo arrotondato, percorso, riga di testo,
   inizio e fine clip. Un modulo *backend* la converte in forme del motore. Nessun altro codice UI dipende dai tipi di scena del motore. Una
   futura pipeline wgpu (opzione B) leggerà la stessa draw list.

Altre conseguenze:
- La UI si integra con gli stamp di revisione: una UI invariata non forza un nuovo disegno.
- Le ombre si approssimano con forme e gradienti; il tema di default si disegna tenendo conto di questo limite.

## Alternative scartate

- **B, pipeline wgpu dedicata:** richiede wgpu nel player (oggi `glow`) e un secondo percorso per il CPU. Resta possibile in futuro grazie alla draw list.
- **C, UI con egui:** lega il gioco a egui, contro il principio di indipendenza dal linguaggio.

## Conseguenze sul piano

§4.1 e §4.8 e le Fasi 1-2 sono riscritte (vedi EVOLVE_UI_PLAN.md).
