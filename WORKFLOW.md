# WORKFLOW — La aplicación instalada (ERPlora para Windows, macOS y Android)

Prefijo: HUB_APP
Alcance MVP: transversal

> Contrato de comportamiento de `apps/tauri` y, por referencia, de `crates/tauri-plugin-erplora-android`
> (el Kotlin de Android: permisos, Bluetooth, NFC, aviso permanente, guardar y imprimir). Contrastado
> contra `origin/develop` del hub el 05/10/2026. Todo lo que aquí se afirma está **leído en el código,
> sin ejecutar**: no se ha instalado la aplicación ni tocado ningún emulador. Lo técnico vive en
> `architecture/hub/apps/tauri.md`; el protocolo de la impresora, en `crates/peripherals/WORKFLOW.md`
> (`HUB_PERIPHERALS`); lo que el servidor hace con la cola, en `WORKFLOW.md` de la raíz (`HUB`, área
> Impresión); lo que se ve en las pantallas del hub, en `apps/web/WORKFLOW.md` (`HUB_SHELL`).

## Para qué sirve y para quién

La aplicación instalada es una **ventana con permiso para tocar el hardware**. No lleva el negocio
dentro: abre `erplora.com` la primera vez y, una vez elegido el negocio, abre el hub de ese negocio
a pantalla completa en cada arranque (el hub lo sirve su servidor, no el instalador). Existe por lo
que un navegador no puede hacer: sacar papel por una impresora térmica, abrir el cajón, leer una
tarjeta NFC, avisar con el sistema operativo, arrancar con el ordenador y abrir ficheros o enlaces
fuera de la ventana. La usan el **administrador** y el **responsable** (la instalan, enlazan el negocio
y montan las impresoras) y, sin saberlo, el **cajero**, la **camarera** y el **empleado** cada vez que
cobran, disparan una comanda o pasan la placa.

Gobierna `apps/tauri/` y, **por referencia**, `crates/tauri-plugin-erplora-android/` (sus comandos y su
Kotlin son la mitad Android de los flujos de aquí). No gobierna `crates/peripherals` (qué bytes salen
por la impresora: `HUB_PERIPHERALS`), ni el servidor del hub, ni lo que las pantallas del hub pintan.
Hay un solo binario por sistema bajo una sola identidad, `com.erplora.app`.

## Referencia adoptada

No se rehace: la de impresión y hardware está contrastada en `.claude/qa/qa-hub.md` §8 y la de la app
de Android en `.claude/agents/qa-hub-android.md`. Se adopta solo esto (los nombres de productos son los
que citan los comentarios del código; **no se han vuelto a contrastar con el mercado en esta ronda**):

- **Montar el puesto como Square, Toast, Lightspeed o Shopify POS**: la impresora se encuentra sola o
  se añade por su IP, se prueba con una hoja y se le da una función (Recibo, Cocina, Barra, Etiqueta).
- **Explicar antes de pedir un permiso** (guía de Android y práctica de los TPV): una frase nuestra
  primero y el diálogo del sistema después, una sola vez; la respuesta «ahora no» no se vuelve a pedir
  sola.
