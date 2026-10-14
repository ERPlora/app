> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-09-30-avisos-escritorio-y-android-en-frio.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## ADR-NNNN · Avisos en escritorio y Android en frío: el shell guarda el toque y la página lo reclama — 2026-09-30

- **Estado:** aceptada · hub#2360.
- **Contexto.** Desde hub#2305 tocar un aviso abre su pantalla en Android e iOS con la app viva. En
  escritorio el plugin oficial de notificaciones muestra el aviso y descarta su respuesta (el clic
  solo trae la app al frente), y en Android, cuando el toque tiene que arrancar la app, llega antes
  de que la página remota se haya suscrito, así que se pierde.
- **Decisión.** El shell guarda el toque y la página lo reclama con `erplora_take_notice_tap`, al
  arrancar y cada vez que el shell emite `erplora://notice-tapped`. En escritorio el shell muestra
  el aviso él mismo (`mac-notification-sys` en macOS, `notify-rust` en Windows/Linux) y espera el
  clic en un hilo propio. En Android el plugin propio guarda el toque que arranca la app por los
  dos caminos en frío: sin tarea, del intent con el que se crea la actividad; con el proceso muerto
  por el sistema y la tarea todavía en recientes, la actividad vuelve con su intent de lanzador y el
  toque entra por `MainActivity.onNewIntent`. El toque del que nació la tarea (id y contenido) se
  recuerda en las preferencias de la app, porque al volver por el icono a una tarea que nació de un
  toque Android devuelve aquel intent con sus extras y sin marca de recientes, en cada vuelta del
  proceso. El toque que entra por `onNewIntent` no se recuerda ni se compara: Android lo entrega una
  sola vez, y el intent de origen, que se lee después, no lo pisa. El aviso lleva la ruta en
  `extra.path` para la página nueva que ya no tiene el mapa id → ruta, siempre filtrada por la regla
  de pantallas del shell.
- **Descartado.** (a) Esperar a que el plugin oficial reporte clics en escritorio: no hay fecha y
  el TPV de escritorio es el caso principal. (b) Guardar el destino en el almacenamiento de la
  página: la página remota puede cambiar de origen y el toque en frío llega antes de que cargue.
  (c) Marcar el intent ya usado (`putExtra`) o fiarse de la marca de recientes: ninguna sobrevive a
  la muerte del proceso ni a la vuelta por el icono (medido en emulador en la revisión de hub#2411).
  (d) Recordar «el último toque guardado», venga por donde venga: con la tarea nacida de un toque y
  el proceso muerto, el toque nuevo entra por `onNewIntent`, pasa a ser «el último» y el intent de
  origen deja de coincidir, así que se abría la pantalla del aviso viejo (medido en la segunda
  revisión de hub#2411).
- **Consecuencias.** + En macOS, Linux y Windows (aviso en pantalla) el clic abre la comanda, la
  agenda o la pestaña del contador; en Android también cuando el toque arranca la app, haya o no
  tarea en recientes. − Si la tarea se quitó de recientes y el toque que la vuelve a crear es el del
  mismo aviso (mismo id y mismo contenido) del que nació la anterior, solo trae la app al frente;
  dos avisos distintos no coinciden porque la página siembra los ids con el reloj. − En Windows el clic desde el Centro de actividades no llega (hub#2409). − Dos
  dependencias de escritorio más, ya presentes en el lock por el plugin.
- **Docs afectados:** [`hub/apps/tauri.md`](../hub/apps/tauri.md) (avisos).
