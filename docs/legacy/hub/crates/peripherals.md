> Copia histórica de `erplora-old/architecture/hub/crates/peripherals.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../../README.md).

# erplora-peripherals — lógica de hardware POS (tres transportes: red 9100 · SPP en Android · cola RAW del SO por USB en escritorio)

> ⭐ **ADR-0196 (2026-08-03) mueve la COLA de impresión de aquí al Hub.** La `PrintQueue` con
> reintentos se reutiliza, pero deja de vivir en el dispositivo: cualquier cliente —PWA incluida—
> encola `{role, html, jobId}` en el hub, y el dispositivo con la app registrado como *host de
> impresión* de ese `role` la drena. Si no hay cliente conectado, el trabajo **espera**. Este crate
> ya no tiene consumidor standalone: `apps/bridge` **murió** (hub#340, 2026-08-08). **La cadena del
> hub está COMPLETA** (comprobado el 2026-09-06, hub#1593): cola `_print_queue` (hub#341), registro
> de hosts `_print_host` (hub#342), drenaje por `GET /ws/print` (hub#343) y `sdk.print` **encolando**
> desde el shell (hub#344) — ver [`../print-queue.md`](../print-queue.md). Este crate ya no es el
> camino vivo por defecto: es el que SACA el papel en el dispositivo que drena la cola.

> Cargar este doc antes de editar `hub/crates/peripherals`.

`erplora-peripherals` concentra la **lógica de hardware POS reutilizable** del Hub —
impresión ESC/POS, apertura de cajón, control de caja, descubrimiento y registro de
dispositivos — como **librería sin servidor ni UI**. Renderiza ESC/POS una sola vez y lo entrega
por **el transporte que nombre el `printer_id`**: red (TCP **9100**), SPP en Android o la cola RAW
del sistema operativo por USB en escritorio. Es el reemplazo en Rust del Bridge Python
(`bridge/ERPlora-Bridge-desktop`) y hoy lo consume **un** cliente, que monta su propio
transporte: la app instalable (`apps/tauri`), por `invoke` in-process. Fundamento: [`../apps/bridge.md`](../apps/bridge.md) (reubicado desde `ARQUITECTURA.md §2.7`). Estado del crate: **implementado** — 2.459 líneas y 24 tests, **cero `todo!()`**
(contado el 2026-08-01). Este doc decía «esqueleto + porte en
curso» con `todo!()` donde la
implementación real es fase siguiente — ver `hub/crates/peripherals/README.md`).

## Consumidores y transporte

| App | Producto | Transporte que monta sobre este crate |
|-----|----------|----------------------------------------|
| 🪦 `apps/bridge` (standalone) — **borrado del árbol** en hub#340 (2026-08-08) | — | *Histórico*: servidor Axum `GET /status` + `WS /ws` (puerto `BRIDGE_WS_PORT = 12321`). Se quedó sin cliente en hub#339 al retirarse `WsBridgeTransport` (ADR-0196 §3) y el binario se borró en hub#340. No es un consumidor: la fila se conserva para que el `BRIDGE_WS_PORT` que sigue en las constantes tenga explicación |
| `apps/tauri` (el shell ES el bridge) | app instalable (Win · macOS · Linux · **Android**) | El shell expone el hardware por **`invoke` in-process** sobre este crate: **no** hay servidor localhost ni WS. La fila decía lo contrario («bridge embebido HTTP/WS», [ADR-0050](../../adr/ADR-0050.md)); lo cambió [ADR-0159](../../adr/ADR-0159.md) y estaba sin reflejar |

El **único** consumidor vivo es la app instalada, que habla el mismo protocolo JSON de siempre pero
por **`invoke` in-process** (ADR-0159/0196): el WS localhost de `apps/bridge` se fue con hub#340 y
el canal HTTP/WS compartido de ADR-0050 quedó superado. El crate no impone I/O de UI: cada
consumidor decide su transporte, y por eso siguen cabiendo más de uno.

> `crates/server` declara `erplora-peripherals` en su `Cargo.toml`, pero **solo como
> dev-dependency**: lo usa `tests/print_to_real_printer.rs`, donde el test hace de host de impresión
> contra una térmica real (hub#501). El servidor **no** depende de este crate en producción — el hub
> no toca hardware (`ARQUITECTURA.md §2.7`).

## Los tres transportes: de «red-only» a la cola del SO (§2.7)

El `printer_id` nombra el transporte, y `discovery::parse_print_target` es la **única** puerta que
lo decide. Hoy son **tres**, y el render ESC/POS es el mismo para los tres: lo único que cambia es
por dónde salen los bytes.

| `printer_id` | Transporte | Dónde | Decisión |
|---|---|---|---|
| `network:{ip}:{port}` | ESC/POS por TCP al **9100** | todas las plataformas | la vía original, y la única con cola y reintentos propios (`queue`) |
| `bluetooth:{mac}` | **Bluetooth Classic SPP** por el plugin Kotlin `tauri-plugin-erplora-android` | **solo Android** | [ADR-0204](../../adr/ADR-0204.md), hub#388 |
| `usb:{queue}` | la **cola RAW del sistema operativo** (`lp -d <cola> -o raw`) | **solo escritorio** (macOS y Linux, vía CUPS) | [ADR-0441](../../adr/ADR-0441.md), hub#1083 |

La decisión original era **red-only** y su motivo era bueno: «un driver por SO no escala» —`pyusb`,
udev, Zadig, la pelea con el driver del fabricante—. Ese motivo **sigue en pie**, y por eso la
puerta del USB no es una librería USB: **es la cola del sistema**. El driver lo pone el fabricante
(Star, Epson), el SO ya es dueño del cable y ESC/POS ya es el idioma que generamos entero, así que
se da USB **sin incumplir** la razón que lo mantenía fuera — es lo que razona
[ADR-0441](../../adr/ADR-0441.md), que enmienda el «red-only» de ADR-0196 §4.

**Sigue fuera, y por razones distintas:** **Windows** (el mismo diseño contra su spooler, pero sin
máquina real donde verificarlo — hub#1269), el **USB en Android** (exigiría Kotlin, permiso por
intent y cables OTG, y ahí la impresora barata ya la cubre el SPP), **`libusb`/WebUSB/OPOS** (cada
uno reintroduce el driver por SO) e **iOS**, que sigue red-only. El escáner por HID lo maneja el
SO/navegador como teclado: por eso **no** hay `scanner` ni `keyboard`, y respecto al protocolo
Python se eliminan `toggle_keyboard`, el evento `barcode` y `keyboard_toggled`.

En Android (hub#388, mergeado 2026-08-13, PR hub#922) `parse_print_target` valida y normaliza la
MAC en el borde, el descubrimiento fusiona las impresoras **EMPAREJADAS** del sistema
(`BluetoothSpp.kt`, bonded list + RFCOMM, sin `BLUETOOTH_SCAN`: emparejar es de los ajustes de
Android) y print/test/cajón rutean por target — red por la cola, BT directo al plugin. Los
dispositivos BT quedan explícitamente FUERA del watchdog/recovery ARP (fase 1 sin camino de salud,
por decisión del ADR). Validación en hardware real: hub#924 (tique end-to-end + lista de modelos).

> ⭐ **ADR-0204 (2026-08-06) revierte parcialmente la parte Bluetooth**: la premisa «Tauri-Android
> pierde el SPP» era errónea (un plugin Kotlin corre in-process con las APIs completas de Android).
> El **Bluetooth Classic SPP vuelve SOLO en Android**, como transporte de
> `tauri-plugin-erplora-android` ([hub#388](https://github.com/ERPlora/hub/issues/388), tras la
> cola-en-el-hub de ADR-0196 §6): `printer_id` ganó la variante `bluetooth:{mac}` — el test de
> `discovery.rs` que la rechazaba se invirtió al ejecutarlo. **Ejecutado el 2026-08-13.** Ese ADR
> dejaba el **USB descartado** y escritorio e iOS red-only; esa parte la levantó **ADR-0441**
> (2026-09-04) solo para escritorio, y con ella nace `usb.rs`.

### La cola RAW del SO: «enviado» no es «impreso» (hub#1083 · #1541 · #1564)

`usb.rs` habla con CUPS por cuatro clientes de línea de órdenes del mismo paquete (`cups-client`;
macOS lo trae), así que el transporte **no añade dependencia** ninguna. Los tres pasos son
contrato, no detalle de implementación: sin el primero y el tercero, una térmica sin papel devolvía
un tique dado por impreso.

| Paso | Cómo | Por qué así |
|---|---|---|
| **1. Antes — ¿esta cola puede imprimir?** (hub#1541) | `lpoptions -p <cola>`, y se leen los atributos **IPP** `printer-state` (3 idle · 4 processing · 5 stopped, RFC 8011 §5.4.11) y `printer-state-reasons` → `ready` / `stopped` / `unknown` | Se pregunta a `lpoptions` y **no** a `lpstat -p` porque `lpstat` contesta con una **frase** y CUPS la traduce; `lpoptions` contesta con el atributo, y un enum es un enum en todos los idiomas. Antes la lista enseñaba un `"ready"` fijo |
| **2. Enviar** (hub#1083) | `lp -d <cola> -o raw`, con los bytes ESC/POS por stdin | `-o raw` marca el trabajo `application/vnd.cups-raw` y CUPS **salta sus filtros**: entregamos ESC/POS ya renderizado, no un PDF que un driver reinterprete |
| **3. Después — ¿salió de la cola?** (hub#1564) | Se lee el `request-id` de la respuesta de `lp` y se espera (`JOB_COMPLETION_WINDOW`, 15 s) a que ese trabajo desaparezca del listado; si sigue ahí se lee la avería y se `cancel`a | Un trabajo retenido no es papeleo inofensivo: es un tique que sale **por sorpresa** horas después, cuando alguien recarga el rollo, con un número que la caja ya reportó como fallido y el cliente ya en la calle |

Matices que costaron un fallo y por eso hoy son test:

- **Solo `-error` para una caja.** De las tres severidades IPP (`-error`, `-warning`, `-report`)
  solo la primera puede parar un TPV: un rollo bajo reporta `media-low-warning` **durante horas**
  mientras imprime perfectamente.
- **`printer-state` ausente es `unknown`, no `ready`.** La ausencia de veredicto es justo lo que el
  `"ready"` fijo estaba escondiendo; `unknown` no convierte en fallo un tique que sí se imprimió.
- **La ventana de espera dura MÁS que la cola del propio backend.** El backend USB retiene un tique
  **ya impreso** unos segundos tras el último byte (medido: ~8 s), así que 15 s se eligen por encima
  de eso: por debajo se cancelaría un tique que sí había salido.
- **El listado se casa por el id del trabajo, nunca por la fecha**, que CUPS traduce — la misma
  trampa de localización que ya puso `lpstat -v` (hub#1083) y `lpstat -p` (hub#1541).
- **El nombre de cola se valida en el borde** (`is_cups_queue_name`: ≤127 bytes, sin espacio, TAB,
  `/` ni `#`, y sin `-` inicial). `lp -d -algo` leería un flag, y un tique que se imprime **en otro
  sitio** es peor que uno que falla.