- **Seguir a la escucha con la pantalla apagada** como las aplicaciones de pedidos para comercios
  (Uber Eats Orders, Glovo Partners, Deliveroo Restaurant Hub): servicio en primer plano con su aviso
  permanente. **No hay push** (decisión vigente, ERPlora/hub#2307, cerrada).
- **Atrás cierra lo de encima antes de salir** (Square, Loyverse) y **un aviso abre lo que sonó**
  (Square, Shopify Inbox, Zendesk).
- **Cobrar fuera de la aplicación**: las compras se hacen en el navegador del sistema (ADR-0114 §4),
  porque Google Play no permite pagar bienes digitales dentro; la copia de Play además se niega a
  seguir páginas del SaaS que cobran.
- **Actualizar por la tienda**: Play y Microsoft Store actualizan sus instalaciones; el resto recibe un
  aviso con la descarga.

## Antes de empezar

- Una **cuenta de ERPlora** y un negocio (el gratuito se crea en erplora.com con un paso después de registrarse: registrarse solo no lo crea) y **conexión a internet**:
  la aplicación no funciona sin red; con la red caída solo enseña una pantalla de espera (HUB_APP-F11).
- **Sistemas**: Windows 10 1809 o posterior, 64 bits, con WebView2 (Windows 10/11 al día lo traen);
  Android 7 (API 24) o posterior, compilada para Android 16 (API 36); macOS en Apple Silicon, que se
  construye pero erplora.com no reparte (no hay descarga de macOS). También se construyen para Linux
  (`.deb`, AppImage): no es un sistema anunciado, pero la descarga web de erplora.com sí reparte la
  AppImage (Dudas abiertas).
  iOS está en el código pero no se construye ni se publica. El manifiesto de Android declara también el
  lanzador de Android TV (`LEANBACK_LAUNCHER`); no es un dispositivo probado.
- Para imprimir: una **impresora térmica ESC/POS** encendida, en la misma red (puerto 9100), por USB
  en un ordenador macOS o Linux, o emparejada por Bluetooth en Android. La configuración paso a paso
  de las impresoras es la de Impresión (`PRINTING`, «Configuración inicial»); aquí cambia lo que pasa
  por debajo.
- Para NFC: una tableta o móvil Android con lector y el NFC encendido; en un ordenador, un lector USB
  que escribe como teclado (no es de esta aplicación).
- En **Android 17**, el permiso de red local; en **macOS 15**, el de red local; en Android 13 o
  posterior, el de notificaciones; en Android 12 o posterior, el de Bluetooth (HUB_APP-F07, F09).

## Pantallas

La aplicación tiene muy pocas pantallas **propias**; casi todo lo que se ve lo pinta el hub o el SaaS
dentro de su ventana. Cada flujo dice cuál es.

### Ventana de ERPlora
La ventana principal (`main`): 1280×800, mínimo 960×600, titulada «ERPlora», sin barra de direcciones
ni botón de recargar. Carga `erplora.com/shell/` si no recuerda ningún negocio o el hub recordado si
lo hay. Si la dirección inicial es ilegible carga la pantalla «Sin conexión a internet». Una sola por
instalación en ordenador (una segunda apertura entrega sus argumentos a la ya abierta). En Android la
ventana reserva el hueco de la barra de estado y del recorte de pantalla (`SystemBarInsets.kt`, hub#1719,
hub#1895) para que ninguna página se pinte sobre el reloj.

### Sin conexión a internet
Página incluida en la propia aplicación, con la marca. Título «Sin conexión a internet»; texto «ERPlora
no puede conectar con tu negocio ahora mismo. Es un problema de conexión, no es un fallo de la
aplicación: no se ha perdido nada de lo que guardaste.»; consejo «Comprueba el wifi o los datos móviles.
ERPlora vuelve a conectarse solo en cuanto haya red.»; botón «Reintentar»; línea de estado «Comprobando
la conexión…» o «Sigue sin conexión. Comprueba el wifi o los datos móviles.». Va en español, o en
inglés si el dispositivo no está en español. Vacía/cargando/error: es ella misma el estado de error de red.

### Aviso «Esta página no está disponible en la aplicación.»
Un diálogo del sistema (`alert`) que sale encima de la página en la que se queda la ventana cuando la
copia de Google Play se niega a seguir una página del SaaS. En el idioma del dispositivo (español, o inglés si no está en español).

### Ventana de impresión
Solo en ordenador: una ventana de 820×1060 titulada «ERPlora» que enseña el documento A4 y abre el
diálogo de impresión del sistema (lista de impresoras y «Guardar como PDF»). En Android es la pantalla
de impresión del propio Android.

### «¿Abrir … en este dispositivo?»
Un diálogo del sistema (no de la página) que sale cuando un enlace `erplora://hub/…` lleva a otro negocio que el
enlazado (HUB_APP-F03): título «¿Abrir <dirección> en este dispositivo?», el aviso de que ese negocio usará la
impresora, el cajón y el lector de tarjetas y será el que abra la aplicación al arrancar, y los botones «Abrir» y
«Cancelar». En inglés («Open <dirección> on this device?», «Open», «Cancel») si el sistema no está en español.
Cerrarlo es «Cancelar», y en ordenador «Cancelar» es también el botón de Intro.

### Pedir permiso
Los diálogos del sistema operativo (red local, notificaciones, Bluetooth), precedidos por la frase del
hub («Vamos a buscar tu impresora», «Deja que te avisemos»). Los textos son de `HUB_SHELL`.

### Aviso del sistema
La notificación del sistema (Windows, macOS, Linux, Android) con título y cuerpo. Y, en Android, la
notificación permanente «ERPlora está a la escucha» («Te avisará cuando algo necesite tu atención, aunque
la pantalla esté apagada.»; canal «Avisos con la pantalla apagada»), silenciosa y marcada como permanente (Android 14 o posterior deja descartar las de un servicio en primer
plano: sin confirmar en un dispositivo).

### Pantallas de otros que esta aplicación usa
`saas: Abre tu hub` (la lista de negocios de `/shell/`) · `printing: Impresoras` (buscar, añadir, probar,
función) · `HUB_SHELL: Sistema` (tarjeta de impresión, avisos, aviso de actualización) · `HUB_SHELL:
Ajustes` (Hub › «Arrancar al iniciar sesión») · `HUB_SHELL: Barra superior` («Cambiar de negocio»).

## Qué puede hacer cada página (los tres juegos de permisos)

La aplicación no «cambia de modo»: hay **dos puertas** y una orden de la aplicación tiene que pasar
las dos (HUB_APP-F10).

1. **Por patrón** (Tauri): decide por el origen de la página qué órdenes nativas acepta. Hay tres juegos
   (`capabilities/*.json`), todos para la ventana `main`. Cualquier otra ventana (la de impresión) no
   tiene ninguno.
2. **Por el negocio enlazado** (`src/hub_link.rs`, ERPlora/hub#2504): de las páginas que deja pasar el
   patrón, **solo la del negocio enlazado** (mismo origen que `hub.url`: esquema, nombre y puerto) usa las
   órdenes de la aplicación, y solo **una vez cargada** si la ventana llegó a ella desde otro origen (la
   dirección de la ventana cambia al empezar la navegación, mientras la página anterior sigue viva). Las
   únicas abiertas a cualquier página que pase el patrón son
   `device_context`, `forget_hub` y `shell_retry`; cualquier otra, también una que se añada mañana, contesta
   el error `not_the_linked_hub` sin ejecutarse. Sin negocio enlazado (instalación nueva, tras «Cambiar
   de negocio» o tras olvidar uno borrado) ninguna página maneja el equipo. Las órdenes del plugin de Android
   (`plugin:erplora-android`: permisos, escucha, salir, ajustes) pasan por la misma puerta, sin ninguna abierta
   (ERPlora/hub#2642), y también la del plugin de avisos `plugin:notification` (escuchar los toques;
   ERPlora/hub#2658). El plugin `plugin:app` de Tauri (la versión) no se puede poner tras la puerta, así que
   ningún juego concede su versión: el negocio enlazado la lee con `erplora_bridge_status`; de ese plugin solo
   queda la escucha, por la que llega el botón Atrás de Android (ERPlora/hub#2658).

| Juego | Se aplica a | Puede | No puede |
|---|---|---|---|
| `default` | `https://*.erplora.com/*`: **cualquier subdominio** (los hubs de cualquier negocio, y también `www` y `pre`, que sirven el SaaS), `http://127.0.0.1:8787` y `:5173` (puertos de desarrollo que viajan en el binario de producción) **y la página incluida en la aplicación** | **Si además es el negocio enlazado (segunda puerta)**: todo el hardware (buscar, añadir, probar, imprimir, cajón, funciones, nombres, quitar), `device_context`, `forget_hub`, abrir enlace externo, guardar descarga, imprimir A4, avisos del sistema, escuchar los toques de los avisos y reclamar el de un aviso, la versión de la aplicación (`erplora_bridge_status`), NFC, arranque automático y los permisos de Android, la escucha, salir y sus ajustes. **Cualquier página del patrón**: los juegos básicos de Tauri, `core:default` menos `core:app`, del que solo queda la escucha (el botón Atrás de Android, HUB_APP-F28; ERPlora/hub#2658) | Las órdenes crudas del plugin de abrir ficheros; navegar la ventana; `plugin:app` (versión, ERPlora/hub#2658) |
| `onboarding` | `https://erplora.com/*` (el apex, el SaaS) y `127.0.0.1:8001` (desarrollo) | Solo `device_context` (identidad para iniciar sesión) y `forget_hub` | Todo el hardware, abrir enlaces, guardar, imprimir, avisos, NFC, permisos de Android, versión |
| `degraded` | Solo la página incluida en la aplicación (sin `remote`) | Añade solo `shell_retry` (comprobar la red y mover la ventana) | Nada más propio; hereda de `default` lo que ya tiene esa página |

Lo que esto significa de verdad:

- El hardware **es del negocio enlazado**: otro negocio, `www`, `pre` o el bucle local pasan el patrón pero
  la segunda puerta les contesta `not_the_linked_hub`. Una página no se enlaza a sí misma: `?shell=1` solo
  enlaza cuando la ventana llega desde la página de entrada del SaaS (o es el negocio ya enlazado). Lo mismo los
  permisos de Android, la escucha y salir de la aplicación (ERPlora/hub#2642). Y escuchar los toques de los avisos y
  leer la versión (ERPlora/hub#2658). Lo que queda abierto: que un enlace `erplora://hub/…` acepta como destino
  también `www`/`pre` (ERPlora/hub#2645); ese enlace, si lleva a otro negocio, solo enlaza cuando la persona
  contesta «Abrir» en un diálogo del sistema (ERPlora/hub#2644). `[SEG]`
- El bucle local pasa el patrón solo en `:8787` y `:5173`, pero un enlace profundo lo acepta —y lo enlaza—
  con **cualquier puerto** (`hub_url_for_host`) y la captura lo deja recordado también con `https`
  (`trusted_hub_origin`): un proceso local que escuche en un puerto y un enlace dejan el mostrador arrancando
  siempre en su página, sin barra de direcciones y titulada «ERPlora» (en Android release lo frenaría quizá
  `usesCleartextTraffic=false`: sin confirmar; ERPlora/hub#2643). `[SEG]`
- Una página de otro dominio (dominio propio del negocio) **no puede ni pedir su identidad**, así que parece
  funcionar y no imprime. El SaaS usa para el alta el `app_access_url` (dirección de la plataforma) por esto
  (hub#448, cerrada).

## Órdenes nativas: quién las llama y qué flujos tocan

**Regla: una orden nativa no se renombra ni se quita; se añade la nueva y la vieja se mantiene.** El hub sirve
la página nueva mientras en los mostradores hay binarios viejos (y al revés): renombrar una orden rompe todas
las instalaciones hasta que se actualicen. Hay que tocar a la vez `build.rs`, `generate_handler!`,
`capabilities/*.json`, `permissions/autogenerated`, el SDK (`packages/module-sdk`), `apps/web/src/lib` y los
tests `tests/shell_surface.rs`, `app_update_channel.rs` y `notice_tap.rs`.

| Orden | Quién la llama | Flujos HUB_APP | Fuera de HUB_APP |
|---|---|---|---|
| `device_context` | `device.ts` (inicio de sesión) | F06 | HUB-F137, HUB-F139; SaaS (login) |
| `forget_hub` | `change-hub.ts` (`signOutAndForgetHub`, también el 410 de `main.ts`) | F04, F05 | HUB_SHELL Barra superior |
| `open_external_url` | `open-external.ts` (pago, plan, descarga) | F29, F31 | HUB_SHELL Aplicaciones/plan, Sistema |
| `save_download` | `save-download.ts` | F30 | HUB_SHELL Archivos, Ajustes |
| `print_document` | `native-print.ts` → `print.ts` | F22 | PRINTING-F08 |
| `shell_retry` | `shell-dist/index.html` | F11 | — |
| `erplora_bridge_status` | SDK `detect` → Impresión, Sistema; `app-update.ts` (la versión instalada, ERPlora/hub#2658) | F18, F31 | PRINTING-F02 |
| `erplora_discover_printers` | SDK → Impresión | F13, F15, F16 | PRINTING-F02 |
| `erplora_get_devices` | `print.ts`, `print-host.ts`, `print-on-sale.ts` | F17–F21 | HUB-F196 |
| `erplora_print` | SDK → `print.ts`, `print-host.ts` | F19 | PRINTING-F07/F10, SALES-F01, KITCHEN-F08, HUB-F199 |
| `erplora_test_print` | SDK → Impresión | F20 | PRINTING-F03/F05 |
| `erplora_open_drawer` | SDK → `print-on-sale.ts` | F21 | PRINTING-F13, SALES-F01, HUB-F207 |
| `erplora_set_device_role` | SDK → Impresión, `print-host.ts` | F17, F18 | PRINTING-F04 |
| `erplora_add_network_printer` | SDK → Impresión | F14 | PRINTING-F03 |
| `erplora_set_device_name`, `erplora_remove_device` | nadie | F17 | — |
| `erplora_notify` | `bridge-transport.ts`, SDK `notify` | F24 | KITCHEN-F05, HUB-F60 |
| `erplora_take_notice_tap` | `main.ts:342-345` | F25 | — |
| `erplora_nfc_read` | `nfc-badge.ts` | F23 | HUB-F134 |
| `autostart_*` | `autostart.ts` → Ajustes | F27 | HUB_SHELL Ajustes |
| `plugin:erplora-android` `check_permissions` y `request_permissions` (solo el negocio enlazado, ERPlora/hub#2642) | `device-permission.ts`, `bridge-transport.ts`, SDK | F07, F08, F13 | PRINTING-F02 |
| `plugin:erplora-android` `keep_listening` (solo el negocio enlazado, ERPlora/hub#2642) | `notice-listening.ts` | F26 | — |
| `plugin:erplora-android` `leave_app` (solo el negocio enlazado, ERPlora/hub#2642) | `router/back-closes-overlay.ts` | F28 | — |
| `plugin:erplora-android` `open_app_settings` (solo el negocio enlazado, ERPlora/hub#2642) | `device-permission.ts` → Sistema | F08 | — |
| `plugin:notification` `register_listener` (solo el negocio enlazado, ERPlora/hub#2658) | `notice-tap.ts` vía `main.ts:339` | F25 | HUB_SHELL-F67 |
| `plugin:app` `version` (Tauri; solo la contestan las aplicaciones anteriores a ERPlora/hub#2658, en las nuevas ningún juego la concede) | `app-update.ts` | F31 | — |

## Flujos

El detalle de cada flujo vive en `workflow/<área>.md`; este índice solo lo enumera.

| Flujo | Título | Estado | Fichero |
|---|---|---|---|
| HUB_APP-F01 | Instalar la aplicación | parcial | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F02 | Primer arranque: entrar y abrir el negocio | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F03 | Abrir un negocio desde un enlace | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F04 | Cambiar de negocio | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F05 | Olvidar un negocio que ya no existe | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F06 | Saber qué equipo es este | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F07 | Android pide los permisos en su momento | parcial | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F08 | Volver a activar un permiso negado | hecho | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F09 | Red local en ordenador (macOS y Windows) | parcial | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F10 | Cada página tiene su juego de permisos | parcial | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F11 | Cuando no hay conexión con el negocio | hecho | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F12 | En la copia de Google Play, solo páginas del SaaS que no cobran | parcial | [workflow/instalar-y-enlazar.md](workflow/instalar-y-enlazar.md) |
| HUB_APP-F13 | Buscar impresoras | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F14 | Añadir una impresora por su IP | hecho | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F15 | Impresora Bluetooth (solo Android) | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F16 | Impresora USB (solo ordenador) | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F17 | Dar nombre y función a una impresora, y quitarla | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F18 | Ser el puesto que imprime | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F19 | Imprimir un tique, una factura o una comanda | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F20 | Hacer una hoja de prueba | hecho | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F21 | Abrir el cajón | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F22 | Imprimir un documento A4 con el diálogo del sistema | hecho | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F23 | Leer una tarjeta NFC para entrar | parcial | [workflow/impresion-y-hardware.md](workflow/impresion-y-hardware.md) |
| HUB_APP-F24 | Avisar con el sistema aunque nadie mire la pantalla | hecho | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F25 | Tocar un aviso | hecho | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F26 | Seguir a la escucha con la pantalla apagada (Android) | parcial | [workflow/permisos-y-avisos.md](workflow/permisos-y-avisos.md) |
| HUB_APP-F27 | Arrancar con el ordenador | hecho | [workflow/salidas-y-actualizacion.md](workflow/salidas-y-actualizacion.md) |
| HUB_APP-F28 | Botón Atrás de Android | hecho | [workflow/salidas-y-actualizacion.md](workflow/salidas-y-actualizacion.md) |
| HUB_APP-F29 | Abrir un enlace fuera de la aplicación | hecho | [workflow/salidas-y-actualizacion.md](workflow/salidas-y-actualizacion.md) |
| HUB_APP-F30 | Guardar una descarga | parcial | [workflow/salidas-y-actualizacion.md](workflow/salidas-y-actualizacion.md) |
| HUB_APP-F31 | Saber que hay una versión nueva y actualizar | parcial | [workflow/salidas-y-actualizacion.md](workflow/salidas-y-actualizacion.md) |

## Cobertura contra la referencia

| Elemento | Estado | Flujo |
|---|---|---|
| Instalar desde la tienda (Play, Microsoft Store) | parcial: publicación sin confirmar | F01 |
| Instalador directo de Windows y `.dmg` de macOS | parcial: sin firmar / sin notarizar | F01 |
| Elegir negocio y recordarlo | hecho | F02 |
| Enlace profundo para abrir un negocio | hecho | F03 |
| Cambiar de negocio | hecho | F04 |
| Olvidar un negocio borrado | hecho | F05 |
| Escanear un QR dentro de la aplicación para enlazar | no existe (la aplicación no tiene cámara; el QR del menú abre el negocio por https en el móvil) | F03 |
| Permisos de Android con explicación previa | parcial | F07, F08 |
| Permiso de red local en macOS | parcial | F09 |
| Juegos de permisos por origen y hardware solo para el negocio enlazado | parcial: el binario publicado acepta el bucle local como negocio (hub#2643) | F10 |
| Pantalla sin conexión y vuelta sola | hecho | F11 |
| Buscar impresoras (red, Bluetooth, USB) | parcial: Bluetooth negado sin aviso | F13 |
| Añadir por IP | hecho | F14 |
| Impresora Bluetooth | parcial: no se empareja desde ERPlora | F15 |
| Impresora USB | parcial / no hecho en Windows | F16 |
| Nombre, función y quitar impresora | parcial | F17 |
| Puesto de impresión | parcial | F18 |
| Imprimir tique y comanda con aviso si falla | parcial: red sin papel sin aviso | F19 |
| Hoja de prueba | hecho | F20 |
| Abrir el cajón | parcial | F21 |
| Imprimir A4 con el diálogo del sistema | hecho | F22 |
| Tarjeta NFC | parcial: sin validar con hardware | F23 |
| Aviso del sistema y abrir su pantalla al tocarlo | hecho | F24, F25 |
| Avisos con la pantalla apagada (sin push) | parcial | F26 |
| Arranque automático | hecho | F27 |
| Atrás de Android | hecho | F28 |
| Enlace externo, guardar descarga | hecho / parcial | F29, F30 |
| Actualización | parcial: sin actualización en el sitio | F31 |
| Báscula | no existe (hub#1217) | — |
| Push (FCM) con la aplicación cerrada | no existe, por decisión (hub#2307) | F26 |
| iOS | sin construir | — |

## Datos: de quién es cada dato

- **De la aplicación, en su carpeta de datos del usuario** (no en el hub): `device.id` (identificador de
  instalación), `hub.url` (origen del negocio recordado) y `devices.json` (registro de impresoras:
  clave, MAC si se conoce, dirección, puerto, nombre, función, tipo, vistas por primera y última vez,
  en línea). Ninguno se sube al hub por sí solo.
- **De Android**: el último toque de aviso recordado (`com.erplora.notice_taps`) y los permisos del sistema.
- **Del navegador integrado**, guardado en disco por origen: la sesión del hub (`erplora.hub_session`), el
  usuario con nombre y correo (`erplora.session`), los tokens de acceso y refresco de erplora.com
  (`erplora.access`, `erplora.refresh`), la memoria de «ya te pregunté» de los permisos
  (`erplora.notifications.primerAnswered`, `erplora.localNetwork.primerAnswered`) y la última versión
  anunciada. «Cambiar de negocio» (HUB_APP-F04) borra la sesión, el usuario y los tokens; sobreviven la memoria
  de los permisos y la última versión anunciada, que son del dispositivo.
- **Del hub**: la cola de impresión, los puestos, las funciones, las sesiones (HUB, área Impresión y
  Acceso). **Del SaaS**: la cuenta, los negocios y las versiones publicadas.
- **Datos personales** (inventario RGPD): la aplicación no guarda ninguno por su cuenta, pero **sí hay datos
  personales en disco**: el nombre y el correo de quien entró y sus credenciales (arriba), en el
  almacenamiento de la ventana; la MAC y la IP de impresoras de la empresa y los nombres que se les dieron.
  Pasan por memoria, sin registro: títulos y cuerpos de avisos (pueden nombrar a un cliente), documentos de
  impresión (nombre, NIF y dirección del cliente; mesa y camarero) y el UID de una tarjeta NFC. El centro de
  notificaciones de Windows y de macOS guarda los títulos y cuerpos de los avisos. Desinstalar o borrar la
  carpeta de datos elimina lo guardado por la aplicación.

## Reglas que no se rompen

- **El hardware solo lo usa el negocio enlazado** (ERPlora/hub#2504): la primera puerta lo limita a orígenes
  `*.erplora.com`, al bucle local en dos puertos y a la página incluida, sin el apex (`capabilities/default.json`,
  `tests/remote_acl.rs`); la segunda, al origen exacto del negocio enlazado **ya cargado** (`src/hub_link.rs`:
  tras una navegación a otro origen nada maneja el equipo hasta que la página nueva termina de cargar). Toda orden nueva
  nace cerrada: solo `device_context`, `forget_hub` y `shell_retry` están abiertas a cualquier página del patrón.
  Las del plugin de Android pasan por la misma puerta (ERPlora/hub#2642) y las de `plugin:notification`
  también (ERPlora/hub#2658); `plugin:app`, que no se puede envolver, no lo concede ningún juego.
- **Solo el SaaS elige el negocio**: `?shell=1` enlaza y se recuerda solo si la ventana sale de la página de
  entrada (el origen del SaaS; en desarrollo también el de `ERPLORA_SHELL_URL`) o cae en el negocio ya enlazado.
  La aplicación enlaza por su cuenta el negocio recordado al arrancar; el destino de un enlace `erplora://hub/…`
  a otro negocio, solo cuando la persona contesta «Abrir» en el diálogo del sistema (ERPlora/hub#2644).
- **Solo se recuerda o se abre por enlace** `<etiqueta>.erplora.com` (incluidos `www` y `pre`) o el bucle local
  con cualquier puerto; al navegador del sistema también van el apex y `checkout.stripe.com`.
- **El enlace profundo se resuelve, no se abre**: la dirección se reconstruye desde el nombre.
- **Una orden que no pudo hacerse se devuelve, no se traga** (abrir enlace, guardar, imprimir A4, NFC,
  Bluetooth, USB). También la impresora de red: espera a que el papel llegue a la impresora y, si no
  llega, devuelve el error (HUB_APP-F19, hub#2494).
- **La copia de Google Play no sigue páginas del SaaS que cobran ni nombra otro sitio donde pagar**, salvo los
  cambios de página por dentro del panel y las salidas al navegador del sistema, que no pasan por la lista
  (ERPlora/hub#1918, abierta).
- **Un permiso que Android no conoce no se pide, y uno que falta no se lee como denegado.**
- **Una descarga nunca pisa otro fichero en el escritorio; el nombre lo pone la página (saneado y rechazado si no
  vale) y la carpeta la decide la aplicación.**
- **Una orden nativa no se renombra ni se quita** (ver «Órdenes nativas»).
- **La actualización no interrumpe**: no recarga, no cierra y no instala sola.

## Lo que NO hace, a propósito

- No lleva el negocio ni la base de datos dentro: no hay servidor local, ni modo sin conexión, ni
  autorización de módulos (la decide el servidor).
- No hay push (FCM): los avisos con la pantalla apagada van por mantener la aplicación escuchando.
- No empareja Bluetooth ni escanea dispositivos (solo lee los emparejados); no pide el permiso de escaneo.
- No actualiza la aplicación por sí misma.
- No tiene icono de bandeja ni mantiene la aplicación viva al cerrar la ventana.
- No lee básculas ni escáneres de códigos (el escáner es un teclado para el sistema).
- No escanea códigos QR ni usa la cámara.
- No maneja impresoras de etiquetas (ZPL/TSPL/EPL) ni imprime imágenes (`HUB_PERIPHERALS`).
- No sigue con permisos las páginas de otros dominios: un negocio con dominio propio no recibe hardware.
- No deja el hardware, los permisos de Android, los toques de los avisos ni la versión a una página de
  erplora.com que no sea el negocio enlazado, aunque la ventana la enseñe (otro negocio, la web pública, el entorno de pruebas): contesta `not_the_linked_hub`.
- No deja ningún registro de lo que falla: la aplicación no instala destino para `log`/`tracing`.

## Dudas abiertas

Se resuelven con `market-decision`; no las decide el worker.

1. **Báscula**: no existe en la aplicación (ni orden, ni permiso, ni evento `erplora:scale-weight`);
   ERPlora/hub#1217 sigue abierta (P1, hardware). ¿Entra en el MVP o después del primer cliente?
2. **¿Se sigue anunciando Linux?** Se construyen `.deb` y AppImage («canal de QA») y ningún documento
   de producto los ofrece, pero la descarga web de erplora.com sí reparte la AppImage. ¿Se retira de la matriz o se declara?
3. **macOS**: el `.dmg` sale sin notarizar y `downloadPlatform` no tiene destino para él. ¿Se publica una
   descarga o se declara fuera del MVP?
4. **Impresora de red encendida pero sin papel o con la tapa abierta**: hoy contesta «correcto». ¿Se
   consulta su estado ESC/POS en tiempo real antes de dar el papel por salido? (hub#2716; la impresora apagada ya
   la cerró hub#2494: se confirma la escritura en el socket y la caja ve un aviso fijo con «Reintentar».)
5. **Puesto de impresión que solo imprime** en Android: ¿debe mantenerse a la escucha aunque no haya cocina,
   citas ni contador de campana? Hoy no.
6. **El QR del menú abre el negocio por https**, no `erplora://`: en un móvil con la aplicación instalada se
   abre en el navegador. ¿Es lo que se quiere?
7. La ventana emergente de **Meta** («Tu número» de WhatsApp) usa una ventana nueva del navegador integrado,
   que en la aplicación no abre ninguna (`window.open` no hace nada): sin confirmar cómo se completa ahí.

8. ~~`[SEG]` ¿Se restringe el hardware a los hubs?~~ Resuelta por ERPlora/hub#2504: solo el negocio enlazado
   usa las órdenes de la aplicación, desde ERPlora/hub#2642 las del plugin de Android y desde
   ERPlora/hub#2658 los toques de los avisos y la versión. Desde ERPlora/hub#2644 un enlace a otro negocio
   pregunta antes de enlazarlo. Quedan `www`/`pre` como negocio (ERPlora/hub#2645) y el bucle local en producción
   (ERPlora/hub#2643).
9. ~~`[SEG]` **Cambiar de negocio** debería cerrar la sesión del hub.~~ Resuelta por ERPlora/hub#2503: para la
   escucha de Android, revoca la sesión, borra la sesión, el usuario y los tokens de erplora.com y solo entonces
   olvida el negocio (HUB_APP-F04); el 410 de HUB_APP-F05 va por el mismo camino.
10. **Sin confirmar en un dispositivo**: que `usesCleartextTraffic=false` impida en Android release cargar el
    bucle local por http; que la notificación del servicio en primer plano sea descartable en Android 14+;
    si el aviso de la comanda suena (canal por defecto del plugin); si `ping`/`arp` del vigilante hacen
    parpadear una consola en Windows; si en macOS `wait_for_click` vuelve tras pasar el aviso al Centro de
    notificaciones; el estado real de Play y de la Store.
11. **Escucha en Android**: ¿debe pararse el servicio si los avisos se niegan después en los ajustes del
    sistema? Hoy no se para.

## Fuentes contrastadas

- `GOOGLE-PLAY.md` dice «Sin declaración de FGS… la app no corre en segundo plano»: **ya no es cierto**; el
  manifiesto declara el servicio en primer plano `specialUse` desde hub#2307 y lo arranca la página.
- `apps/tauri/README.md` y los comentarios de `app-update.ts` dicen que macOS «se construye solo en local»;
  `tauri-release.yml` ya construye el `.dmg` en CI (sin notarizar).
- El guion de QA de Android (`qa-hub-android.md`) no tiene identificadores de escenario, solo fases;
  `PRINTING` cita `qa-hub-android §15`, que es el paso 15 de la Fase 3: debería escribirse `qa-hub-android Fase 3`.
- **No hay registro técnico**: el shell escribe con `log::` y el crate de periféricos con `tracing::`, pero la
  aplicación no instala ningún destino (ni `tauri-plugin-log`, ni `android_logger`, ni `tracing-subscriber`;
  `Cargo.lock`), y los `eprintln!` van a un stderr que nadie lee. El comentario de `Cargo.toml` («sin `log`, un
  fallo es invisible en logcat») es falso, como el de `usb.rs:19-22` sobre las colas USB sin función.
- `PRINTING-F02` dice que el permiso de red local negado sale «sin traducir»; por el código el shell lo cambia
  por la frase en español (`bridge-transport.ts:151-167`): verificar en pantalla.
- `KITCHEN-F05` omite tres condiciones del aviso «Nueva comanda»: avisos sin negar, la escucha de Android
  (HUB_APP-F26) y que sale en todos los dispositivos con el hub abierto, también el que la disparó.
- `architecture/hub/apps/tauri.md` §«Las cuatro preguntas del ciclo de vida» dice que no hay control para
  cambiar de negocio (hub#447): sí lo hay (HUB_APP-F04), cerrada.
- `README.md` de la app llama «Bridge» a la app: el Bridge no existe (ADR-0196); lo que queda con ese nombre
  es `erplora_bridge_status` (devuelve la versión) y `bridge-transport.ts` (el transporte de hardware).
- El guion dice que `qa-hub-android` prueba el deep link en «Pendiente (fase C)»: no hay escenario aún.
