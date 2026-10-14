> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-09-30-android-escucha-avisos-con-la-pantalla-apagada.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## ADR-NNNN · En Android, los avisos con la pantalla apagada llegan porque la app se queda a la escucha, sin push — 2026-09-30
- **Estado:** aceptada
- **Contexto:** hub#2307. Los tres avisos del shell (comanda, citas, campana) nacen en la página del
  WebView: el socket de eventos y el sondeo de la campana. Con la pantalla apagada o la app en
  segundo plano no llegaban. Medido en el emulador (Pixel Tablet, API 37): sin nada, el sistema
  **congela el proceso**; con un servicio en primer plano el proceso vive, pero **Chromium congela
  la página oculta al minuto** (evento `freeze` a los 60 s, con o sin Doze, con o sin socket
  abierto), porque `WryActivity.onPause` pausa el WebView y la ventana pasa a `GONE`. Decisión de
  Ioan del 30/09: de momento **sin FCM** (ni Firebase, ni tokens, ni envío desde el servidor).
- **Decisión:** mientras haya sesión, los avisos no estén rechazados y el hub tenga algo que avise,
  la app instalada se queda **a la escucha**, como los TPV de mostrador (Square, Toast):
  1. un **servicio en primer plano** `specialUse` (`NoticeListeningService`) con su notificación
     fija y silenciosa en el idioma de la app, que mantiene vivo el proceso y su red bajo Doze;
  2. y, mientras escucha, el plugin **deshace el ocultado de la página** (`PageKeeper`): tras
     `onPause`/`onStop` vuelve a `WebView.onResume()` y, cuando la ventana deja de verse, le
     repite `dispatchWindowVisibilityChanged(VISIBLE)`. No se pinta nada (la ventana no tiene
     superficie); solo siguen corriendo los temporizadores y el socket de la página.
  El shell lo enciende tras la pregunta de avisos del inicio de sesión y lo apaga al cerrarla
  (`lib/notice-listening.ts`); se re-sincroniza al reactivar los avisos en Sistema.
- **Consecuencias:** medido con sesión real en banco-pre, pantalla apagada y Doze forzado, 4
  comandas creadas desde fuera: **4/4** avisos a la escucha frente a **0/4** sin ella (proceso
  congelado). A cambio, la página sigue ejecutándose con la pantalla apagada: más batería mientras
  dura el turno, que es lo que ya asume un TPV enchufado; al cerrar sesión o quitar la app de
  recientes se detiene. `specialUse` exige justificarlo en la ficha de Google Play. Si una build
  futura de Chromium/WebView dejara de respetar la visibilidad reafirmada, los avisos volverían a
  llegar solo con la app a la vista: la sonda de la PR (latidos cada 10 s durante 3 min con la
  pantalla apagada) es la que lo detecta.
- **Docs afectados:** `hub/apps/tauri.md` (§Android SÍ va por Tauri, «Avisos con la pantalla apagada»).