- **El descubrimiento pide DOS listados**: `lpstat -e` para los nombres (no localizado) y
  `lpstat -v` para los cables (`usb://`), porque el segundo **sí** se traduce. Una cola que aparece
  en `-v` pero no en `-e` no se ofrece.

## Módulos del crate

Fuente: `hub/crates/peripherals/src/`.

| Módulo | Responsabilidad | Origen Python |
|--------|-----------------|---------------|
| `protocol` | **Fuente única de verdad** del contrato JSON Hub↔Bridge: `Command` (tag `action`), `Event` (tag `event`), `PrinterInfo`, `Device`. | `protocol.py`, `Protocol.kt` |
| `discovery` | `parse_printer_id` (`network:{ip}:{port}`), escaneo de subred /24 al 9100 (254 sondas concurrentes), mDNS (`_pdl-datastream._tcp`, `_ipp._tcp`), dedup, enriquecido MAC. Desde **ADR-0235** el escaneo declara qué le deja el SO (`LocalNetworkAccess`) y responde un `PrinterDiscovery`, no un `Vec`. | `discovery.py` (rama network) |
| `escpos` | Render `document_type` + `data` → bytes ESC/POS (`EscposBuilder`: align/bold/size/cut/barcode/total_line; cp437 vía `oem_cp`). | `printer.py` + `_RawNetworkPrinter` |
| `drawer` | Kick ESC/POS del cajón por el socket de la impresora (pin 2 = `ESC p 0 25 50`, pin 5 = `ESC p 1 25 50`). | `drawer.py` |
| `queue` | Cola de impresión asíncrona con **reintentos/backoff** (`RetryPolicy` default 3 intentos / 2000ms). `enqueue` contesta al entregar; `print` contesta lo que pasó con el papel (Ok o «impresora inalcanzable» tras los reintentos), y es la que usa la app instalada desde ERPlora/hub#2494. Nuevo: en Python la impresión era síncrona. | *(nuevo, §2.7)* |
| `usb` | Cola RAW del SO por CUPS (ADR-0441): descubrimiento (`lpstat -e`/`-v`), estado previo (`lpoptions -p`), envío (`lp -o raw`) y confirmación posterior con cancelación del retenido. | *(nuevo, ADR-0441)* |
| `registry` | Registro persistente `devices.json` (`key` → `Device`), resolución MAC vía ARP, **watchdog** (health-check + recovery scan por MAC tras cambio DHCP). Dos puertas de alta según la identidad: `register` para lo que tiene socket y `register_queue` para una cola del SO. | `network.py` + `watchdog.py` |

