> Copia histórica de `erplora-old/architecture/modules/printing.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

> Antes de tocar este módulo, carga este doc.

# Módulo `printing` — Impresión y hardware (cola en el HUB, ADR-0196)

> **Referencia funcional:** [`WORKFLOW.md` de `printing`](https://github.com/ERPlora/printing/blob/main/WORKFLOW.md) — qué hace el módulo, sus flujos y a quién afecta cada uno; este documento es el técnico. El camino del cobro a la AEAT: [`REC_FISCAL`](../workflows/cadena-fiscal.md).

| Campo | Valor |
|---|---|
| **id** | `printing` |
| **Nombre** | Printing |
| **Tier** | `free` (sin `billing` declarado en `module.json`, ADR-0032) |
| **depends_on** | — (ninguno) |
| **Backend** | SQL declarativo (Tier 0); **sin WASM** |
| **Capabilities** | `printer` (ADR-0079) — la exige el listener-host `host.print` antes de encolar (declarada **y** concedida) |
| **Eventos** | emite `printing.print.due` (intención de impresión, [ADR-0349](../00-overview/decision-log.md#adr-0349)); no escucha ninguno |
| **UI** | `erp-printing-settings` (`dist/printing.esm.js`) — `erp-printing-routing` **retirado** en [printing#25](https://github.com/ERPlora/printing/issues/25) |
| **Hardware** | ninguno propio: la cola vive en el **runtime del Hub** y la drena el dispositivo con `erplora-app` registrado como **host de impresión** ([ADR-0196](../adr/ADR-0196.md) §6 → [`../hub/print-queue.md`](../hub/print-queue.md)) |

## Propósito

`printing` es la **configuración de negocio de impresión** del tenant: cabecera/pie del tiquet,
ancho de papel, y auto-impresión y apertura de cajón al cerrar venta. **No imprime él**: encola.

> 🪦 **El enrutado categoría→estación se RETIRÓ** ([printing#25](https://github.com/ERPlora/printing/issues/25),
> 2026-08-20). Este doc llevaba desde el 2026-08-07 diciendo que había que hacerlo (ver «Notas»), y
> mientras tanto la pestaña **Routing** seguía **publicada al comerciante** con su tabla, su query y
> sus dos commands, y **nadie leía nada de eso**. Se fueron la entrada de navegación, el componente,
> `printing.routing.list`, `printing.routing.set`/`.remove`, sus dos permisos (`view_routing`,
> `manage_routing`), sus ficheros SQL/schema/fixture y sus 16 claves i18n.
>
> **La tabla `printing_routing` se CONSERVA**, y es una decisión: un `DROP` en una migración de
> módulo exige `kind: contract`, y por [ADR-0269](../00-overview/decision-log.md#adr-0269) una
> migración `contract` es justo lo que deja una versión **sin vuelta atrás** — para una tabla que ya
> no es alcanzable desde ninguna parte y cuyas filas son configuración del comerciante. Fijado en
> `tests/routing_retired.contract.test.py` del módulo, que falla tanto si vuelve la pestaña como si
> alguien dropea la tabla por reflejo.
>
> **Dónde se enruta hoy:** `kitchen` → **Estaciones** (`kitchen_category_station`,
> `kitchen.stations.set_routing`). No es un sustituto peor: enruta por estación explícita → producto
> → **categoría**, y la categoría de la línea le llega de verdad desde
> [sales#12](https://github.com/ERPlora/sales/issues/12), verificado en
> [kitchen#32](https://github.com/ERPlora/kitchen/issues/32).

> 🪦 **El Bridge ya no existe** ([ADR-0196](../adr/ADR-0196.md), ejecutado en
> hub#339 + hub#340 el 2026-08-08). Este doc describía el disparo como «client-side por WebSocket
> contra el Bridge local»: **eso es historia**. No hay ningún WS `:12321`, ni `devices.json`, ni
> proceso de puente. Hoy la **cola vive en el Hub** (`crates/runtime/src/print_queue.rs`) y la drena
> por `GET /ws/print` el dispositivo con `erplora-app` que se registra como **host de impresión** de
> un `role`; el `jobId` da la idempotencia. Contrato completo y vivo:
> [`../hub/print-queue.md`](../hub/print-queue.md).

Tres responsabilidades **separadas** que conviene no confundir:

- **Formato/render** del tiquet y la factura = **OutfitKit** (`ok-receipt` 80 mm, `ok-invoice` A4),
  presentacionales (JSON + QR) — **no** este módulo
  ([decisión](../00-overview/decision-log.md), memoria `ok-receipt-outfitkit-tier4-exception`).
- **Datos** de la venta/factura = módulos `sales` / `invoice`.
- **Config de negocio del recibo + control de hardware** = **este módulo** (`printing`) — salvo el
  **texto** del tique: la cabecera y el pie son de `sales` desde hub#1921 (printing#44, abajo).
- **Enrutado de comandas a estaciones** = **`kitchen`**, no este módulo (printing#25).
- **Rol físico de cada impresora** (qué impresora es "recibo"/"cocina") **no vive aquí**: es el
  **registro de host de impresión** del core, por `role` (hub#342). `printing` solo dice *qué* se
  imprime y *a qué estación lógica* va; el host registrado resuelve *en qué impresora física*.

## Modelo de datos (Postgres; subconjunto «ERPlora SQL» — [ADR-0007](../00-overview/decision-log.md#adr-0007--portabilidad-sql-dual-sqlite--postgres-subconjunto-erplora-sql--validador--shim--2026-06-10), Postgres-only [ADR-0154](../adr/ADR-0154.md))

- **`printing_settings`** (singleton por hub, `uq_printing_settings_hub`): `paper_width` (80|58 mm),
  `auto_print_on_sale`, `open_drawer_on_sale`, `print_kitchen` (enrutar comandas) y 🪦
  `receipt_header`/`receipt_footer`, **columnas legado desde printing#44**: ningún papel las lee
  (el tique lo compone el visor de `sales` con `sales.pos_settings.get`) y
  `printing.settings.update` ya **no las escribe** — se conservan, y la query las sigue
  proyectando, solo para que la pantalla pueda ofrecer llevar a Ventas el texto que un negocio
  escribiera antes. Booleanos como `INTEGER 0/1`, PK `TEXT`.
- **`printing_routing`** (`uq_printing_routing_cat` por `(hub_id, category)`): `category` →
  `station` ∈ {`receipt`, `kitchen`, `bar`}. 🪦 **Sin query, sin command y sin pantalla desde
  printing#25** — inalcanzable. Se conserva por no destruir configuración del comerciante y porque
  dropearla exigiría una migración `contract` (ADR-0269), que deja la versión sin vuelta atrás.
- **`printing_jobs`** (migración `002_jobs.sql`, [printing#18](https://github.com/ERPlora/printing/issues/18);
  `uq_printing_jobs_job` por `(hub_id, job_id)`): el registro **de la petición** que hizo
  `printing.jobs.create` — `job_id`, `document_type`, `role`, `format` (los dos últimos nullable). No
  es la cola: la cola es `_print_queue` del runtime (tabla de sistema que el módulo no lee), y el
  desenlace del trabajo lo conoce el runtime, no esta tabla.

Ambas con el contrato estándar §2.5 (scoping `hub_id` + soft-delete + auditoría).

## Queries

| Capability | Permiso | Descripción |
|---|---|---|
| `printing.settings.get` | `printing.view_settings` | Ajustes de impresión (cabecera/pie, ancho, auto-print, auto-cajón). Responde **siempre una fila**: la guardada o los defaults del DDL (printing#42). |

## Commands

| Capability | Permiso | Descripción |
|---|---|---|
| `printing.settings.update` | `printing.manage_settings` | Actualiza los ajustes (upsert del singleton). |
| `printing.jobs.create` | `printing.print` | **La puerta pública a la cola de impresión** (printing#18). Payload = `PrintIntent` en camelCase (`jobId`, `documentType` ∈ `print_queue::DOCUMENT_TYPES`, `document` objeto, `role?`, `format?` ∈ {`receipt`,`a4`}); registra la petición en `printing_jobs` (`ON CONFLICT DO NOTHING`) y **emite `printing.print.due`**. |

Permisos adicionales **de acción** (no SQL): `printing.print`, `printing.open_drawer` — gatean el
encolado desde la UI/POS (y `printing.print` gatea además `printing.jobs.create`).

## Sacar papel sin pantalla — `printing.jobs.create` → `printing.print.due` ([ADR-0349](../00-overview/decision-log.md#adr-0349), hub#957/hub#992, [printing#18](https://github.com/ERPlora/printing/issues/18), 2026-08-18)

Hasta el 2026-08-18 `printing` **no publicaba nada hacia fuera**: ni command que otro módulo o un flujo
pudiera invocar para imprimir, ni evento. La única vía era `erplora.print(...)` → `POST /api/print/jobs`
desde una **pantalla**; un flujo corre en el runtime sin navegador. hub#957 abrió la puerta en el core
con la opción que menos descongela ADR-0283 D1 (hub#992, en `hub/develop` desde el 2026-08-15, hub
v1.1.7): un **segundo listener-host** del outbox, `*.print.due` → `host.print` → `print_queue::enqueue`.

```
step `command` (flujo) ─▶ printing.jobs.create {jobId, documentType, document, role?, format?}
        │  (permiso printing.print · schema jobs_create.json · INSERT printing_jobs ON CONFLICT DO NOTHING)
        └─▶ _event_outbox  printing.print.due  {payload + system params}
                └─▶ relay ─▶ deliver_host_print ─▶ capabilities::require(printing, printer) ─▶ print_queue::enqueue
                                                    (declarada Y concedida, default-deny)          (idempotente por jobId)
