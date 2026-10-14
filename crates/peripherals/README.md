# erplora-peripherals

Lógica de hardware POS reutilizable: renderiza **ESC/POS** y lo entrega por el transporte que
nombre el `printer_id`.
Diseño en [architecture/hub/apps/bridge.md](../../../architecture/hub/apps/bridge.md) (reubicado desde `ARQUITECTURA.md §2.7`). Es el reemplazo en Rust del Bridge Python
(`bridge/ERPlora-Bridge-desktop`), empaquetado como **librería** sin servidor ni UI.

## Consumidores

- **`apps/tauri`** — **la** app instalable (ADR-0196/0204): sus `invoke` de hardware llaman a este
  crate **in-process**. No hay servidor local, ni puerto, ni token de emparejamiento.

> 🪦 Hubo un segundo consumidor, el **Bridge standalone** (`apps/bridge`, Axum `GET /status` +
> `WS /ws` en `localhost:12321`), al que la web shell llegaba por localhost. ADR-0196 lo retiró y
> **hub#340 lo borró del árbol**: el precio explícito es que una PWA en un navegador, sin la app,
> no imprime tiques térmicos. Historia en
> [architecture/hub/apps/bridge.md](../../../architecture/hub/apps/bridge.md).

Habla el **mismo protocolo JSON** que la web shell del Hub consume.

## Transportes (`printer_id`)

La decisión original (§2.7) era **red-only**, y su motivo —«un driver por SO no escala»— sigue en
pie. Lo que ha cambiado dos veces es que hay maneras de dar USB y Bluetooth **sin escribir un
driver**, y por ahí han vuelto los dos:

| `printer_id` | Dónde | Transporte | Cola + reintentos |
|---|---|---|---|
| `network:{ip}:{port}` | en todas partes | ESC/POS por TCP al 9100 | **sí** (`queue`) |
| `bluetooth:{mac}` | **solo Android** (ADR-0204, hub#388) | SPP, en el plugin Kotlin | no — fase 1 |
| `usb:{queue}` | **solo escritorio** (hub#1083) | `lp -o raw` a la cola del SO (`usb`) | no — fase 1 |

En los tres el **renderizado es el mismo**: los módulos llaman a `sdk.print` y nunca ven el
transporte. Lo que cambia es por dónde salen los bytes.

- **USB no lleva driver nuestro.** El transporte genérico es la cola RAW del SO (CUPS en macOS y
  Linux); el driver lo escriben Star y Epson, y ESC/POS ya es el lenguaje universal que generamos
  entero. Descartados por reintroducir el problema del driver: `libusb`, WebUSB, OPOS/JavaPOS.
- **Fase 1 = sin cola** para Bluetooth y USB: `PrintJob.target` es `NetworkTarget` y la política de
  reintentos de `queue` está escrita alrededor de un `connect` TCP. El fallo no se pierde: vuelve
  al llamante y el print host marca el trabajo `failed`. Matiz del USB: el `Ok` de `lp` significa
  que **el spooler aceptó el trabajo**, que es hasta donde ve `lp`; sin papel o con el cable fuera
  el trabajo se queda **retenido en la cola del SO** (con el título `ERPlora`, para reconocerlo
  allí) y `lp` sale 0 igual. Lo que sí vuelve como error es que el spooler lo rechace (cola
  inexistente, deshabilitada o que no acepta trabajos), con la frase de `lp` dentro.
- **El descubrimiento USB pregunta dos veces**: `lpstat -e` (los nombres de destino, que CUPS
  **nunca** traduce) y `lpstat -v` (el URI de cada cola, que dice el cable). La segunda **sí se
  traduce** —«dispositivo para …» en un Mac en español, y macOS elige el idioma por
  `AppleLanguages`, no por `LANG`/`LC_ALL`—, así que cada línea se casa contra los nombres de la
  primera en vez de contra una plantilla por idioma.
- **Windows queda fuera** (spooler RAW: `OpenPrinter`/`StartDocPrinter`/`WritePrinter`) — misma
  idea contra el otro spooler, pero no se puede verificar sin una máquina Windows real: hub#1269.

El escáner por HID lo sigue manejando el SO/navegador como teclado; por eso **no** hay
`scanner`/`keyboard`.

### Validar el USB contra hardware

En CI no hay ninguna impresora enchufada, así que el descubrimiento y la impresión reales se
comprueban a mano en la máquina que tiene el cable (`qa-hub-macos`):

```bash
cargo run -p erplora-peripherals --example usb_queue_probe            # lista, no imprime nada
cargo run -p erplora-peripherals --example usb_queue_probe -- <cola>  # página de prueba REAL
```

## Mapeo Python → Rust (qué portar y de dónde)

| Módulo Rust | Origen Python | Qué hace |
|---|---|---|
| `protocol` | `erplora_bridge/protocol.py`, `Protocol.kt` | Tipos `serde` de `Command`/`Event`/`PrinterInfo`/`Device`. **Elimina** `toggle_keyboard`, evento `barcode`, `keyboard_toggled`. |
| `discovery` | `hardware/discovery.py` (solo rama `network`) | `parse_printer_id`, escaneo de subred /24 al 9100, mDNS, enriquecido MAC. |
| `usb` | *(nuevo, hub#1083)* | Cola RAW del SO en macOS/Linux: `lp -d <cola> -o raw` y enumeración por `lpstat -v`. Sustituye al `_discover_usb` de `pyusb`, que sí era un driver. |
| `escpos` | `hardware/printer.py` + `_RawNetworkPrinter` (`discovery.py`) | Comandos ESC/POS (align/bold/size/cut/barcode) + renderizadores por `document_type`. |
| `drawer` | `hardware/drawer.py` | Kick ESC/POS pin 2/5 por el socket de la impresora. |
| `queue` | *(nuevo)* | Cola + reintentos (§2.7). En Python la impresión era síncrona. |
| `registry` | `hardware/network.py` + `hardware/watchdog.py` | Registro persistente `devices.json`, MAC vía ARP, watchdog de re-localización por DHCP. |

Lo que **no** se porta: `hardware/scanner.py` y `toggle_keyboard`/teclado virtual. El USB
(`pyusb`) y el Bluetooth (`pyserial`) no se portaron tampoco — volvieron por otra vía, sin driver
propio: la cola del SO y el plugin Kotlin.

## Estado

**Implementado**: los módulos (`protocol`, `discovery`, `escpos`, `drawer`, `queue`,
`registry`, `usb`, `lib`) tienen lógica real, no firmas con `todo!()`. `cargo test -p erplora-peripherals`
corre en verde (render ESC/POS contra bytes esperados, cola/reintentos contra un socket, apertura
de cajón, sonda del watchdog, discovery).

> 🪦 Hubo un módulo `printer` con una capa de traits (`Printer`/`CashDrawer`) y **su propia**
> política de reintentos, propuesta en pm#4 y nunca consumida: la app instalable encola por `queue`
> y abre el cajón por `drawer`. **hub#379** lo borró —dos políticas de reintento coexistiendo son
> una que nadie mantiene— y movió sus tests a las vías vivas, que hasta entonces no tenían ninguno.