#### La identidad de un dispositivo en `devices.json` (hub#1536)

La `key` de una entrada **no es siempre una MAC**. Es lo único estable que ese transporte tiene:

| Transporte | `key` | Por qué |
|---|---|---|
| Red / Bluetooth | La MAC normalizada, o el `printer_id` (`network:{ip}:{port}`) si ARP no la resuelve | El fallback existe porque sin él un equipo sin MAC no llegaba a registrarse y **no se le podía dar un rol**; pasa siempre en Android y con VPN/contenedores/firewall |
| Cola del SO (`usb:{queue}`) | El `printer_id` **entero** | No tiene MAC que resolver ni `ip:port` al que caer, así que `device_key` le daba `network::0` a **todas**: la segunda cola registrada pisaba a la primera y heredaba el rol que su dueño le había puesto. El nombre de destino de CUPS ya es único en la máquina, así que **ES** su identidad |

Consecuencias que son contrato, no detalle:

- **Una cola descubierta se registra** (`register_discovered_queues`, en el shell, junto a las
  ramas de red y Bluetooth) y por tanto **admite rol** (cocina/barra/caja) como cualquier otra
  impresora. Antes `set_role("usb:…")` respondía «imprime, pero todavía no se le puede dar un rol».
- **`register` rechaza** el caso sin MAC y sin dirección en vez de escribir la clave colapsada: por
  ahí volvería la colisión, y pisar el rol de otro en silencio es exactamente el fallo. Una
  impresora Bluetooth emparejada **sí** se registra con `ip` vacía (ADR-0204): su MAC es su
  identidad.
- **Las claves que ya están en disco no se mueven.** `devices.json` es dato de usuario y una entrada
  cuya clave cambiara perdería el rol que le puso su dueño: la migración de lo viejo es que **no
  hay** migración.
- El `printer_id` de una cola se valida con el **mismo** parser que el camino de impresión
  (`discovery::parse_print_target`): un id sin cola detrás crearía una entrada fantasma que la
  siguiente cola heredaría, rol incluido. Un fallo se **loguea y se salta** —un descubrimiento es un
  lote— y nunca aborta el resto.

### Tipos de documento (`escpos::DocumentType`)