```

Lo que el módulo declara, y por qué cada pieza es obligatoria:

- **`emit: ["printing.print.due"]`** en el command y **`events.emits`** con el mismo nombre. Regla
  hub#240/2b en `commands.rs::validate_handler_event`: un evento no declarado se rechaza (o no existe
  para el editor de flujos, hub#709). El módulo se declara y mergea **antes** de que nadie lo emita.
- **`capabilities.printer`** — `deliver_host_print` la exige declarada **y concedida** (Ajustes →
  Permisos). Sin la declaración ningún dueño podría concederla y todo trabajo moriría en el
  dead-letter con `host.print: capability denied`. Sin el grant, la petición se registra y el evento
  se emite; encolar es lo que se niega — y se ve, no se pierde.
- **Sin WASM**: el `emit` de un command declarativo lleva `bound` (payload + system params) al outbox y
  `PrintIntent::from_event_payload` **tolera** los params de sistema (`hub_id`, `now`…), así que el
  camino Tier 0 basta. `document` (objeto JSON) viaja en el payload y no se bindea en SQL.
- **`jobId` es la idempotencia** y **ya estaba construida** en `print_queue::enqueue` (`ON CONFLICT DO
  NOTHING`); `printing_jobs` la espeja para que un replay del mismo `jobId` (reintento de flujo, doble
  clic) no sea un error del command. `role` opcional (hub#987: el hub resuelve la estación desde
  `documentType`); `format` opcional (= `receipt`).
- **NO se declaran `printing.job.printed` / `printing.job.failed`.** El desenlace lo conoce el runtime
  (`print_ws.rs` / `print_drain.rs`) y hoy no tiene puente hacia los eventos de este módulo. Un evento
  que aparece en el desplegable de Automatizaciones y no se dispara nunca es un flujo que no corre; se
  declararán cuando el hub los emita (pieza aparte del hub).

**Frontera con `kitchen` (decidida en printing#18, mercado delante: Toast, Square, Lightspeed, Revel,
Odoo — el enrutado a estación es propiedad del artículo, no de una automatización):** la comanda la
sigue imprimiendo `kitchen` al disparar el pedido (ADR-0144). Esta puerta **complementa** —un flujo
avisa, registra, escala o imprime *otro* documento—; no es una segunda autoridad sobre la comanda.
La receta R0 #10 de flows#18 queda construible: `printing.jobs.create` + step `notify`.

Verificación: `printing/tests/jobs_create.contract.test.py` (contrato del manifest y del schema espejo
de `PrintIntent`; SQL contra Postgres real: un registro por job, replay sin error ni duplicado,
`role`/`format` ausentes, `jobId` por hub) y `erplora validate printing --pg`.

## Integración con el flujo de venta (implementada — [printing#1](https://github.com/ERPlora/printing/issues/1), [ADR-0019](../00-overview/decision-log.md#adr-0019--wiring-de-impresión-al-cierre-de-venta--listener-del-shell-sobre-salecompleted--2026-06-10))

El **punto de orquestación es el shell** (`hub/apps/web/src/lib/print-on-sale.ts`, arrancado en
`main.ts`): un listener client-side sobre `sale.completed`
([ADR-0010](../00-overview/decision-log.md#adr-0010--evento-canónico-de-cierre-de-venta--salecompleted--2026-06-10))
— no el módulo `sales` (imprimir es client-side, el runtime no toca hardware §2.7) ni un WC
(debe escuchar siempre, no solo con una vista montada). Al cerrar venta, según `printing_settings`:

- `auto_print_on_sale` → ticket (`document_type: receipt`, datos autoritativos de
  `sales.get` + `sales.lines`) **encolado** para el host con rol `receipt`.
  Desde sales#283 es el valor **por defecto**: el interruptor «Imprimir tiquet» del cobro viaja en
  la venta como `print_receipt` (en `sale.completed`) y, si es un booleano, **manda sobre el ajuste
  para esa venta** en los dos sentidos (forma Square/Toast). Sin la clave —API, asistente, otro
  productor— decide el ajuste. El cajón no depende del interruptor.
- `open_drawer_on_sale` → kick del cajón por el host de recibo.
- **Solo en la caja que cobró** ([hub#1980](https://github.com/ERPlora/hub/issues/1980)): cada
  shell oye todos los `sale.completed`, y el frame dice qué pestaña mandó el cobro
  (`client_instance`, ver `hub/auth.md` § «Y el frame dice qué PESTAÑA lo causó»). El tique y el
  cajón solo salen en esa pestaña. Una venta sin pestaña (API, flujo) no la imprime ninguna caja.
- ~~`print_kitchen` → comandas agrupadas por `printing.routing.list`~~ — **muerto desde ADR-0144**
  y retirado del manifest en printing#25. La comanda cuelga del disparo del pedido
  (`print-comanda.ts`) y la enrutan las **estaciones de `kitchen`**.

Todo defensivo: sin módulo `printing` instalado → no-op silencioso; **sin host de impresión
registrado el trabajo NO se pierde**, se queda en la cola del Hub hasta que alguien la drene (ese es
justo el agujero que cerró ADR-0196 §6). El runtime nunca habla con la impresora: el ESC/POS lo pone
`crates/peripherals`, llamado **in-process** por `erplora-app` (`hub/apps/tauri`), no por un proceso
aparte.

> **Gap cerrado** (era: «`sales.lines` no proyecta la categoría»): desde
> [sales#12](https://github.com/ERPlora/sales/issues/12) `order.fired` lleva `category_id` por línea
> y `kitchen` lo reenvía a `_insert_item`, cuyo `COALESCE` cae a `kitchen_category_station` —
> verificado en [kitchen#32](https://github.com/ERPlora/kitchen/issues/32). El enrutado por
> categoría está **vivo**, en `kitchen`.

## UI

- `erp-printing-settings` — ajustes del ticket (cabecera/pie, ancho, auto-print/auto-cajón) +
  impresoras de red descubiertas por `erplora-app` y asignación de rol físico (entrada `Printers`).
  Desde [printing#17](https://github.com/ERPlora/printing/issues/17) (2026-08-18) **no hay interruptor
  «Enviar a cocina»** —`print_kitchen` es un mando muerto desde ADR-0144 y se retiró de la pantalla
  con su clave i18n `ui.routeToKitchen`; la columna se conserva y la UI la hace round-trip— y los
  `.row` de los interruptores llevan la misma separación vertical que los `.field`.
  Desde [printing#36](https://github.com/ERPlora/printing/pull/36) (hub#1565) la cola de esa misma
  pantalla tiene un **cuarto bucket, «Retirados recientemente»** (`status: 'discarded'`, página
  propia de 20, en el orden que manda el hub: el último retirado primero), pedido **solo si la
  sesión administra el hub** —el mismo criterio que el core aplica al sello— y cada ficha escribe
  **frases completas** del catálogo `en` + `es`: quién retiró o relanzó el tique (el **nombre**,
  cayendo al principal si el hub no supo nombrarlo), desde qué módulo, cuándo y por qué. Un hub más
  viejo que el módulo no manda esos campos y la pantalla no pinta ningún sello fantasma.
- 🪦 `erp-printing-routing` — **retirado** en
  [printing#25](https://github.com/ERPlora/printing/issues/25) (2026-08-20). Existió desde
  [printing#3](https://github.com/ERPlora/printing/issues/3) (2026-06-11) y quedó sin consumidor con
  ADR-0144 (2026-08-07): una pestaña **en la navegación**, documentada al usuario como viva, cuyas
  reglas no aplicaba nadie. Es el peor tipo de superficie muerta, y es lo que las «Notas» de este
  doc llevaban 13 días pidiendo que se retirase.

🪦 **Los dos `ion-input` de cabecera y pie se retiraron en printing#44** (ver la sección siguiente):
en su lugar la pantalla explica que el tique se configura en Ventas, enlaza a `/m/sales/settings` y
—mientras quede texto huérfano aquí y Ventas no tenga el suyo— lo enseña con un botón que lo lleva
allí. La regla de ADR-0143 / hub#760 sigue vigente para cualquier `ion-input` que se añada: `fill`
exige `mode="md"` junto, porque el shell fija `mode: 'ios'` y ahí Ionic **no** pinta el `fill`.

## El texto del tique es de `sales` — y el ítem 70 se retiró ([printing#44](https://github.com/ERPlora/printing/issues/44), 2026-09-20)

Hasta hub#1921 el tique que salía solo al cobrar lo armaba el **shell**
(`apps/web/src/lib/receipt-document.ts`) leyendo `printing.settings.get` → `receipt_header` /
`receipt_footer`. Desde hub#1921 los **dos** papeles —el automático y el del botón de imprimir— son
el `printableDocument()` del visor de `sales`, que lee `sales.pos_settings.get`. Consecuencia
directa: las dos casillas de esta pantalla escribían un campo que **no sale en ningún papel**. Quien
las rellenaba creía haber personalizado su tique y no cambiaba nada.

Qué cambió en el módulo:

- **Se quitan las dos casillas.** En su lugar, la pantalla dice dónde se configura el tique y enlaza
  a `/m/sales/settings` (el form genérico del shell sobre el bloque `settings` de `sales`).
- **El texto huérfano no se pierde.** Si este módulo aún guarda cabecera o pie y Ventas no tiene lo
  suyo, la pantalla lo enseña y ofrece **llevarlo** con `sales.settings.adopt_receipt_text`, una
  puerta estrecha de `sales` que **solo rellena lo que allí esté vacío**, campo a campo (nunca
  encima de lo que el negocio escribió en Ventas: una plantilla podría estampar su cabecera de demo
  en el tique de alguien). No es `sales.settings.update` porque esa toma el **snapshot completo** y
  dejaría los otros dieciséis ajustes en sus defaults.
- **Lectura y escritura son OPCIONALES** (`queryOptional`/`commandOptional`, ADR-0127/ADR-0438):
  `printing` **no** depende de `sales` — un hub puede tener impresoras y no tener TPV—, así que sin
  Ventas la pantalla degrada: enseña el texto para copiarlo a mano y no enlaza a ninguna parte.
- **El éxito se cree de la RELECTURA, no de la llamada**: `commandOptional` responde `undefined` si
  el módulo se fue, y el comando no escribe si Ventas ya tenía texto. Decir «llevado» sin comprobar
  sería decirle a alguien que su texto está a salvo cuando sigue huérfano aquí.
- **La hoja de prueba se firma como el tique**: `business_name` sale de la primera línea del
  `receipt_header` **de Ventas** y, si no hay, de `sales.business.get` (la razón social,
  [ADR-0061](../00-overview/decision-log.md#adr-0061)) — el mismo orden que el mapper de `sales`.
  Antes salía de la cabecera de este módulo, o sea firmada con un nombre que el tique de al lado no
  imprimía nunca.
- 🪦 **El bloque `setup` se retira, y con él el ítem 70.** Se comprobaba con `receipt_header`
  *truthy*, así que pedía a todo salón nuevo un dato que su tique no usa. No se puede reapuntar a lo
  que el paso sí necesita —**≥1 impresora con rol `receipt`**— porque `setup.query` tiene que ser
  una query del **propio** módulo y el rol vive en el registro de dispositivos del core (ADR-0196,
  hub#341-344); las demás columnas tienen default usable y marcarían el ítem el día uno. El paso lo
  da ahora el core, `printer` en la misma ranura 70
  ([hub#1948](https://github.com/ERPlora/hub/issues/1948)), y lleva a Ajustes → Impresión (`#tickets`, antes «Tiques»; hub#2243), cuya fila
  sigue a la pantalla Impresoras de este módulo. Detalle:
  [`../hub/setup-status.md`](../hub/setup-status.md) §6.

