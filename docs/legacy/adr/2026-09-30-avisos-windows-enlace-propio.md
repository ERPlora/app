> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-09-30-avisos-windows-enlace-propio.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## ADR-NNNN · En Windows, el aviso del sistema lleva su propio enlace `erplora://notice` para que el clic desde el Centro de actividades abra su pantalla — 2026-09-30

- **Estado:** aceptada · hub#2409.
- **Contexto:** hub#2360 hizo que el shell de escritorio muestre él mismo los avisos y espere su clic
  en un hilo. En Windows eso iba por `notify-rust`, que espera el clic con un solo `recv()`: cuando el
  aviso caduca en pantalla y pasa al Centro de actividades, el hilo termina y el clic posterior se
  pierde; con la app cerrada no hay ni proceso que lo reciba. `tauri-winrt-notification` (el backend
  de `notify-rust`) no deja fijar `launch` ni `activationType`.
- **Decisión:** en Windows el shell crea el toast por WinRT (crate `windows`) con
  `activationType="protocol"` y `launch="erplora://notice?id=<id>&path=<ruta>"`. El clic lanza la app
  con ese enlace: abierta, `tauri-plugin-single-instance` lo reenvía; cerrada, llega por argumentos y
  `setup` lo contesta tras abrir la ventana. Los dos caminos acaban en el mismo `on_click` del resto
  del escritorio. El MSIX declara el protocolo `erplora`. La ruta del enlace es entrada no confiable:
  solo se guarda una ruta propia y la página la revalida con `SCREEN_PATH`.
- **Descartado:** (1) activador COM (`ToastActivatorCLSID`): exige un servidor COM, un CLSID en el
  acceso directo del instalador NSIS y en el MSIX, y registrar la clase; más piezas para el mismo
  resultado que un protocolo que la app ya registra. (2) Seguir esperando `Activated` en el proceso:
  no llega tras caducar el aviso ni con la app cerrada, que es justo el caso de la issue.
- **Consecuencias:** `notify-rust` queda solo para Linux. El `deliver` de Windows no espera ningún
  clic (devuelve «sin clic» siempre). Cualquiera puede lanzar `erplora://notice?...`: como mucho abre
  una pantalla del propio shell. No verificado todavía en un Windows real: lo valida `qa-hub-windows`.
- **Docs afectados:** `hub/apps/tauri.md` (§Tocar un aviso abre su pantalla y §El deep link).