`receipt` · `kitchen_order` · `invoice` (el cuerpo del tique + lo de la factura completa, hub#2005) · `delivery_note` ·
`barcode_label` · `cash_session_report` (cierre de caja) · `prebill` (la CUENTA de la mesa) ·
`generic`. El "control de caja" se materializa como el documento `cash_session_report`
(apertura/cierre/diferencia/transacciones).

**`prebill` NO delega en `receipt`** (hub#748), al contrario que `invoice`: lo que la cuenta no
puede llevar —número de serie, QR VeriFactu, método de pago (ADR-0141)— es justo lo que un tique
existe para llevar, así que compartir el cuerpo dejaría el mobiliario fiscal a un `if` olvidado de
distancia del papel que el camarero entrega. Se cae en el renderizador, no confiando en el
productor, y el aviso «no es una factura» tiene valor por defecto. Detalle en
[`../print-queue.md`](../print-queue.md#la-cuenta-prebill-no-es-un-tique-y-el-papel-lo-dice-hub748).

**Una cuenta SIN `items` se rechaza**, no sale en blanco: el renderizador lee por clave, así que un
documento con la forma de la PANTALLA (`business.name`, `lines[]`) imprimiría «ERPlora», sin líneas
y TOTAL 0,00 sin dar error. Papel equivocado es peor que papel que no sale — nadie revisa una cuenta
que sí se imprimió.

**`invoice` es una FACTURA COMPLETA en el rollo de 80 mm, o no sale** (hub#2005). Antes caía en el
cuerpo del tique, que no lee el NIF del cliente ni un desglose por tipo: el cliente que pedía su
factura se llevaba un tique y nada lo decía. Los TPV españoles (Ágora, Revo, Glop) imprimen la
factura completa en rollo, válida si lleva todos los datos del RD 1619/2012 art. 6; así que
`render_receipt(…, Fiscal::FullInvoice)` añade el título `FACTURA`/`INVOICE` (negrita, doble alto,
antes de los datos), numera con `Factura: ` en vez de `Ticket: `, identifica al cliente bajo su
nombre y pinta el desglose antes de los totales. **El contrato de `data`** (lo valida
`check_full_invoice` antes de renderizar):

| Campo | Obligatorio | Papel |
|---|---|---|
| `vat_number` | sí | NIF del emisor, bajo el nombre del negocio |
| `receipt_id` | sí | `Factura: <número>` |
| `customer_name` | sí | `Cliente: <nombre>` |
| `customer_tax_id` | sí | `NIF: <nif>`, justo debajo del nombre |
| `customer_address` | no | `Dir: <dirección>`, debajo del NIF |
| `tax_breakdown` | sí, array no vacío | una fila por tipo: `{ rate, base, tax, label? }` |
| `rectifying` | no | `true` → el título es `FACTURA RECTIFICATIVA`/`CORRECTIVE INVOICE` (hub#2381) |

`rate` es el porcentaje (`21`, `5.2`, se imprime sin ceros de cola), `base` y `tax` van en la misma
unidad que `total`, y `label` sustituye al nombre por defecto `IVA <rate>%` (p. ej. un recargo de
equivalencia). Filas en columnas fijas de 8+12+12 = 32 (`Tipo  Base  Cuota`); una etiqueta más
ancha que su columna va en su propia línea. El resto de campos son los del tique. **Si falta
alguno, `render_document` devuelve `InvalidPayload` nombrando el campo** y el host de impresión
marca el trabajo como fallido (la campana lo avisa, hub#987/#1731): papel que no es factura no se
corta como si lo fuera. El `receipt` no cambia ni un byte. Que los productores manden estos campos es
sales#350 (PR sales#353) e invoice#86; la A4 → PDF en la app instalada, hub#2006. Decisión: ADR-0509.

**Una rectificativa lo dice en el título** (hub#2381). Una R1–R4 sale con el total en negativo y,
titulada `FACTURA` como cualquier otra, se leía de un vistazo como una factura más; la A4, Holded,
Odoo y Business Central la titulan «Factura rectificativa». Con `rectifying: true` el título pasa a
`Label::RectifyingInvoiceTitle`, mismo sitio y mismo peso (21 columnas, cabe en la línea). Como
`duplicate`, solo el booleano `true` la enciende; sin la clave el papel no cambia y un hub anterior
la ignora. La manda `invoice` para las R1–R4 (invoice#131). Una R5 (simplificada) sale
como `receipt`, que no lleva título: la clave no la toca.

**Un string desconocido se RECHAZA** (`DocumentType::parse` → `None` → `UnknownDocumentType`).
Hubo una puerta laxa que lo mapeaba a `Generic` —`from_wire`, el `else` de Python—, y esa
degradación silenciosa es justo lo que arregló hub#501: un `Kitchen` o un `kitchn` salía como un
volcado de pares clave/valor sin nombre, y no se descubría hasta que faltaba el plato. Su único
llamador era `apps/bridge` 🪦; con él fuera (hub#340), **hub#578 borró la puerta**. `generic` es
hoy un documento que se PIDE, nunca uno al que se cae.

**Una reimpresión dice «DUPLICADO»** (hub#1931). El RD 1619/2012, art. 14, solo permite un
original de cada factura (el tique es una factura simplificada) y obliga a que toda copia diga
«duplicado». `render_receipt` —y con él `invoice`— pinta la marca centrada y en negrita encima de
los datos del tique cuando el documento trae `duplicate: true`, en el idioma del papel
(`DUPLICATE`/`DUPLICADO`, catálogo `Label::Duplicate`). Solo el booleano `true` la enciende: es una
declaración legal sobre el papel, no un adorno que un string «verdadero» pueda activar. **Quién
decide es el productor**, que es quien sabe si imprime el original o una copia: `sales` la pone en
toda impresión de su visor fuera del cobro, y justo después de cobrar solo el PRIMER papel de la venta
es el original —el de la impresión automática (`printableDocument()` sin argumentos, hub#1921) o el
del botón del tique, el que llegue antes—; los siguientes son duplicados (sales#330). Un `erplora-app` antiguo
ignora la clave y saca el papel de antes.

### La comanda de cocina (`kitchen_order`): qué lleva y cómo se jerarquiza

Dos productores, los dos en el shell (no el módulo — imprimir es client-side, ADR-0017/0144), y
los dos con una hoja por **rol de impresora**, cada hoja un `render_document` propio:

- `apps/web/src/lib/print-comanda.ts`: la comanda al disparar la ronda (`kitchen.order.created`,
  HUB_SHELL-F72).
- `apps/web/src/lib/print-void.ts` (kitchen#168, HUB_SHELL-F78): el **vale de anulación** al
  cancelar una ronda ya enviada (`kitchen.order.cancelled`, a mano o porque se eliminó la cuenta).
  Es **el mismo documento `kitchen_order`, sin campo nuevo**: el renderizador vive en `erplora-app`
  instaladas que no se actualizan, y un campo que una app vieja ignorase haría salir el vale como
  una comanda NUEVA (el plato se cocinaría dos veces). El «anulado» se dice con lo que la app vieja
  ya pinta: `label` pasa a «ANULADA · Mesa 4» (doble alto) y cada `items[].quantity` va en
  **negativo** («-2x Croquetas»); `fmt_qty` conserva el signo (test
  `a_void_slip_prints_the_word_and_negative_quantities`). Nunca lleva `priority`. Clave de cola
  propia (`kitchen-void-<pedido>-<rol>`) para que la cola no lo tome por repetido de la comanda.
  Desde hub#2640 también escucha **`kitchen.item.voided`** (KITCHEN-F29, el TPV anula UN plato ya
  enviado): un vale por **línea de venta** (`sales_order_item_id`; un menú anulado entero llega como
  un aviso por componente y sale un vale por rol con todos ellos, sin los ya servidos), con `label`
  «PLATO ANULADO · Mesa 4» —no «ANULADA», que en papel se lee como la mesa entera— y clave
  `kitchen-void-<pedido>-<línea de venta>-<rol>`; un `Set` por pestaña evita el segundo vale si el
  aviso llega repetido. El vale de la ronda deja fuera las líneas `voided` (ya tuvieron el suyo):
  si el plato anulado era el último vivo, la ronda se cancela y no sale vale de ronda encima.

**Campos que lee `render_kitchen_order`** (hub#1156). Todos opcionales: lo que no llega, no se
pinta, y una app vieja que no los mande imprime exactamente la hoja de siempre.

| Campo | Qué es | Cómo sale |
|---|---|---|
| `receipt_id` | Nº de comanda | `#K-217`, centrado |
| `label` | La etiqueta OPACA de sala (ADR-0141/0144): «Mesa 4», «Barra», «Recogida Ana» | **Tal cual, sin prefijo**, negrita + doble alto + **doble ancho** |
| `table` | Forma **legacy** — hoy no la manda nadie | `Mesa: 4`, con su prefijo de siempre |
| `waiter` | **Quién disparó la ronda** (hub#1410), para que cocina sepa a quién llamar cuando el plato sale mal o va tarde | `Camarero: Ana`, sin realce |
| `round_number` | Nº de pase de esa mesa | `Ronda 2` — **sólo a partir de la 2** |
| `items[].modifiers` | Suplementos congelados en la fila (pm#93) | Sangrado bajo su plato, **en negrita** |
| `items[].notes` | Nota libre del camarero | `>> …` sangrado, sin realce |
| `items[].combo_ref` / `combo_name` | El MENÚ del que la línea es componente (ADR-0381) | Cabecera **sin negrita** + componentes sangrados |
| `priority` | Urgencia del pase — hoy **inalcanzable**: el vocabulario de `kitchen` es `normal`/`rush`/`vip`, nada casa con `HIGH` y el productor no manda el campo (hub#1411) | `!! URGENTE !!` **al pie**, centrado, negrita + doble alto — **solo con la forma exacta `HIGH`** |

⚠️ `label` y `table` conviven a propósito: `label` gana, `table` sigue funcionando. El contrato del
dispositivo **se amplía, nunca se sustituye** — `erplora-app` ya desplegada lee esta forma.

⚠️ `waiter` es un **nombre ya resuelto**, no un id. La comanda guarda un `waiter_id` opaco y
`kitchen` **no** lo une con `hub_user` a propósito (kitchen#63 —la regla quedó escrita en
`queries/orders_display.sql` del módulo—; ADR-0192: el nombre es presentación y unir una tabla del
core desde una query de módulo ataría el pase a la forma de las tablas del hub). Lo resuelve el
productor contra `hub.users.list` y, si ahí no está, contra el equipo de la app Personal
(`staff.members.list`, lectura **opcional** por `queryAllOptional`, hub#2033): desde sales#318/#320
la caja puede decir que atiende una ficha del equipo **sin usuario**, y entonces el `waiter_id` es el
id de esa ficha. Mismo criterio que la tarjeta del KDS (kitchen#82): `full_name` o `first_name +
last_name`, sin filtrar bajas ni inactivos (quién disparó la ronda es un hecho histórico); sin la
app Personal, sin permiso o con error, nada. Y **si no hay nombre no manda el
campo**: un UUID en el papel es peor que el hueco, porque el cocinero lo lee a dos metros, no puede
usarlo y deja de fiarse de la cabecera. Y si cualquiera de las dos lecturas **falla**, la comanda sale igual, sin
nombre: el papel del pase no puede depender de una consulta de presentación.

#### La jerarquía del papel se hace con SANGRADO, y el realce se reserva a los CAMBIOS

Decisión de mercado (15 productos + 8 hilos de foro, kitchen#57 / hub#1156). Es la traducción a
térmica de la regla de pantalla «el realce es para alérgenos y cambios, **nunca** para jerarquía».

```text
COCINA
#K-217
================================
Mesa 4                            ← negrita + doble alto + doble ancho
Camarero: Ana                     ← sin realce
Ronda 2
Hora: 14:32
--------------------------------
2x Croquetas                      ← negrita + doble alto  (col 0)
   SIN CEBOLLA                    ← NEGRITA, altura normal (col 3)
   >> para compartir              ← sin realce            (col 3)
MENU DEL DIA                      ← SIN NEGRITA, normal   (col 0)
  1x Gazpacho                     ← negrita + doble alto  (col 2)
  1x Entrecot
     Al punto                     ← NEGRITA               (col 5)
================================
```

Las cuatro reglas, con su porqué:

1. **Jerarquía = sangrado.** Niveles 0 → 2 → 5, **dos como mucho**: a 32 columnas un tercero deja
   el texto sin sitio. Simphony documenta los componentes *«indented beneath the combo meal name»*
   y dice explícitamente que vale **en modo chit**; Aloha lo tiene como parámetro numérico
   (`QC indentation size`, 8-100 px). El antipatrón con nombre es Square: el combo sale como
   *«one long run-on paragraph, which makes it kind of hard to decipher when in the kitchen»*, con
   la petición abierta años y «comprar otra impresora» como único remedio real.
2. 🔴 **La cabecera del menú renuncia a la NEGRITA; no se encoge.** Un menú realzado le roba el ojo
   a lo que hay que cocinar. De las 15 referencias, **cero** imprimen el nombre del combo en doble
   altura y **cero** en Font B. Eats365 da tamaño propio a seis campos del chit —nº de pedido,
   mesa, notas, etiquetas de fuego…— y al nombre del combo sólo un interruptor de visibilidad:
   cuando un fabricante te da tamaño para seis cosas y para la séptima sólo «sí/no», te está
   diciendo que esa séptima no es una instrucción de cocina.
3. **El realce (negrita) es para los CAMBIOS.** El suplemento sí va en negrita: es lo que hace que
   el plato vuelva. Toast imprime el plato en negro y el modificador en **rojo**, Lightspeed tiene
   `print sub-items in red`, Revel igual — tres de tres realzan el cambio. En una térmica sin cinta
   bicolor, el equivalente es `ESC E`. La nota libre del camarero **no** es un cambio de la carta:
   se lee, no se grita.
4. **La mesa es lo más grande del papel** (doble alto + doble ancho). Es el único campo que se lee
   desde el **otro lado del pase**, a velocidad, para casar un plato con su destino; el plato se lee
   a distancia de brazo. Toast lo pone *«in large, bold font in the top-left corner»*. Renuncia al
   doble ancho si no cabe en 32 columnas: una etiqueta partida en dos trozos por la impresora es lo
   contrario de un campo que existe para leerse de un vistazo.

❌ **Nunca `ESC M 1` (Font B) en el cuerpo de la comanda.** No existe el consenso «Font B = apagado»:
su papel documentado es **caber en más columnas** (42 vs 32), que es una solución a un problema de
espacio, no una señal de jerarquía. Y cambiaría el ancho a mitad de ticket, rompiendo las reglas de
`====` y cualquier alineación. La prueba más limpia es Lightspeed: sus plantillas de cocina **no
exponen ni un control de tamaño de fuente** — toda su jerarquía son márgenes, reglas y color.
Además, el modo de fallo del papel es el **opuesto** al de la pantalla: el hilo de Square pidiendo
comandas MÁS GRANDES lleva años abierto porque a un metro y entre vapor se compite por
legibilidad, no por atención.

❌ **Sin precios**, unánime. *«the kitchen has no reason to see the … value of a dish»*.

**La cabecera se repite en CADA hoja de estación** que reciba al menos un componente — opción
`11 - Send to Combo Parent Order Devices` de Simphony; TouchBistro lo hace por defecto y sin
opción. Un cocinero de la plancha que no lee «MENÚ» no sabe que su entrecot va acoplado a un
gazpacho, y lo saca cuando le viene bien. El modo huérfano (el componente sin el nombre del padre,
`Print or show modifier(s) only` de Toast) es al que se entra **a propósito**, no el defecto.

**Sin `combo_name` no se imprime cabecera de relleno**: el sangrado agrupa igual y no hace falta
una cadena más. (Cuando se escribió esto el renderizador no traducía nada; desde hub#1159 las
etiquetas que SÍ posee salen del catálogo `(en, es)` de abajo.)

⚠️ **`modifiers` llega hoy como UNA cadena ya compuesta** por `kitchen` (`modifiers_for_display`,
unida con `", "`), así que en papel sale envuelta, no una por línea. El renderizador ya lee la
**lista** (hub#1138, tabla de abajo); que `kitchen` la entregue es kitchen#64 — y **tiene que
esperar a la flota de imágenes** (ver «Orden de despliegue» abajo).

### El IDIOMA del papel: catálogo en el crate, `locale` en el documento (hub#1159)

Todas las etiquetas del renderizador estaban **cableadas en español** (se portaron así desde
`printer.py`): un hub con el idioma en otra cosa pintaba el KDS traducido y sacaba por la térmica
una comanda en español. La comanda y el tique son lo que ve **el cliente y la cocina**, así que es
justo la superficie que tiene que hablar el idioma del negocio (ADR-0055/0199).

**El catálogo vive en el crate y el idioma viaja en el documento** (`data.locale`). Se descartó la
alternativa —que cada productor mande sus ~30 etiquetas ya traducidas dentro de `data`— porque
reparte un catálogo entre N repos de módulo, convierte «añadir una etiqueta» en un cambio de N
repos, y compra un tique a medio traducir por cada módulo que se deje una. Es el mismo modelo que
usa Odoo: al informe se le pasa el `lang` y el catálogo de la plantilla resuelve las cadenas.

- **Inglés es la FUENTE, `es` la traducción** (ADR-0055/0199) — pero el **fallback es `es`**: un
  documento sin `locale` (que es toda `erplora-app` desplegada hoy) tiene que imprimir el papel de
  hoy **byte a byte**. El contrato del dispositivo se amplía; no se mueve bajo una flota que no se
  actualiza tan rápido como un módulo.
- Idiomas soportados: `es` y `en` (los mismos que `hub_settings.language`). Uno que el dispositivo
  no lleve **degrada al papel de hoy**, no rechaza el trabajo: papel que no sale pierde un servicio.
  La región se descarta (`es-ES`, `en_GB` → el idioma antes del separador).
- `locale` es campo del **sobre**, no una línea del documento: `render_generic` —que imprime toda
  clave que no conoce— lo salta explícitamente.
- Quien lo sella es la **cola**, una vez, para todos los productores:
  [`../print-queue.md`](../print-queue.md#el-idioma-del-papel-se-sella-en-la-cola-hub1159).

### La escala de la moneda: `decimals` en el documento (hub#2129)

Los importes llegan en unidades mayores (`float`), ya divididos por el productor según la escala de
la moneda; el renderizador solo necesita saber **cuántas cifras** enseñar. Lo lee de
`data.decimals` (`money_decimals`) y todos los importes pasan por `EscposBuilder::money`: líneas,
`total_line` (subtotal, IVA, descuento, total, entregado, cambio, cierre de caja), el desglose de
IVA de la factura y el precio de la etiqueta. El `%` del tipo de IVA no es dinero y no cambia.

- **Sin `decimals`, o con uno que no sea un entero de `0..=4`** (el rango de ISO-4217): dos
  decimales, el papel de siempre **byte a byte**. Un documento que pida nueve no se cree.
- Quien lo sella: la cola ([`../print-queue.md`](../print-queue.md#la-escala-de-la-moneda-se-sella-en-la-cola-hub2129))
  y, en la vía directa, la puerta de impresión del shell.
- **Orden de despliegue: da igual.** Una imagen vieja ignora la clave e imprime dos decimales como
  hoy; ninguna combinación de versiones empeora el papel.

### Los suplementos se leen como LISTA (hub#1138)

`item.modifiers` admite **dos formas** y las dos tienen que llegar al papel:

| Forma | Quién la manda | Qué imprime |
|---|---|---|
| `"Extra queso, Sin cebolla"` (string) | lo que compone `kitchen` hoy (`modifiers_for_display`) | **una** línea, partida a mano conservando el sangrado |
| `["Extra queso", "Sin cebolla"]` o `[{name}]` | la lista que pide hub#1138 | **un renglón por suplemento**, cada uno con su sangrado |

Antes, un array era *truthy* pero no era string, así que `str_field` devolvía `""` y los suplementos
**desaparecían del papel sin dar error** — la caída silenciosa contra la que avisa sales#78, y el
peor fallo posible en papel que nadie relee.

El string se queda como **una** entrada a propósito: partirlo por la coma aquí sería adivinar un
límite que pertenece al módulo, y el nombre de una opción puede llevar coma.

**Precedencia en el tique y en la cuenta** (`render_receipt` / `render_prebill`). Conviven tres
claves y no son lo mismo:

| Clave | Qué dice | Cómo sale |
|---|---|---|
| `components[]` | de qué se compone el **MENÚ** (ADR-0396, hub#1165) | un renglón sangrado, sin importe |
| `modifiers` | qué **CAMBIÓ** el cliente (hub#1138) | un renglón sangrado, sin importe, **después** de los componentes |
| `notes` | texto libre del camarero | `  > …`, **sólo si no hay ninguna de las dos listas** |

Componentes y suplementos son **ejes distintos**, así que se imprimen **los dos** —componentes
primero, el cambio después, el mismo orden que la comanda—. `notes` se aparta ante **cualquiera** de
las dos listas porque `sales` sigue encadenando ahí el mismo texto, y decirlo dos veces es el
defecto, no el arreglo. Un ítem sin ninguna lista imprime **byte a byte** como antes.

**Sin importe propio**: en ERPlora el delta ya vive dentro del `unit_price` de la línea
(`authoritative_modifiers`), así que el `line_total` impreso ya lo incluye y un segundo número en esa
columna no sumaría con los demás (sales#148; el contraejemplo es el bug abierto de Odoo #187509).

**Orden de despliegue: DISPOSITIVO primero, módulo después.** Los módulos se auto-actualizan; la
imagen del hub NO (el puntero solo se mueve con `promote` manual). Una imagen **vieja** que reciba
un array lo evalúa *truthy* pero no-string: `str_field` devuelve `""` y los suplementos desaparecen
del papel **sin error** — el mismo defecto de hub#1138, recreado al revés. Por eso el volteo de los
módulos quedó FUERA de hub#1138 a propósito y tiene sus issues propias:

- **kitchen#64** — la comanda pasa a mandar la lista **solo** cuando la flota de imágenes lleve el
  renderizador de hub#1159/#1138.
- **sales#229** — el tique y la cuenta mandan **las dos claves** (`modifiers` lista + `notes`
  encadenado): eso es seguro YA, porque la imagen vieja imprime `notes` como hoy y la nueva imprime
  la lista y aparta `notes`. Quitar `notes` sí exige el gate de flota de kitchen#64.

### El texto libre del negocio se parte por PALABRAS (hub#2462)

El encabezado (`receipt_header`), el pie (`receipt_footer`), la dirección (`business_address`, en
el tique, la factura y la cuenta) y las notas de los QR (`claim_note`, `promo_note`) son prosa que
escribe el negocio o el productor, de cualquier largo. Impresa como una sola línea, la impresora la
dobla por su columna sin mirar dónde, y el cliente recibía palabras partidas: «…de 2,00 E» /
«UR, ya aplicado…» en un rollo de 58 mm, «ya apl» / «icado…» en uno de 80 mm a 42 columnas.

Ahora pasan por `print_wrapped`: cada línea propia del texto (los `\n` que trae se respetan, también
una línea en blanco puesta a propósito) se reparte por palabras a las **32 columnas** del papel con
`wrap_to_width`, el mismo helper del aviso de la cuenta. Como el renderizador compone a 32 columnas
para cualquier rollo, una línea que cabe en 58 mm cabe también en 80 mm (42 o 48 columnas) y ninguna
impresora tiene que doblarla. `wrap_to_width` mide el texto **tal como llega al papel**
(`to_printable`): `€` sale como `EUR`, tres columnas por un carácter. Un texto que ya cabía sale en
una línea, igual que antes.

## Flujo de un trabajo de impresión

```mermaid
sequenceDiagram
    participant Hub as Hub (bridge.js)
    participant App as app instalada (invoke in-process, ADR-0196)
    participant P as protocol
    participant E as escpos
    participant Q as queue (PrintQueue)
    participant Pr as Impresora (TCP:9100)

    Hub->>App: Command::Print { printer_id, document_type, data, job_id }
    App->>P: deserializa Command (tag "action")
    App->>E: render_document(DocumentType, data) → Vec<u8> ESC/POS
    App->>Q: print(PrintJob { target, payload }) — espera el resultado (ERPlora/hub#2494)
    Q->>Pr: send_once (TcpStream::connect + write_all)
    alt éxito
        Q-->>App: JobOutcome::Completed
        App-->>Hub: Event::PrintComplete { job_id }
    else fallo (apagada / fuera de la red; sin papel no se detecta por el 9100)
        Q->>Q: reintenta hasta max_attempts (backoff_ms)
        Q-->>App: JobOutcome::Failed { error }
        App-->>Hub: Event::PrintError { job_id, error }
    end
```

## Descubrimiento: «no me dejan mirar» ≠ «no hay impresoras» (ADR-0235)

`discover_printers` **no devuelve un `Vec`**: devuelve un `PrinterDiscovery`, y exige que quien
llama declare qué le permite el SO.

```rust
pub enum LocalNetworkAccess { Granted, Denied { permission: String } }

pub enum PrinterDiscovery {
    Scanned { printers: Vec<PrinterInfo> },          // status: "scanned"
    PermissionDenied { permission: String },          // status: "local_network_permission_denied"
}

pub async fn discover_printers(registry: &DeviceRegistry, access: LocalNetworkAccess)
    -> Result<PrinterDiscovery>;
```

- **Por qué la firma pide el permiso.** El crate no puede preguntárselo al SO: cada plataforma lo
  guarda en otro sitio (permiso de runtime `ACCESS_LOCAL_NETWORK` en Android API 37+, consentimiento
  de red local de macOS 15). Pedirlo por parámetro convierte «suponer que hay permiso» en una
  **declaración explícita** de cada consumidor, comprobable en el compilador.
- **`Denied` NO escanea.** Sin permiso el barrido son 254 timeouts y una ventana mDNS que no resuelve
  nada: solo puede devolver vacío. Saltárselo ahorra 1,5 s y —lo importante— deja la razón en el
  valor de retorno. Hay test que lo prueba por tiempo (responde dentro de 200 ms, muy por debajo de
  la ventana mDNS) y por efecto (no registra ni un dispositivo).
- **`scanned_printers() -> Option<&[PrinterInfo]>`.** `Some(&[])` = miramos y no había nada;
  `None` = nunca llegamos a mirar. Que sean tipos distintos es todo el punto: un `Vec` vacío no
  puede expresar la diferencia y por eso se perdía.
- **Un solo literal de cable**: `LOCAL_NETWORK_PERMISSION_DENIED = "local_network_permission_denied"`
  es a la vez el `status` del outcome (camino `invoke`) y el `code` del `Event::Error` del WS del
  bridge. La pantalla aprende a ramificar **una** vez.

Quién lo consume, y con qué:

| Consumidor | Qué declara | Qué devuelve al Hub |
|-----------|-------------|---------------------|
| `apps/tauri` (shell) | `local_network_access(check_permissions())` — lee el mapa del plugin Android **sin** volver a preguntar (la PWA ya pidió el permiso antes de invocar) | El outcome tal cual (`{status,…}`) |
| 🪦 `apps/bridge` (standalone) — *borrado en hub#340* | `Granted` explícito: no había permiso de runtime que consultar | `Event::Printers` o `Event::Error { code }` |
| `@erplora/module-sdk` | — | `discoverPrinters()` **rechaza** con `LocalNetworkPermissionDeniedError` en vez de resolver `[]` |
| `apps/web` (shell) | — | Traduce ese rechazo a la frase de i18n (`hardware.printersBlocked`) |

> ⚠️ **Un mapa que no menciona el permiso no es una denegación.** Escritorio contesta un mapa vacío,
> y un Android anterior a la API 37 tampoco lo conoce. Leer ese silencio como «denegado» dejaría el
> TPV clavado en una plataforma donde ese permiso no existe — la misma mentira mirando al otro lado.

## Alta por IP tecleada: la salida cuando el escaneo no la ve (hub#1924)

El barrido solo cubre la `/24` del propio dispositivo y mDNS no cruza el router: una impresora en
otra subred o en una Wi-Fi de invitados aislada **nunca aparece**. Como en cualquier TPV (Square
«Add printer → Network → IP address», Loyverse, Toast, Odoo POS), el dueño escribe la IP que sale en
la hoja de configuración de la impresora.

```rust
pub const MANUAL_PRINTER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub async fn add_network_printer(registry, host, port, probe_timeout) -> Result<PrinterInfo>;
pub fn with_manual_printers(outcome: PrinterDiscovery, registry) -> PrinterDiscovery;
```

- **Se prueba antes de guardar.** `add_network_printer` exige una IPv4 con puntos y un puerto ≠ 0
  (si no, `InvalidPrinterId`, sin conectar), abre **un** `connect` TCP con 3 s de margen (más que
  el 1 s del barrido: es una sola conexión, a menudo a otra subred) y **solo si responde** la
  registra. Guardar una que no responde dejaría en la lista una impresora contra la que falla cada
  tique en silencio.
- **Se recuerda como tecleada.** `DeviceRegistry::register_manual` marca `Device::manual`
  (`devices.json`, `#[serde(default)]`, no se serializa si es `false`: los ficheros viejos no
  cambian). `upsert` la conserva con un `|=`: un escaneo que también la ve no la degrada.
- **Sigue en la lista.** `with_manual_printers` añade al resultado del escaneo las tecleadas que
  este no vio, con el estado del watchdog (`offline` → `offline`; lo demás → `ready`). **Solo** las
  tecleadas: una impresora escaneada ayer y apagada hoy no se queda para siempre. Un escaneo
  **bloqueado** sigue bloqueado: la tecleada va detrás del mismo permiso.
- **El rechazo viaja con código.** `PeripheralError::code()` da la palabra estable
  (`invalid_printer_address` · `printer_unreachable` · …); la orden del shell
  `erplora_add_network_printer` rechaza con `{code, message}` (no con el string pelado de
  `HardwareError`), el SDK lo convierte en `ErploraError(code)` — `printer_add_failed` si la app es
  anterior y no conoce la orden — y el módulo `printing` lo dice en el idioma del usuario.

| Capa | Pieza |
|---|---|
| `crates/peripherals` | `add_network_printer` · `with_manual_printers` · `register_manual` · `Device::manual` · `PeripheralError::code` |
| `apps/tauri` | orden `erplora_add_network_printer` (+ `allow-…` en `capabilities/default.json`); `erplora_discover_printers` pasa por `with_manual_printers` |
| `@erplora/module-sdk` | `BridgeTransport.addNetworkPrinter?(host, port = 9100)` — **opcional**: un hub con SDK anterior no la tiene y el módulo no ofrece el formulario · constantes `INVALID_PRINTER_ADDRESS` / `PRINTER_UNREACHABLE` / `PRINTER_ADD_FAILED` |
| módulo `printing` | «Añadir impresora por IP» bajo «Impresoras en la red»: IP + puerto (9100), «Añadir y probar» → la lista la enseña y sale la página de prueba |

Limitación conocida: en escritorio, si ARP resuelve la MAC de una impresora tecleada que el escaneo
**también** ve, el escaneo la registra por MAC (otra `key`) y el rol puesto sobre la tecleada no se
ve en esa fila; la tecleada existe porque el escaneo **no** la veía, así que el caso es marginal.

## Constantes y errores

- `ESCPOS_NETWORK_PORT = 9100` (raw printing) · `BRIDGE_WS_PORT = 12321` (compat `bridge.js`).
- CUPS (`usb`): `CUPS_LP` · `CUPS_LPSTAT` · `CUPS_LPOPTIONS` · `CUPS_CANCEL`; `CUPS_TIMEOUT = 20 s` por invocación y `JOB_COMPLETION_WINDOW = 15 s` para la confirmación. Estados de cola: `QUEUE_STATUS_READY` / `QUEUE_STATUS_STOPPED` / `QUEUE_STATUS_UNKNOWN`.
- `PeripheralError`: `InvalidPrinterId` · `Unreachable` · `UnknownDocumentType` ·
  `InvalidPayload` · `Io` (`#[from]`) · `Json` (`#[from]`). `code()` da la palabra estable de cada
  una para la pantalla (hub#1924).
- `MANUAL_PRINTER_PROBE_TIMEOUT = 3 s` (alta por IP tecleada, hub#1924).

## Pendiente de verificar

- ~~Esqueleto: parte de las firmas siguen con `todo!()`~~ **Ya no**: cero `todo!()` al 2026-08-01.
  Queda de aquella nota que la implementación real y los tests
  del crate (render ESC/POS contra bytes esperados, cola/reintentos con socket simulado)
  son fase siguiente (README del crate).
- El `Watchdog::run(&self, registry)` no recibe canal del consumidor: hoy emite
  recuperación/pérdida por `tracing` en vez de un `mpsc` (la firma se mantuvo intacta a
  propósito; cablear un canal real exigiría cambiarla). Pendiente de verificar el wiring
  de `WatchdogEvent` → `Event::DeviceRecovered`/`DeviceLost` en los consumidores.

## Documentos hermanos

- [sync.md](./sync.md) — el WebSocket `/ws` de **eventos** del server (no confundir con el WS `:12321` del Bridge, que ya no existe).
- [guest-sdk.md](./guest-sdk.md) · [wasm-host.md](./wasm-host.md)
- [hub/CLAUDE.md](../../../hub/CLAUDE.md) · [../apps/bridge.md](../apps/bridge.md) (reubicado desde `hub/ARQUITECTURA.md §2.7`) · `hub/crates/peripherals/README.md`.