Lo fijan `tests/legacy_receipt_text.postgres.test.py` (no hay `setup`, la puerta ya no escribe las
columnas ni aunque el payload las traiga, la query las sigue proyectando) en `printing`,
`tests/adopt_receipt_text.postgres.test.py` en `sales` (rellena lo vacío, nunca encima; resucita el
singleton soft-borrado; no toca nada cuando no hay nada que llevar) y los tests del WC.

## Notas

- Módulo **propio** (confirmado 2026-06-10): es config + control de hardware, concern distinto del
  formato (OutfitKit) y de los datos (sales/invoice). Este doc se creó porque faltaba.
- ✅ **`printing.routing.*` RETIRADO** (printing#25, 2026-08-20) — lo que esta misma nota venía
  pidiendo desde el 2026-08-07. Se fueron la vista, la query, los dos commands, los dos permisos y
  sus claves i18n; **la tabla se conserva** (motivo arriba, en «Propósito»). La sección «Integración
  con el flujo de venta» describe el diseño de ADR-0019 y solo sigue vigente en su parte de **recibo
  + cajón**.
- ⚠️ **`print_kitchen` sigue siendo columna muerta** desde
  [ADR-0144](../00-overview/decision-log.md#adr-0144): el interruptor salió de la pantalla
  (printing#17) pero la **columna** y el campo requerido de `schemas/settings_update.json` siguen
  (a diferencia de `receipt_header`/`receipt_footer`, que printing#44 sí sacó del schema porque
  dejaron de ser de este módulo) —
  son contrato externo (columna de BD, campo requerido del payload) y quitarlos rompe a quien ya los
  escribe. Retirarlos es un cambio propio, con su deprecación.
- 🔧 `commands/settings_upsert.sql` **resucita** la fila en su `DO UPDATE` (`is_deleted = 0`,
  `deleted_at = NULL`) desde printing#25. `uq_printing_settings_hub` **no es parcial**, así que una
  fila soft-borrada seguía ocupando la ranura del `ON CONFLICT`: guardar reportaba éxito, escribía
  una fila que `settings_get.sql` nunca devuelve, y el formulario volvía a los defaults por más
  veces que se pulsara Guardar. Su gemelo `routing_upsert.sql` ya resucitaba; la diferencia era un
  olvido, no una decisión. Desde printing#44 ese `DO UPDATE` **ya no escribe** `receipt_header` ni
  `receipt_footer`: el texto huérfano tiene que sobrevivir hasta que se lleve a Ventas.
- 🔧 `queries/settings_get.sql` responde **siempre una fila** desde printing#42: fila ancla +
  `LEFT JOIN` a la fila viva de este hub (mismo patrón que `verifactu.config.get`, verifactu#107).
  Antes, un hub que nunca había pulsado «Guardar ajustes» —todo salón montado con plantilla— recibía
  cero filas: la pantalla pintaba sus defaults («Imprimir ticket al cobrar: encendido») y el
  escuchador `print-on-sale` del shell, sin fila, no imprimía ni encolaba ni avisaba. Sin fila viva
  (nunca guardada o soft-borrada) responde los defaults del DDL (`auto_print_on_sale = 1`,
  `receipt_header = ''`…) con `id = NULL`; leer no escribe. Si cambia un `DEFAULT` del DDL, el `COALESCE` de la
  query cambia con él: lo fija `tests/settings_defaults.postgres.test.py`, que compara contra una
  fila nacida del propio DDL.
- Desde printing#18 la frase «`printing` no emite nada» de otros docs (p. ej. `event-outbox.md`
  §gate de emisión: «el 24º, `printing`, no emite nada») ha dejado de ser cierta: emite
  `printing.print.due` y pide la capability `printer`.
- 👁️ **La cola del runtime YA es legible y operable desde este módulo** (hub#1107 + hub#1108,
  2026-08-25). La nota de arriba —«`_print_queue` … tabla de sistema que el módulo no lee»— sigue
  siendo cierta *sobre el SQL*: lo que cambia es que ahora hay **puertas**, y son las del contrato
  WC → SDK → dispatcher, no un fetch crudo:
  - **Leer**: `erplora().query('hub.print.coverage')` y `erplora().query('hub.print.jobs')` —
    queries del namespace reservado del core ([ADR-0192](../00-overview/decision-log.md#adr-0192)),
    con gate `hub.users.view` (**cualquier sesión local, no admin**: quien está al lado de la
    impresora). Devuelven, respectivamente, `{role, waiting, liveHosts, waitingSeconds, undrained}`
    y la cola como **vista de estado** (nunca el documento). Desde hub#1565
    ([ADR-0448](../00-overview/decision-log.md#adr-0448)) la fila de `hub.print.jobs` trae además el
    **sello** —`discardedBy`/`discardedByName`/`discardedByModule`/`discardReason`/`discardedAt` y
    los cuatro `retried*`— **solo** cuando la sesión lleva `hub.administer`; al mostrador le llegan las
    mismas filas sin esas claves, y los buckets cerrados (`discarded`, `done`) vienen del más reciente
    al más antiguo. Desde hub#2368 cada fila trae también **`documentRef`** (el número impreso del
    documento): la pantalla rotula el trabajo «Recibo T-000123» y nombra cada gesto por él
    («Descartar Recibo T-000123», printing#52); sin él —un hub anterior— cae a «Recibo · fecha y
    hora». El `jobId` no se pinta. Las frases que cuentan (aviso de la cola, «Todo al día», «Imprime
    desde…») eligen clave singular `…One` o plural según el número (printing#53): nada de «(s)».
  - **Escribir**: `erplora.forModule('printing').printQueue.retry(jobId)` y `.discard(jobId, reason?)`
    — sesión **admin** más la capability **`printer`** declarada y **concedida**. ⚠️ El getter es
    `printQueue`, no `print`: `erplora.print(req)` es la llamada publicada con la que todos los
    módulos ENCOLAN un documento.

  Lo que sigue faltando es **la pantalla**: [printing#28](https://github.com/ERPlora/printing/issues/28).
  El core ya sirve los cuatro datos y los dos gestos.
