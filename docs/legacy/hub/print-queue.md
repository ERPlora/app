> Copia histórica de `erplora-old/architecture/hub/print-queue.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Hub — Cola de impresión (la cola vive en el HUB)

> Cargar este doc antes de editar la cola de impresión (`crates/runtime/src/print_queue.rs`,
> `crates/runtime/src/print_hosts.rs`, `crates/runtime/src/print_stations.rs`,
> `crates/runtime/src/print_routes.rs`, `crates/runtime/src/print_drain.rs`,
> `crates/server/src/print.rs`, `crates/server/src/print_ws.rs`,
> `apps/web/src/lib/print-drain.ts`, `apps/web/src/lib/print-host.ts`,
> `apps/web/src/lib/print-host-registration.ts`, `apps/web/src/lib/sale-document.ts`,
> `apps/web/src/lib/print-coverage.ts`, `apps/web/src/lib/print-alert.ts`,
> `crates/peripherals/src/escpos.rs`, `crates/runtime/src/hub_users.rs` (las queries core
> `hub.print.*`), `packages/module-sdk/src/index.ts` (`PrintApi`), las migraciones de sistema
> **v18**, **v22**, **v25**, **v47**, **v50** y **v54**) o cualquier consumidor suyo.

Propósito: describir **dónde vive** y **cómo se comporta** la cola de trabajos de impresión del
Hub. Antes vivía en el proceso **Bridge del dispositivo** (`crates/peripherals/src/queue.rs`: un
`mpsc` en memoria con reintentos contra el socket ESC/POS). Eso tenía dos agujeros de producto:
sin nadie con el Bridge abierto **el trabajo no se encolaba en ningún sitio** —se perdía— y el
camarero con la PWA en el móvil no podía imprimir un tique aunque hubiera un TPV encendido a tres
metros con la impresora en la misma red.

[ADR-0196](../adr/ADR-0196.md) §6 lo invierte: **la cola vive en el Hub**.
Cualquier dispositivo —PWA incluida— encola `{ role, documentType, document, jobId }`; el
dispositivo con la app instalada, en la red de la impresora, se registra como **host de impresión**
de ese `role` y drena la cola por el WS del runtime (`GET /ws/print`, hub#343). Lo que cambia no es
la mecánica de reintentos (se conserva la forma de `crates/peripherals`), es **dónde vive la cola**.

> **El documento viaja ESTRUCTURADO, no como HTML** ([ADR-0266](../00-overview/decision-log.md#adr-0266),
> hub#501). La cola guardó `html` hasta la v25 y **eso no podía llegar al papel**: el renderer sólo
> habla documentos estructurados. Ver la sección [El documento que viaja](#el-documento-que-viaja).

> **Hay un SEGUNDO productor, y no es un dispositivo** ([ADR-0349](../00-overview/decision-log.md#adr-0349),
> hub#957). Todo lo de arriba describe a alguien con una pantalla delante, y por eso un **flujo** no
> podía imprimir: corre en el runtime, sin navegador. Desde el 2026-08-15 el **relay del outbox**
> encola también: un command de un módulo con la capability `printer` emite `<algo>.print.due` con
> `{jobId, role, documentType, document, format}` y el listener-host `host.print` llama a este mismo
> `enqueue`. **La cola no cambia** —ni su forma, ni sus garantías, ni su idempotencia por `jobId`—:
> lo único que cambia es quién llama a la puerta. Detalle en
> [event-outbox.md](event-outbox.md) y [flows.md](flows.md) §5.2.

Docs hermanos: [event-outbox.md](event-outbox.md) (la otra cola durable del runtime, de la que esta
copia el patrón), [apps/bridge.md](apps/bridge.md) (🪦 el componente que se retira),
[apps/tauri.md](apps/tauri.md) (la app instalable que será el host de impresión),
[crates/peripherals.md](crates/peripherals.md) (el ESC/POS y la `PrintQueue` en memoria original),
[README.md](README.md) (índice).

---

## Las tres garantías

| Garantía | Mecanismo | Dónde |
|----------|-----------|-------|
| **Idempotencia por `jobId`** | `PRIMARY KEY (hub_id, job_id)` + `INSERT … ON CONFLICT DO NOTHING`. Un `jobId` repetido no escribe **ni sobrescribe**: se responde `duplicate`. | `_print_queue` |
| **Tarde, no perdido** (solo lo que entra en la cola) | El trabajo queda `pending` hasta que un host de su `role` lo reclame. Sin host conectado **espera**; no se descarta ni se degrada en silencio. ⚠️ El tique que la caja imprime directo nunca entra en la cola: desde ERPlora/hub#2494 la app instalada espera a que la impresora de red se quede los bytes (`PrintQueue::print`) y, si tras los reintentos no contesta, la caja recibe `printerFailed` y avisa con «Reintentar» (no se encola: lo sacaría el mismo dispositivo con la misma impresora apagada). Un trabajo de la cola entregado a una impresora de red apagada vuelve `failed` → `pending`, pero sin aviso a nadie (ERPlora/hub#2711). Una impresora encendida sin papel se queda los bytes y cuenta como impresa (el 9100 no informa del papel) | `_print_queue.status` |
| **Sobrevive al reinicio** | Es una **tabla**, no un canal en memoria: un runtime que se reinicia encuentra la cola tal cual estaba. | migración de sistema **v18** |

Consecuencia clave: **el productor no necesita ser idempotente**. `sdk.print` puede reintentar, el
cajero puede tocar «imprimir» dos veces y la app puede reconectar: mientras el `jobId` sea el mismo,
sale **un** tique.

---

## Máquina de estados

```text
   LA MÁQUINA                                                    UNA PERSONA (hub#1108)

           enqueue                claim_next                 mark_done
  (nada) ──────────▶ pending ────────────────▶ printing ──────────────▶ done
                       ▲                          │
                       │  mark_failed             │  vence el lease (el host murió)
                       └──────────────────────────┤
                       ▲                          ▼   attempts >= MAX_ATTEMPTS
                       └───── requeue ────────── dead
                                                   │
   pending ─────┬───────── discard ────────────────┤
                │                                  ▼
                └──────────────────────────▶ discarded   (la fila SE QUEDA, sellada)
```

- **`attempts` cuenta ENTREGAS, no fallos.** Se incrementa al **reclamar**, no al fallar. Un host
  que se lleva el trabajo y muere sin contestar quema un intento igual: si no, un dispositivo roto
  podría hacer girar la cola indefinidamente sin que nadie lo notara.
- **Todo claim lleva un `lease`** (`DEFAULT_LEASE_SECONDS` = 90 s). Vencido, `reclaim_expired`
  devuelve el trabajo a `pending` — el tique no se queda colgado en `printing` porque alguien cerró
  la app. Es exactamente el «trabajo perdido» que ADR-0196 §6 descarta.
- **`dead` es diagnóstico, no borrado**: la fila se conserva con su `last_error`, como el
  dead-letter del Outbox. Y su `jobId` sigue guardando: reenviar ese id no reimprime nada.
- **`dead` ya NO es el final del camino** (hub#1108). Una **persona** puede devolverlo a `pending`
  (`requeue`, con `attempts` a 0) o retirarlo (`discard` → estado `discarded`, sellado con quién,
  cuándo y por qué, **sin borrar la fila**). Es la mitad que faltaba: la máquina de arriba reparte y
  se rinde, pero nadie podía deshacer ninguna de las dos cosas. Ver
  [Sacar del atasco UN trabajo](#sacar-del-atasco-un-trabajo-hub1108).
- **El reparto es atómico**: `UPDATE … WHERE job_id = (SELECT … ORDER BY seq FOR UPDATE SKIP
  LOCKED LIMIT 1) RETURNING …`. Dos hosts del mismo `role` compitiendo se llevan trabajos
  **distintos**, nunca el mismo tique dos veces.

---

## La tabla `_print_queue` (migración de sistema v18)

| Columna | Para qué |
|---------|----------|
| `hub_id`, `job_id` | **PK compuesta** = la garantía de idempotencia. `job_id` lo elige el productor. |
| `seq` (`BIGSERIAL`) | **Orden de entrega.** `created_at` no basta: dos comandas encoladas en el mismo instante empatan y la primera podría imprimirse después. |
| `role` | La `key` de la **estación** (`receipt`, `kitchen`, `bar`, …), siempre canónica: desde la v47 no se guarda lo que llegó, se guarda lo que resolvió. El rol físico deja de vivir en el `devices.json` del Bridge. |
| `station_id` | **A qué estación va** (v47, hub#457). Es por lo que filtra `claim_next`: la cola y el host se encuentran por una **fila**, no por dos cadenas que casualmente coinciden. |
| `document_type` | Qué renderizador lo convierte en papel: `receipt`, `kitchen_order`, `invoice`, `delivery_note`, `barcode_label`, `cash_session_report`, `prebill`, `generic`. **Vocabulario cerrado** (v25; `prebill` entra en hub#748). |
| `document` | El documento **estructurado** (JSON en TEXT): el objeto que lee `escpos::render_document`. Tope de 512 KiB sobre su serialización **del documento que mandó el productor**: esto es un tique, no una subida de ficheros. Al guardarlo se le añaden `locale` (ver [§El idioma del papel](#el-idioma-del-papel-se-sella-en-la-cola-hub1159)) y `decimals` (ver [§La escala de la moneda](#la-escala-de-la-moneda-se-sella-en-la-cola-hub2129)). |
| ~~`html`~~ | **Borrada en la v25** (hub#501). Guardaba un documento autocontenido que ningún host podía imprimir. |
| `format` | `receipt` (80 mm) o `a4`. Sin él, el host no sabe con qué papel renderizar — una factura salía con ancho de tique. |
| `status`, `attempts`, `last_error` | La vista observable: qué espera, qué se está imprimiendo, qué murió y por qué. |
| `claimed_by`, `lease_expires_at` | Quién se lo llevó y hasta cuándo (instantes en TEXT RFC3339, comparables lexicográficamente en UTC, como `_event_outbox`). |
| `created_at`, `completed_at` | Traza. |
| `discarded_at`, `discarded_by`, `discard_reason` | **El sello del descarte** (v54, hub#1108). Retirar un trabajo **nunca** borra la fila —es la única prueba de que el tique existió— y una fila cerrada sin autor ni motivo haría invisible justo lo que pasó. Mismas columnas y tipos que hub#660/hub#955 en `_event_outbox`. |
| `discarded_by_module` | **Qué MÓDULO pidió el descarte** (v58, hub#1532), o `''` si no lo pidió ninguno —la shell, `curl`—. `NOT NULL DEFAULT ''` como `discard_reason`: «ningún módulo» es una respuesta real y frecuente, y tiene **una sola** grafía. No sustituye a `discarded_by`: la pregunta útil («Ana no tiró este tique, ¿QUÉ lo tiró?») no tiene respuesta sin las dos. |
| `retried_at`, `retried_by`, `retried_by_module` | **El sello del reintento** (v58, hub#1532) — el gesto hermano no sellaba nada. Es el **ÚLTIMO** reintento, no un historial: un trabajo se puede volver a disparar, morir y volver a dispararse, y cada uno pisa el anterior. Guardar todos es un log de auditoría, que es otra tabla y otra decisión. |

Índices: `(hub_id, station_id, status, seq)` para el reparto (v47; el viejo `(hub_id, role, …)`
sigue ahí y ya no lo usa el reparto) y `(hub_id, status, lease_expires_at)` para el barrido de
leases.

> **El número de la migración no es cosmético, y un HUECO NO RESERVA NADA.** `apply` compara
> contra el **máximo** aplicado, así que una migración numerada en o por debajo de él **se salta en
> silencio**: el hub arranca creyendo que está al día, sin la tabla y sin un solo log. Esta cola
> nació como v14 (choque con `hub_certificate_slots`, hub#316), se movió a la v15 que hub#317 había
> dejado libre a propósito… y esa v15 tampoco servía: hub#317 (v16) y hub#357 (v17) llegaron a
> `develop` antes, así que cualquier hub desplegado desde ahí ya está en `max=17` y jamás correría
> una v15. Acabó en **v18**, el siguiente número POR ENCIMA del máximo. La v15 queda vacía para
> siempre (inofensivo: el test de orden solo exige que crezcan).
>
> Lo detectó un test, no una revisión: `a_hub_already_migrated_still_receives_the_print_queue`
> aplica sobre un hub con TODO el catálogo ya registrado salvo esta migración, y falló en rojo
> mientras el número fue v15. **Regla:** coge siempre el siguiente número por encima del máximo del
> catálogo; nunca rellenes un hueco.

> **`hub_id` en la PK, no solo en el `WHERE`.** Es el mismo criterio que `hub_module` (migración
> v1): el mismo `jobId` en dos hubs son **dos** trabajos.

---

## Superficie

**Runtime** (`crates/runtime/src/print_queue.rs`): `enqueue` · `claim_next` · `mark_done` ·
`mark_failed` · `reclaim_expired` · `list` · `role_of` · **`requeue`** · **`discard`** (hub#1108).
Envueltas en `Runtime::enqueue_print_job`, `Runtime::print_queue`, `Runtime::retry_print_job` y
`Runtime::discard_print_job` (scoped al `hub_id` del despliegue, no spoofable).

**HTTP** (`crates/server/src/print.rs`):

| Endpoint | Auth | Contrato |
|----------|------|----------|
| `POST /api/print/jobs` | sesión de usuario | Body `{ jobId, role?, documentType, document, format? }` (camelCase, la forma que ya produce `PrintRequest` del shell). `200 { ok, jobId, status: "queued" \| "duplicate", role, liveHosts }`. Un `duplicate` es **éxito**: el `jobId` hizo su trabajo. **`role` y `liveHosts` desde hub#1731**: la estación en la que el trabajo CAYÓ (resuelta, no la palabra del productor) y cuántos equipos la están drenando **ahora**. `liveHosts: 0` es «encolado y no va a venir nadie a por él», que es lo único que separa «sale tarde» de «no sale nunca» — sin ese dato, `via:'queue'` se leía igual que impreso y el TPV cobraba en silencio en un hub sin ninguna impresora ([abajo](#la-cola-contesta-si-hay-alguien-que-la-drene-hub1731)). Malformado → `422 invalid_payload`. **`role` es OPCIONAL desde la v50** (hub#987): omitirlo enruta por `documentType`; mandarlo es un override, y uno que no resuelve a ninguna estación de este hub sigue siendo **`422` que NOMBRA las que sí tiene** (v47). |
| `GET /api/print/jobs?role=&status=&limit=` | sesión de usuario | `200 { ok, jobs[] }` en orden de entrega (los buckets **cerrados** —`discarded`, `done`— del más reciente al más antiguo, hub#1565). Es una **vista de estado**: NO lleva el documento. El documento viaja al host que reclama el trabajo, no a quien hace polling. **El sello** (quién retiró o relanzó el trabajo, cuándo, por qué y desde qué módulo, con el **nombre** de la persona) viaja **solo** a la sesión con `hub.administer`; al mostrador le llegan las mismas filas sin esas claves ([abajo](#pero-el-sello-es-del-back-office-hub1565)). |
| `POST /api/print/jobs/{jobId}/retry` | **admin** (+ `printer` si llama un módulo) | Devuelve un trabajo `dead` a `pending` con `attempts = 0`, sellando quién lo pidió y a través de qué módulo (hub#1532). `404` si no es de este hub · `409 print.job_not_requeueable` **nombrando** el estado real. Ver [abajo](#sacar-del-atasco-un-trabajo-hub1108). |
| `POST /api/print/jobs/{jobId}/discard` | **admin** (+ `printer` si llama un módulo) | Body opcional `{ reason }`. Retira un `pending`/`dead` sellando quién, **por qué módulo** (hub#1532), cuándo y por qué. **Nunca borra.** `409 print.job_not_discardable` para uno `printing`. |

**Dispatcher** (`hub.*`, ADR-0192) — la lectura que hace la cola visible para un **módulo**:

| Query | Gate | Devuelve |
|-------|------|----------|
| `hub.print.coverage` | `hub.users.view` (sesión local, **no admin**) | Una fila por estación: `role`, `waiting`, `liveHosts`, `liveHostLabels`, `waitingSeconds`, `undrained`. |
| `hub.print.jobs` | `hub.users.view` (sesión local, **no admin**) | La cola en orden de entrega, como **vista de estado**. Params opcionales `role` / `status` / `limit`. Con `hub.administer` cada fila trae además **el sello** (hub#1565); sin él, las mismas filas sin esas claves. |

Ver [La cola, legible por un MÓDULO](#la-cola-legible-por-un-módulo-hub1107).

---

## La cola, legible por un MÓDULO (hub#1107)

El runtime sabía lo que pasa en la cola desde hub#341 (`print_queue::list`) y hub#800
(`print_hosts::coverage`), y lo servía por HTTP. Un **módulo** no podía leer ninguna de las dos: el
contrato es **WC → SDK → dispatcher** ([ADR-0192](../00-overview/decision-log.md#adr-0192)) y el SDK
no expone —a propósito— ninguna forma de llamar a una ruta del core. Resultado: la única pantalla que
veía la cola era del **shell** (Ajustes › Recibos + la campana de hub#987), y la pantalla
*Impresoras* del módulo `printing` no podía pintarla.

Las cinco puertas que **no** valían, y por qué (inventario de [printing#28](https://github.com/ERPlora/printing/issues/28)):

| Camino | Por qué no |
|--------|-----------|
| Fetch crudo de `/api/print/*` leyendo la sesión de `localStorage` | El antipatrón que hub#714 quitó para flows: funciona solo mientras la sesión siga en `localStorage` y rompe el contrato WC → SDK → dispatcher. |
| SQL directo a `_print_queue` | El validador lo bloquea: está en `RUNTIME_SYSTEM_TABLES` (`module-toolkit/src/validate-sql.mjs`). |
| Eventos | No existen `job.printed`/`job.failed`: la cola no publica nada. |
| Una superficie SDK tipada, como `EventsApi` | Es la forma correcta para **escribir** (hub#1108), pero para leer sería una segunda vía de datos junto al dispatcher, que es justo lo que el namespace del core existe para evitar. |
| Otra query de módulo | Un módulo no puede registrar nada bajo `hub.` (el instalador lo rechaza) y la cola no es suya. |

### Dos queries del namespace reservado

Se despachan **antes** del registry (`queries::execute_page`), así que ningún módulo puede
suplantarlas — el mismo camino exacto que `hub.setup.status` y `hub.fiscal.limits`.

- **`hub.print.coverage`** → `role`, `waiting`, `liveHosts`, **`liveHostLabels`**, `waitingSeconds`
  y **`undrained`**.
- **`hub.print.jobs`** → `jobId`, `role`, `documentType`, `format`, `status`, `attempts`,
  `createdAt`, `lastError` y **`documentRef`** (hub#2368), en orden de entrega, con filtros
  `role`/`status` y tope `limit`. `documentRef` es el número **impreso** del documento
  (`document.receipt_id`, recortado: el tique o la factura de sales/invoice, el `order_number` de
  kitchen) y **no viene** si el documento no trae uno utilizable — nunca como etiqueta vacía. Es lo
  que la pantalla de `printing` usa para rotular la fila («Recibo T-000123», printing#52); el
  `jobId` es la clave de idempotencia del productor, no un rótulo.
  **Solo para la sesión con `hub.administer`** (hub#1565), y solo en las filas donde el gesto
  ocurrió: `discardedAt`, `discardedBy`, `discardedByName`, `discardedByModule`, `discardReason` y
  `retriedAt`, `retriedBy`, `retriedByName`, `retriedByModule`. Ver
  […pero el SELLO es del BACK OFFICE](#pero-el-sello-es-del-back-office-hub1565).

**Tres propiedades que no son detalles:**

1. **El documento NO viaja.** Es una vista de estado; el tique (nombres, líneas, totales, el QR
   fiscal) solo sale por el drenaje, pasando por sus dos guardas (hub#343). La **única** excepción
   es su número impreso (`documentRef`, hub#2368): va en el papel, lo ve cualquiera junto a la
   impresora, y es lo que permite saber qué trabajo se descarta. Un test lo fija
   afirmando que el contenido del documento no aparece **en ninguna parte** de la fila.
2. **`undrained` y `waitingSeconds` llegan RESUELTOS.** Si la pantalla rederivara el umbral
   (`UNDRAINED_ALERT_SECONDS`) habría dos definiciones de «atascado», que es exactamente lo que
   hub#987 colapsó en una. Misma regla que ya aplica la API. ⚠️ Hoy hay una segunda: la tarjeta
   «Estado de impresión» de Ajustes recalcula el estado de cada función en el shell (`classifyRole`,
   `apps/web/src/lib/print-coverage.ts:56`), y su «El dispositivo que imprimía esto no responde» no es
   el «sin atender» del hub (comprobado el 2026-10-05).
3. **Una sola definición de la forma.** `print_hosts::coverage_view` y `print_queue::status_view`
   viven en el **runtime** y las usan las dos puertas —la HTTP y la query—, en vez de dos copias
   escritas a mano. La primera que se separaría es el campo que nunca debe aparecer.

### El gate es de MOSTRADOR, no de admin

`hub.users.view`: lo lleva **cualquier sesión local** y **ninguna API key**. Es la misma audiencia
que hub#987 decidió para `GET /api/print/undrained`, y por el mismo motivo — una cola que nadie drena
necesita a **quien está al lado de la impresora**, que es quien puede encender la caja y quien está a
punto de no darle el tique a nadie. Restringirla a admin escondería la alarma justo de esa persona.

**No paginan.** Se sirven enteras, como `users.list`/`roles.list`: el tope duro lo pone
`print_queue::list` (500) y un hub con más de eso esperando tiene un problema de impresora, no de
paginación. Lo contrario que `hub.approvals.list` (hub#884), que sí pagina porque su tabla crece para
siempre.

### …pero el SELLO es del BACK OFFICE (hub#1565)

hub#1108 y hub#1532 hicieron que el hub **escriba** quién retiró o relanzó un tique, cuándo, por qué
y desde qué módulo. Nada lo **leía**: `status_view` no servía ninguna de esas columnas, así que el
sello llegaba solo a quien acababa de hacer el gesto (vuelve en la respuesta del propio
descarte/reintento) y la pregunta del día siguiente —«¿quién me tiró este tique?», «¿por qué salió
esta comanda dos veces?»— no tenía respuesta fuera de `psql`.

Leerlo obligaba a decidir algo que hub#1107 no decidía: el listado lo lee **cualquier sesión**,
también un cajero. La respuesta la dio el mercado, no nosotros
([ADR-0448](../00-overview/decision-log.md#adr-0448); la investigación, en la issue): en todo TPV
maduro «anulado por X, porque Y» vive en el **back office** —el informe de comps & voids de Square
exige el permiso `reports`, el de Toast es Toast Web, el de Lightspeed es Back Office, el de Odoo es
el grupo PoS Manager— y por la misma razón en todos: ese informe existe para detectar sisa en caja,
así que la población vigilada no puede ser su audiencia.

**Dos hechos, dos audiencias, UNA puerta.** `print_queue::QueueAudience` (`Counter` | `Admin`) la
resuelve `audience_of(ctx)` con `permissions::has(ctx, "hub.administer")` —`has`, no `check`: esto
**filtra**, no rechaza; no ser admin es una respuesta más corta, nunca un `403`—, y
`status_view(&job, audience, &names)` sigue siendo la **única** definición de la forma para las dos
puertas (HTTP y query), que es lo que hub#1107 quiso y lo que dos vistas escritas a mano habrían roto.

- **El estado de la cola sigue siendo del mostrador** (hub#987, sin mover): la misma fila, con
  `status: "discarded"`, le llega al cajero.
- **El sello viaja solo a `Admin`, y solo en la fila donde el gesto ocurrió**: un `pending` que
  nadie tocó no lleva `discardedBy: ""`, porque toda pantalla pintaría «retirado por —» sobre un
  tique que simplemente espera. Para el mostrador las claves están **AUSENTES**, no vacías: `""` ya
  significa algo en este sello («ningún módulo se nombró»), así que responderlo sería decir «nadie lo
  tiró» en vez de «no te lo cuento». Los tests fijan además que ni el motivo ni el nombre se cuelan
  por otra clave.
- **El sello dice el NOMBRE.** Se guarda `hub_user:<id>` porque es la única identidad que un cuerpo
  no puede falsificar, y es ilegible en pantalla —el mismo defecto que hub#1560 cerró para el
  dispositivo que imprime, y el que `hub.approvals.list` ya evitaba en el core—. `ActorNames::of`
  resuelve la página con **una** lectura de `hub_user` (se omite si ninguna fila trae sello: la cola
  viva que el TPV sondea cada 30 s no paga una segunda query) y viajan las dos cosas: `discardedBy`
  (id estable, para soporte) y `discardedByName` (lo que se pinta). `label()` **cae al principal,
  nunca a vacío**: una empleada que ya no está no convierte una fila de auditoría en «retirado por —».
- **Un bucket CERRADO se lee desde el otro extremo.** `ORDER BY seq` es el orden de entrega y para
  `pending`/`printing`/`dead` es el extremo correcto —el tique que más espera es el que arde—. Para
  `discarded` y `done` es el equivocado por el mismo argumento: esas filas nunca salen de la tabla, y
  un hub con un año de historia paginaría para siempre por sus primeras cien sin llegar al tique por
  el que alguien pregunta, que es siempre uno de los últimos. `list` los ordena `seq DESC`.

La variante de Lightspeed («la cajera ve solo los suyos») la tiene uno solo y obliga a comparar
identidad fila a fila: el mandato de la app más sencilla del mercado la descarta. La pantalla que lo
pinta es del módulo `printing` ([printing#36](https://github.com/ERPlora/printing/pull/36)): bucket
«Retirados recientemente», pedido solo por quien administra.

---

## Sacar del atasco UN trabajo (hub#1108)

Todo lo de arriba es **la máquina**: reparte, reintenta, se rinde. Nada de ello es **una persona**.
Hasta hub#1108 no existía la operación a ningún nivel —ni endpoint, ni op del runtime, ni command, ni
superficie SDK—, así que un trabajo `dead` se quedaba muerto para siempre y uno que nadie iba a
imprimir (una sesión de QA, un flujo mal escrito) ocupaba la cola y **bloqueaba su estación**
(`DELETE /api/print/stations/{id}` responde `409` mientras haya algo `pending`).

La forma la copia del **dead-letter del outbox** (hub#660, expuesto a módulos por hub#953): es el
mismo problema sobre la otra cola durable del runtime, y este código ya lo había respondido.

### Reintentar = REQUEUE de la fila, no re-encolar

El gesto universal del mercado es **reimprimir el documento** (Square desde Transactions, Toast
re-fire, Lightspeed desde History), pero aquí el documento no se regenera desde la vista de estado
(se regenera desde la venta, [ADR-0265](../00-overview/decision-log.md#adr-0265)). El equivalente
honesto es devolver **la fila** a `pending`, y ninguna de las alternativas funciona:

- **Re-encolar el mismo `jobId`** es un **no-op por construcción**: la PK es `(hub_id, job_id)` con
  `ON CONFLICT DO NOTHING`. Es la garantía de idempotencia haciendo su trabajo.
- **Encolar uno nuevo** no puede: el documento no viaja en la vista de estado.

**`attempts` se resetea a 0**, y no es cosmético: `MAX_ATTEMPTS` es lo que mandó el trabajo a `dead`,
así que un reintento que conservara la cuenta moriría en su primera entrega. Si la causa sigue ahí,
el trabajo vuelve a morir y vuelve a listarse — la cola se cura sola, no hace magia.

**Y desde hub#1532 el reintento también SELLA** (`retried_at`, `retried_by`, `retried_by_module`):
un tique que sale por la impresora una segunda vez es exactamente por el que alguien pregunta. Las
tres columnas las escribe **la misma `UPDATE`** que resetea el presupuesto de entregas y que lleva
el guardia `AND status = :dead`, así que un gesto negado no puede dejar el nombre de un módulo en
una fila que no movió.

**Solo un trabajo `dead`.** Los demás estados se niegan **nombrando el estado real**
(`409 print.job_not_requeueable` + `status`), para que una pantalla pueda explicar en vez de informar
de un movimiento que no ocurrió:

| Estado | Por qué no |
|--------|-----------|
| `pending` | Ya está en la cola esperando: no hay nada que reintentar. |
| `printing` | Está en manos de un host, y el **lease** ya cubre al host que se murió (`reclaim_expired` lo devuelve a `pending`). |
| `done` | El papel salió. Reimprimirlo es otro gesto de producto ([hub#525](https://github.com/ERPlora/hub/issues/525)), no este. |
| `discarded` | Es una decisión que alguien tomó. |

### Descartar = ESTADO OBSERVABLE, nunca `DELETE`

Corolario directo del diseño de hub#343 («`dead` es diagnóstico, no borrado»), y aplica más fuerte a
una decisión que a un fallo: una fila que desaparece haría invisible justo lo que pasó. El descarte
es el estado `discarded` más el **sello** — `discarded_at`, `discarded_by`, `discard_reason`
(migración de sistema **v54**, las mismas tres columnas y tipos que hub#660/hub#955 pusieron en
`_event_outbox`).

- **`discarded_by` sale SIEMPRE de la sesión resuelta** (`hub_user:<id>`), jamás del cuerpo. Mismo
  criterio que el `created_by` de las API keys.
- **Y `discarded_by_module` dice QUÉ MÓDULO lo pidió** (v58, hub#1532), porque la persona sola no
  contesta la pregunta. La puerta ya resolvía el módulo (`X-Erplora-Module`) para comprobarle el
  permiso `printer` y luego **tiraba el nombre**: un dueño con dos módulos con ese permiso que se
  encuentra tiques que él no tiró solo podía leer «lo descartó Ana» —y Ana no descartó nada—. Sale
  del **mismo sitio contra el que se comprobó el permiso** (`require_module_capability` lo devuelve),
  no de una segunda lectura de la cabecera, que es justo lo que dejaría que las dos se separaran.
- **`reason` es lo único que sí viaja en el cuerpo**: es la única mitad del sello que solo sabe quien
  cierra la fila. Se guarda recortado y con tope por `outbox::clamp_discard_reason` —**una sola
  implementación** para las dos colas durables del hub—, y lo que se devuelve es lo **guardado**, no
  lo que llegó. Es opcional: una cola que exige una redacción para cerrar una fila es una cola que
  nadie vacía.
- **Un `printing` no se descarta bajo las manos de su host** (`409 print.job_not_discardable`): el
  lease ya cubre al host muerto, y tirar a la basura un tique que un host real está renderizando
  sería justo la pérdida silenciosa que esta cola existe para impedir.
- Lo que sí lo retira algún día es la **retención**, no este gesto.

Consecuencias mecánicas que salen gratis: `claim_next` solo reclama `pending`, así que a un
`discarded` no se le vuelve a entregar a nadie; `print_stations::delete` cuenta solo
`pending`/`printing`, así que un trabajo retirado deja de tener secuestrada su estación; y
`print_hosts::coverage` cuenta `waiting` sobre `pending`, así que la campana deja de sonar.

### Las DOS puertas, y el orden importa

Mismo patrón que `outbox_admin` (hub#953), y ninguna sustituye a la otra:

1. **Sesión admin.** Deliberadamente **asimétrico** con LEER la cola, que es cualquier sesión local
   (hub#987). Tirar un tique a la basura o volver a lanzarlo es el gesto del dueño o del encargado —
   la misma clase de decisión que el CRUD de estaciones de al lado, y por la misma puerta. Poner la
   capability delante contestaría `403` a quien nunca se identificó.
2. **La capability `printer`, si quien llama NOMBRA un módulo** (`X-Erplora-Module`, que estampa
   `@erplora/module-sdk`). Sin ella, «hay un admin conectado» significaría que **cualquier módulo
   instalado** puede tirar a la basura los tiques de todos los demás. Quien no nombra ninguno —el
   shell, `curl`— pasa con la sesión sola. El chequeo es `flows_api::require_module_capability`,
   parametrizado por capability: una sola implementación de un *default-deny*, porque dos copias es
   como una acaba siendo la permisiva.

### La puerta del módulo: `erplora.forModule(<id>).printQueue`

`PrintApi` en `@erplora/module-sdk`: **dos métodos, dos rutas**, ninguno que acepte un path —la misma
disciplina que `FlowsApi`/`EventsApi`, fijada por su propio fichero de test (la lista de métodos está
clavada, el prefijo está clavado y un `jobId` que se saldría de él se rechaza antes de que exista la
petición: `fetch` **normaliza** la URL, así que un id pegado en un path es el proxy genérico entrando
por la puerta de atrás).

> ⚠️ **Se llama `printQueue`, NO `print`.** `erplora.print(req)` es un **contrato publicado**: el
> shell le engancha el servicio de impresión a esa misma instancia del cliente
> (`apps/web/src/main.ts`) y **todos** los módulos lo llaman para ENCOLAR un documento. Quedarse con
> ese nombre habría roto la impresión de todos los módulos instalados, en silencio y a la vez. Lo
> cazó el test `hub#714: the scoped client still sees what the shell hung on the singleton`.

**La lectura no pasa por aquí**: `hub.print.coverage`/`hub.print.jobs` son queries del dispatcher, así
que la pantalla las lee con `erplora().query(…)` como cualquier otro dato.

**El permiso se re-resuelve con el módulo que hace CADA petición (hub#1530).** La puerta del
servidor ya lo hace bien: `flows_api::calling_module` relee `X-Erplora-Module` de la petición y
`require_module_capability` resuelve el grant a partir de ahí, sin cachear nada por conexión. Lo que
tuvo que arreglarse fue el lado del cliente: `forModule` construye el scope con `Object.create(this)`
—delegación por prototipo, **no copia**, y eso es deliberado porque el shell le cuelga
`print`/`loadSlot` al objeto ya construido— así que un cliente **ya escopado** que se vuelve a
escopar heredaba el `PrintApi` memoizado del scope anterior, con la cabecera del módulo anterior
capturada dentro. El segundo módulo actuaba bajo el permiso `printer` que el dueño le concedió al
**primero**, y el `403` (cuando llegaba) culpaba al primero.

La regla, por tanto: **ninguna superficie memoizada se hereda entre scopes.** `forModule` limpia las
tres (`flowsApi`, `eventsApi`, `printApi`) y el guard de `print.test.ts` no las nombra — descubre los
getters que rechazan un cliente sin scope con `module_scope_required` y exige que ninguno sobreviva
al re-escopado, de modo que una cuarta superficie queda cubierta el día que se escriba.

La fila del descarte registra a la persona **y** al módulo que actuó desde la v58 (hub#1532), y
desde hub#1565 el sello se **lee** de vuelta por `hub.print.jobs` —solo con `hub.administer`, con el
nombre de la persona resuelto (ver [arriba](#pero-el-sello-es-del-back-office-hub1565)).

---

## El idioma del papel se sella en la COLA (hub#1159)

El dispositivo lleva el catálogo de las etiquetas que son suyas —`COCINA`/`KITCHEN`, `Mesa: `/
`Table: `, `CUENTA`/`BILL`— y elige columna con `document.locale`
([`crates/peripherals.md`](crates/peripherals.md#el-idioma-del-papel-catálogo-en-el-crate-locale-en-el-documento-hub1159)).
**Quién lo sella es `enqueue`, una vez, para todos los productores.**

Dejarlo en manos de cada módulo significa que cada uno tiene que acordarse en **cada** documento, y
el que se olvide saca una comanda en español por la impresora de una cocina en inglés — que es el
defecto, no una versión suave del defecto. Una puerta, todos los productores, cero cambios de módulo.

- El valor sale de `hub_settings.language` (`settings::language_of`, default `es`).
- **Un productor que ya nombró el idioma lo CONSERVA**: un tique para un cliente extranjero es una
  decisión que le corresponde, y un ajuste del hub no la puede pisar.
- Se sella **después** de las guardas de forma —un documento vacío se sigue rechazando, nunca se
  convierte en un `{"locale":"es"}` imprimible— y **después del tope de tamaño**: el tope existe
  para que un productor no encole un blob, así que mide lo que mandó **el productor**. Cargarle
  esos ~16 bytes rechazaría un documento que cabe, y lo pagaría quien esté justo en el límite.

## La escala de la moneda se sella en la COLA (hub#2129)

El renderizador pinta cada importe con `document.decimals` cifras —JPY 0, EUR 2, KWD 3— y con dos
si no viene ([`crates/peripherals.md`](crates/peripherals.md#la-escala-de-la-moneda-decimals-en-el-documento-hub2129)).
Antes pintaba siempre `{:.2}`: un hub en yenes sacaba `1500.00` en el papel mientras la pantalla
decía `1500`, y uno en dinares perdía la tercera cifra (`1.234` → `1.23`). Se sella aquí por la
misma razón que `locale`: una puerta, todos los productores (sales, invoice, cash_register,
inventory…), cero cambios de módulo.

- El valor sale de `settings::currency_decimals_of` → `currency_decimals_in`: `currency_decimals`
  declarado a mano → registro ISO-4217 de `currency` → 2. Es **la misma función** que resuelve
  `currency_decimals` de `/api/hub/context` para la pantalla, así que papel y pantalla no pueden
  discrepar.
- **Un productor que ya declaró `decimals` lo CONSERVA**: él dividió los importes, sabe cuántas
  cifras tienen.
- La vía directa a la impresora de la app (`erplora.print` con impresora del rol) no pasa por la
  cola: allí lo sella la puerta del shell (`apps/web/src/lib/print.ts`) con `hubCurrencyDecimals()`,
  con la misma regla de conservar el del productor.
- Mismo orden que `locale`: después de las guardas de forma y del tope de tamaño.

---

## Las ESTACIONES son filas, no cadenas (migración v47, hub#457)

Hasta la v47, «qué impresora imprime esto» viajaba como **cadena libre comparada literalmente**:
`enqueue` guardaba el `role` que llegara, `register` registraba el `role` que llegara y
`claim_next` unía los dos lados con `role = :role`. Nadie comprobaba nunca que ambos hubieran
tecleado lo mismo, así que **`Kitchen`, `kitchen` y `kitchn` eran tres colas distintas** — y la
tercera no tenía host jamás, en silencio, hasta que faltaba el plato.

### La decisión la tomó el MERCADO, no nosotros (15/08)

Doce referencias —Toast, Square, Lightspeed K, Odoo, Clover, Loyverse, TouchBistro, Revel, Oracle
Simphony, Shopify POS, Epson ePOS y Star CloudPRNT— más sus foros dicen todas lo mismo:

```text
  ítem / tipo de documento ──(FK)──▶ ESTACIÓN (id, nombre) ◀──(FK)── impresora / host
```

**Ni un solo sistema maduro compara una cadena tecleada en el momento de imprimir.** El vocabulario
es **abierto** (los nombres reales son *Grill*, *Frío*, *Barra 2*, *Terraza*) y lo **cerrado** es el
**enlace**, porque es una FK elegida de un selector. Clover, la única referencia con el set cerrado
de etiquetas, es la que tiene el foro lleno de comerciantes que no pueden expresar su local.

Por eso **ninguna de las dos opciones obvias**:

- **`trim`+`to_lowercase` no arregla el bug.** Colapsa `Kitchen`→`kitchen` y deja `kitchn` como cola
  huérfana propia: arregla **uno** de los tres ejemplos. El problema es *comparar cadenas*, no las
  mayúsculas.
- **Cerrar el enum a los cuatro de ADR-0196** es más seguro y se rompe en el primer restaurante real
  —y paga por ello rompiendo un contrato publicado.

### Lo que se gana además

Con estaciones como filas, `waiting > 0, liveHosts: 0` en `coverage` significa **exactamente una
cosa**: la caja está apagada. Ya no puede significar también «alguien escribió mal el nombre de la
cola», porque una cola no se crea tecleando — solo pulsando «nueva estación». La ambigüedad de
diagnóstico que pedía la issue desaparece **por construcción**, gratis.

### La tabla `_print_station`

| Columna | Para qué |
|---------|----------|
| `id` (PK) | La identidad opaca. **Esto** es lo que apuntan `_print_queue.station_id` y `_print_host.station_id`. |
| `hub_id`, `key` | **Índice único.** `key` es el nombre de cable (`kitchen`): estable, porque es lo que mandan los contratos publicados. |
| `label` | Lo que lee el dueño («Cocina caliente»). Renombrar **nunca** mueve un trabajo. |
| `created_at` | Traza. |

Sembrada con las **cuatro de hoy** (`receipt`, `kitchen`, `bar`, `label`) y `key` = la cadena en uso
— y **eso ES la garantía de compatibilidad**: `POST /api/print/jobs`, `erplora_set_device_role`,
`devices.json` y `sdk.print` siguen resolviendo sin tocarse, y los módulos (`sales` con
`role:'receipt'`, `inventory` con `role:'label'`) no se han tocado.

> **La v47 es SOLO DDL.** La siembra, la adopción, el plegado de variantes y el backfill de
> `station_id` viven en `print_stations::ensure_stations`, que corre en **cada arranque** desde
> `system_migrations::apply`. El motivo no es estético: una migración de sistema se registra **por
> BASE DE DATOS**, así que un hub que llega a una BD cuyas migraciones ya corrieron no recibiría
> nada — y ése es justo el hub legacy pre-ADR-0201 (varios hubs, una BD), que arrancaría **sin poder
> imprimir absolutamente nada**.

`ensure_stations` hace cuatro cosas, cada una idempotente por su cuenta:

1. **Siembra** las cuatro del core — **solo si el hub no tiene ninguna**. Un comerciante que borró
   `bar` no se lo encuentra de vuelta mañana; un hub con la lista vacía no puede imprimir y es o
   nuevo o nunca sembrado.
2. **Adopta** toda estación para la que este hub ya tenga un **host registrado**. Una fila de
   `_print_host` es prueba de que el comerciante configuró hardware real. Un `role` que solo vive en
   la **cola** *no* se adopta: una cadena encolada que nadie aloja **es el typo**, y canonizarlo es
   lo único que este cambio no puede hacer.
3. **Pliega** las variantes de mayúsculas del rol de un mismo equipo sobre la fila más reciente y
   normaliza el `role` a su `key`.
4. **Backfilla** `station_id` en ambas tablas donde esté vacío y haya estación que case.

> **`station_id` entra con `DEFAULT ''`, al revés que la v46, y a propósito.** En la ventana del
> start-first (ADR-0269) un `INSERT` del contenedor viejo (sin `station_id`) escribe `''`, que no
> resuelve a ninguna estación: el trabajo **espera** y el siguiente arranque lo apunta a la suya. Con
> un `NOT NULL` sin defecto el encolado del contenedor viejo reventaría y la venta se quedaría **sin
> tique**. Aquí el fallo transitorio no puede ser en abierto.

### Dónde se resuelve, y dónde ya no se compara

| Puerta | Antes | Ahora |
|--------|-------|-------|
| `print_queue::enqueue` | guardaba el `role` tal cual | **resuelve** a estación; guarda `key` + `station_id` |
| `print_hosts::register` | registraba el `role` tal cual | **resuelve** a estación; guarda `key` + `station_id` |
| `print_queue::claim_next` | `WHERE role = :role` | **`WHERE station_id = :station_id`** — el parámetro ya no es una palabra |
| `print_drain::claim` | comprobaba el registro | resuelve la estación **y** comprueba el registro (dos rechazos distintos: `422` payload / `print.role_not_hosted`) |

`trim`+`to_lowercase` sobreviven, pero **solo como tolerancia de ENTRADA**
(`print_stations::normalize_key`), nunca como identidad: `  Kitchen ` resuelve a la estación cuya
`key` es `kitchen`; no *se convierte* en una key. Es el patrón `DeviceMode::parse` (rechazar en el
borde con los valores aceptados en el mensaje) pero **contra una tabla**, así que un hub que añadió
*Terraza* la ve en el error y uno que no, no.

### CRUD de estaciones

**Runtime** (`crates/runtime/src/print_stations.rs`): `list` · `find` · `resolve` · `create` ·
`rename` · `delete` · `ensure_stations`, envueltas en `Runtime::print_stations`/
`create_print_station`/`rename_print_station`/`delete_print_station`.

| Endpoint | Auth | Contrato |
|----------|------|----------|
| `GET /api/print/stations` | sesión de usuario | `200 { ok, stations[] }`. **No** es admin: es lo que lee una pantalla para **ofrecer** un destino, y la misma lista que ya nombra el rechazo de un rol desconocido. |
| `POST /api/print/stations` | **admin** | `{ label, key? }`. Sin `key` se **deriva del label** («Barra de la terraza» → `barra_de_la_terraza`), que es el gesto real. Duplicada → `422`. |
| `PATCH /api/print/stations/{id}` | **admin** | `{ label }`. **No hay `key`**: es lo que mandan los contratos publicados y lo que ya llevan los trabajos encolados. `404` si no es de este hub. |
| `DELETE /api/print/stations/{id}` | **admin** | `404` (no es de aquí) · `409` (tiene trabajo `pending`/`printing`, o es `receipt`) · `200`. Borra **también** los registros de host que la apuntaban. |

**Por qué escribir es admin y registrarse como host no:** un host dice «este equipo, el que tengo en
la mano, drena esa cola»; esto define **qué colas TIENE el negocio**. Es la misma clase de decisión
que emitir una API key o crear un usuario, y va por la misma puerta.

**`receipt` no se borra** (`print_stations::PROTECTED_KEY`): es el destino por defecto del shell
(`apps/web/src/lib/print.ts` cae ahí cuando nadie nombra rol) y el destino al que caerá el
*fail-open* del seguimiento. Un hub sin él no podría imprimir una venta, y el dueño se enteraría en
el mostrador. Todas las demás —`kitchen`, `bar` y `label` incluidas— son del comerciante.

**Y borrar no puede tirar un tique al suelo**: con trabajo `pending`/`printing` apuntando a la
estación, el `DELETE` responde `409`. Una estación que desapareciera con tiques apuntándola sería
justo la pérdida silenciosa que esta cola existe para impedir.

---

## El módulo dice QUÉ, el hub dice DÓNDE (migración v50, hub#987)

La v47 convirtió el **destino** en una fila y dejó sin construir la flecha de la izquierda del mismo
diagrama:

```text
  tipo de documento ──(FK)──▶ ESTACIÓN (id, nombre) ◀──(FK)── impresora / host
  ^^^^^^^^^^^^^^^^^^^^^^^^^^                                  ^^^^^^^^^^^^^^^^
  hub#987 (esta sección)                                      hub#342 (registro de hosts)
```

Sin ella, `sales` llevaba `role: 'receipt'` a fuego e `inventory` `role: 'label'`: **un módulo de
negocio nombrando el periférico del comerciante**, que es la fuga que señaló la decisión de mercado
de hub#457. Un módulo no puede saber que ESTE restaurante manda sus comandas a *Grill* y sus chits a
*Barra 2*; el hub sí, porque el hub es del comerciante.

El eje que un módulo **sí** es competente para declarar ya viajaba en el mismo payload:
`documentType`, vocabulario cerrado de ocho valores. Es el *print class* de Oracle Simphony y el
split recibos/comandas/etiquetas de Square.

### La tabla `_print_route`

| Columna | Para qué |
|---------|----------|
| `hub_id`, `document_type` | PK. Un tipo de documento tiene **un** destino. |
| `station_id` | A qué estación sale. Apunta al `id` de la v47, nunca a una cadena. |
| `updated_at`, `updated_by` | Quién decidió el enrutado y cuándo. |

**Sin FK declarada hacia `_print_station`, a propósito.** Un `ON DELETE CASCADE` borraría la fila al
borrar la estación y un `RESTRICT` impediría borrarla: las dos le quitan al comerciante la traza de
que su mapa quedó roto. La fila **se queda colgando**, `list` la devuelve con la estación vacía para
que la pantalla la enseñe rota y la reapunte, y la resolución falla **abierto**.

**La siembra vive en `print_routes::ensure_routes`, no en la migración** — mismo motivo exacto que
`ensure_stations` (v47): una migración de sistema se registra **por BASE DE DATOS**, y el hub legacy
pre-ADR-0201 (varios hubs, una BD) que llega a una BD ya migrada no recibiría ninguna ruta. Corre en
cada arranque, **después** de las estaciones (una ruta apunta a un `id`).

Siembra **por hueco**, no «solo si no hay ninguna» (que es lo que hace `ensure_stations`): una ruta
no es algo que el comerciante pueda borrar —el CRUD solo la reapunta—, así que no hay ninguna
ausencia deliberada que proteger, y rellenar hueco a hueco es lo que hace que un `documentType`
nuevo de una versión futura llegue a su estación en un hub que ya existía. Un defecto cuya estación
el hub borró se **salta**: resucitarla para que la tabla quede bonita desharía la decisión del
comerciante.

### Los defectos SON la garantía de compatibilidad

| `documentType` | Estación | Por qué |
|----------------|----------|---------|
| `kitchen_order` | `kitchen` | La única que no es `receipt`, y la razón de existir del mapa. |
| `barcode_label` | `label` | Lo que `inventory` mandaba a fuego. |
| `receipt` · `invoice` · `delivery_note` · `cash_session_report` · `prebill` · `generic` | `receipt` | Lo que `sales` mandaba a fuego, y el defecto del shell. |

Reproducen **exactamente** lo que se imprimía el día antes, así que pasar un módulo a «no mandes
`role`» no cambia dónde sale nada. Un test guarda que las dos listas no se separen: un
`documentType` que la cola acepta y nadie enruta caería abierto para siempre.

### `role` es un OVERRIDE, en deprecación

Un productor que sigue mandando `role` recibe exactamente esa estación, por la misma puerta
`print_stations::resolve` de la v47. Lo que cambia es que **omitirlo es el camino normal**: los
módulos pueden dejar de nombrar hardware, uno por PR, sin prisa.

### Fallar ABIERTO, que es el sentido del fallback

Toast imprime en todas las estaciones cuando el enrutado está vacío; Odoo con la lista vacía imprime
todo. El fallo cerrado y silencioso es el de Clover y Loyverse, y sus foros son la documentación de
por qué. Aquí lo más parecido a «imprímelo igual» es caer a `receipt` —`PROTECTED_KEY`, la única que
un hub no puede borrar— y **nunca** a ningún sitio.

**Dos puertas que fallan en direcciones opuestas, a propósito:**

| Quién llama | Puerta | Qué hace con lo desconocido |
|-------------|--------|-----------------------------|
| Nombró una estación (`role`) | `print_stations::resolve` | **422** que nombra las estaciones de este hub. El productor es un humano o una UI: el typo tiene que verse. |
| Solo dijo qué es (`documentType`) | `print_routes::route_for` | **Nunca** se niega. No hay typo que enseñarle a nadie, y las únicas opciones son papel en algún sitio o papel en ninguno. |

⚠️ El fail-open es para el trabajo **que ya entró y se quedó sin destino** (mapa incompleto, estación
borrada). **No** relaja la puerta. Las dos maneras de perder el destino —sin fila y con fila
colgando— colapsan en una sola respuesta justo para que el fallback sea el único sitio que decide.

### CRUD del mapa

| Endpoint | Auth | Contrato |
|----------|------|----------|
| `GET /api/print/routes` | sesión de usuario | `{ ok, routes[], fallbackStation }`. Cualquier sesión: es lo que una pantalla lee para **mostrar** dónde sale cada cosa. |
| `PUT /api/print/routes` | **admin** | `{ documentType, stationKey }`. Admin por lo mismo que crear una estación: decide dónde sale el papel del negocio. |

Ambas mitades se validan contra la realidad: un `documentType` fuera del vocabulario sería una fila
que ningún trabajo puede casar, y una estación inexistente una ruta que nace colgando.

---

## Avisar de lo NO drenado (hub#987)

`print_hosts::coverage` sabía desde hub#800 que la cocina se estaba acumulando —`waiting > 0` con
`liveHosts: 0`, que desde la v47 significa **siempre** «la caja está apagada»—, y el único sitio que
lo decía era **Ajustes › Recibos**, una pestaña que nadie abre en mitad de un servicio. Mientras
tanto el TPV sigue cobrando, la venta cierra con un 200 y el único síntoma es papel que no sale en
otra habitación.

**Faltaban tres cosas, y son estas:**

1. **El umbral.** `coverage` decía *cuánto* esperaba, nunca *desde cuándo*. Ahora `RoleCoverage`
   lleva `waiting_seconds` —la edad del trabajo **más viejo**— y `UNDRAINED_ALERT_SECONDS` = **60 s**
   es el corte, con una definición única (`print_hosts::is_undrained`) por lo mismo que `LIVE_EXPR`:
   un umbral escrito en tres sitios son tres umbrales. La API lo devuelve para que ninguna pantalla
   se invente el suyo, igual que ya viajaba `heartbeatSeconds`.
   - **Ni más bajo**, porque `live_hosts` ya es una señal retrasada: un host cuenta como vivo
     `HOST_TTL_SECONDS` (90 s) desde su último latido, así que «no hay host vivo» ya implica ese
     silencio. Avisar al instante encima pondría un badge rojo en todas las pantallas del local cada
     vez que una caja se reinicia.
   - **Ni más alto**, porque lo que se mide es un cliente de pie esperando su tique. Un umbral lo
     bastante largo para ser educado es lo bastante largo para ser inútil.
2. **El aviso**: `GET /api/print/undrained` → `{ ok, count, thresholdSeconds, stations[] }`.
   **Sesión de usuario, no admin**, y esa es la diferencia sustantiva con el count de dead-letters
   (hub#660) que copia: un evento muerto necesita a un admin; una cola que nadie drena necesita a
   quien está en el mostrador —es quien puede encender la caja y quien está a punto de no darle el
   tique a nadie—. Restringirlo a admin escondería la alarma justo de la persona que está al lado de
   la impresora.
3. **La pantalla**: la **campana** de la topbar, con una fila por estación (`la cocina` y `la barra`
   son dos cajas y dos gestos; un total no dice a cuál ir).

### Por qué la campana y no el checklist de configuración

[ADR-0067](../00-overview/decision-log.md#adr-0067) separó las dos superficies preguntando **qué
clase de cosa** es el aviso. «No has configurado X» es estado derivado que se cura configurando, y
va al checklist del dashboard. Esto no es eso: el hub **está** configurado, y lo que va mal está
pasando **ahora**. Es la misma forma que el contador de dead-letters que ya vive en la campana
(hub#660): algo falló, alguien tiene que ir a hacer algo, y se cura solo cuando lo hace.

Sí conserva la honestidad del ADR sobre el descarte: **no hay «marcar como leído»**, porque el
estado ES la notificación. Se apaga encendiendo la caja.

Consecuencia: la campana pasa a tener **dos fuentes** y un solo número, así que `shell.ts` suma
**por fuente** en vez de dejar que la última escritura pise a la anterior — los dos pollers corren a
su ritmo y sin saber el uno del otro, y un hub con un evento muerto Y una caja apagada que enseñara
«1» estaría escondiendo una de las dos.

---

## La cola contesta si hay ALGUIEN que la drene (hub#1731)

La campana de hub#987 avisa de lo que **lleva rato** sin drenarse (60 s). Esto es lo de antes: en el
instante en que se encola, **¿hay alguien ahí?**

El agujero era que `POST /api/print/jobs` contestaba `{ok:true, status:"queued"}` y nada más. En un
hub sin ninguna impresora dada de alta, «encolado» e «impreso» se leen **igual** — así que el TPV
cobraba, decía «aquí tienes» y no salía papel: ni error, ni aviso, ni línea de log. El **fallo mudo**
que prohíbe la regla de entrega de la raíz, y encima con el panel diciendo «Impresora lista»
([system-info.md](system-info.md)).

**La respuesta ahora lleva la cobertura**, y con ella la cadena entera puede distinguir los dos
casos:

| Lo que contesta el runtime | Qué significa | Qué hace el shell |
|---|---|---|
| `liveHosts > 0` | Alguien está drenando esa estación | **Callar.** Sale tarde, no se pierde |
| `liveHosts: 0` | Encolado y **nadie va a por él** | **Avisar** (`PrintResult.awaitingHost`) |
| clave ausente | El runtime no contestó la pregunta | **Callar.** Un «no lo sé» no es un «no hay nadie» |

Esa última fila es una regla, no un descuido: un runtime antiguo no manda la clave, y convertir su
silencio en `0` pondría el aviso en **todos** los tiques de un hub bien montado — un aviso que sale
siempre es un aviso que nadie lee. Es el mismo criterio que ya regía `unknown` en la pill del panel.

**Una definición de «vivo», no tres.** `print_hosts::live_hosts_for` reusa literalmente `LIVE_EXPR`
y el mismo `live_cutoff()` que `coverage`, y un test ancla las dos contra el mismo escenario: un hub
cuya lista dijera que una caja está viva mientras la cobertura llama descubierta a su estación sería
peor que cualquiera de las dos respuestas por separado.

**Y el aviso NO tumba la venta.** `awaitingHost` es para avisar, nunca para fallar: el trabajo está
a salvo en la cola y sale solo en cuanto se dé de alta la impresora. La frase que lee el cajero lo
dice así —lo que falta es dar de alta la impresora, no reimprimir—, porque «NO se imprimió» le manda
a buscar un atasco que no existe.

**Y se dice en tono informativo, sin nombrar la venta (hub#2210).** El aviso salía en el rojo de
error justo después de un cobro correcto y nombraba la venta por su identificador interno (un UUID):
alarmaba sin motivo y no decía qué venta era. Ahora el tique en espera es un aviso **informativo**
(azul, 6 s para leer la salida); el perdido y el que no se pudo preparar siguen en rojo, y el que
salió sin el QR de VeriFactu en ámbar. Ninguno lleva el identificador —solo llegan a la caja que
cobró (hub#1980), justo tras el cobro—, y ni siquiera viaja como parámetro. Frase y tono los decide
`apps/web/src/lib/print-on-sale-notice.ts`; `main.ts` solo los pinta.

**Y la comanda en espera, en ámbar (hub#2238).** Tampoco se ha perdido —sale sola en cuanto se dé de
alta la impresora de esa estación—, pero a diferencia del tique, hasta que salga nadie en cocina
empieza el plato: alguien tiene que actuar. Por eso es **advertencia** (`warning`, 6 s) y no
informativo; la comanda que de verdad no salió sigue en rojo con su motivo. Frase, color y duración
los decide `apps/web/src/lib/print-comanda-notice.ts` con el mismo patrón que el tique; `main.ts`
solo los pinta, y un test de cable le prohíbe un color literal o volver a `toastError`.

**Y el tercer canal: el registro del hub (hub#1781).** El cajero y el panel ya lo ven, pero quien
vigila la flota no: un negocio podía cobrar sin papel durante días sin que nadie de fuera se
enterara. Así que el mismo `liveHosts: 0` deja **una línea `WARN`** en el log del hub con el código
estable `event="print.job_unattended"` más `job_id` y `role` (nada del documento). Las alertas
casan con el código, nunca con la frase. Reglas: **una por tique encolado de verdad** (un
`duplicate` es el mismo incidente y no repite), y **ninguna** mientras un equipo drene esa
estación, aunque haya otro vivo en una estación distinta. Por qué por tique y no por umbral: es el
aviso **inmediato** (el de 60 s ya existe en la campana, dentro de la pantalla) y su volumen es el
de los tiques, que ya está acotado; un hub con el módulo de impresión activo y sin impresora es
precisamente el caso que hay que ver. Lo ancla `crates/server/tests/print_unattended_log.rs`.

**Y el tique que de verdad no salió dice qué hacer, no el motivo de la máquina (hub#2239).** El aviso
pintaba tras «El tique NO se imprimió:» el motivo de la puerta tal cual («el runtime rechazó el
encolado») y, sin motivo, el literal «sin impresora» en español aunque el negocio estuviera en
inglés. Ninguno le decía al cajero qué hacer, y la salida es siempre la misma: reimprimirlo desde la
pantalla del tique. Así que la frase dice eso y nada más; el motivo va al **log** del shell con la
venta (`[print-on-sale] receipt of sale … not printed: …`), y cuando la puerta no da ninguno viaja
un código (`receipt_not_delivered`), nunca una frase en un idioma. No se traduce el motivo por
código porque la puerta aún no los da (son frases de `print.ts`) y todas llevan a la misma salida.

---

## El registro de HOSTS de impresión (migración de sistema v22, hub#342)

La cola garantiza que un tique no se pierde por no haber nadie con un dispositivo delante. La otra
mitad de la frase de ADR-0196 §6 es **quién lo va a sacar**: el equipo con la app instalada, en la
red de la impresora, que declara «yo imprimo lo de `kitchen`».

> No confundir con `erplora_set_device_role`: ese asigna rol a una **impresora** (qué caja de la red
> es la de cocina). Este registra el **dispositivo que le habla**.

### Las cuatro preguntas que decide el diseño

| Pregunta | Respuesta | Por qué |
|----------|-----------|---------|
| ¿Y si **dos** dispositivos se registran para el mismo `role`? | **Los dos son hosts.** La PK es `(hub_id, device_id, role)`: un rol es un **conjunto**, no una plaza. Y un dispositivo puede tener varios roles (la caja del mostrador imprime `receipt` **y** `kitchen`). | Dos cajas al alcance de la impresora de cocina son un **repuesto**, no un conflicto. Ya es seguro: `claim_next` reparte con `FOR UPDATE SKIP LOCKED`, así que se llevan trabajos **distintos**. Hacerlo exclusivo no compraría nada y le costaría al negocio su caja de repuesto. |
| ¿Y si **ninguno** lo está y alguien cobra? | **La venta sale y el tique espera.** `enqueue` no sabe si hay host y no debe saberlo. Lo que faltaba es que esa espera era **muda**: `coverage` informa, por rol, cuánto espera y cuántos hosts hay vivos. | Negarse a cobrar porque una impresora está desatendida sería repetir el fallo de la cola-en-el-dispositivo. Es un **aviso, nunca una barrera**. |
| ¿Cómo **caduca** el registro de un equipo apagado? | **No caduca: se deriva.** Un host está vivo mientras su `last_seen_at` esté dentro de `HOST_TTL_SECONDS` (90 s = **tres latidos** de `HEARTBEAT_SECONDS` = 30 s). Sin noticias, no está vivo. | Un equipo sin corriente **no ejecuta nada**: no puede des-registrarse. Y derivarlo gana a barrerlo con una tarea de fondo, que es una cosa más que puede estar caída — y mientras lo esté, una bandera almacenada afirmaría que una caja muerta imprime. Una respuesta derivada es correcta aunque no haya corrido nada. |
| ¿**Sobrevive** el registro a un reinicio del runtime? | **Sí, y debe.** Es **configuración**, no sesión: el hub se redespliega en cada merge. Lo que **no** sobrevive es la *vivacidad* — y no hace falta: la mentira dura como mucho el TTL y el host la recupera con su siguiente latido. | Un negocio que tuviera que re-emparejar sus impresoras varias veces por semana dejaría de usar la función. Una mentira **acotada y que se cura sola** sale mucho más barata que perder la configuración. |

**Estar apagado y estar retirado son cosas distintas**: lo primero es un accidente y deja la fila
(no viva), lo segundo es una decisión y es `unregister`. Por eso el dueño ve la caja que dejó de
contestar en vez de verla desaparecer.

### Quién llama al alta: el propio equipo, en el boot (hub#749)

Este registro se construyó entero en hub#342 y quedó **inalcanzable desde el producto**: `POST
/api/print/hosts` **no tenía ni un llamador** —ni el shell, ni la app instalada, ni un módulo—, así
que el hub contestaba `print.host_not_registered` a todo drenaje, el drenaje paraba (es un hecho de
configuración, y hace bien) y **todo lo encolado se quedaba encolado para siempre sin que nadie se
enterara**. Un mecanismo completo sin destinatario.

Lo enciende `apps/web/src/lib/print-host-registration.ts`, en el boot del shell:

1. **Los roles salen del hardware, no de un formulario.** Se dan de alta los roles de las impresoras
   que este equipo alcanza de verdad (`printerRolesOfDevices`: rol asignado **y** dirección). Un
   equipo sin impresora **no se da de alta de nada** — un móvil registrado en `kitchen` reclamaría
   tique tras tique para fallarlos todos, quemando las entregas y mandando a dead-letter trabajo que
   un TPV de verdad iba a imprimir; y el hub **no puede** detectarlo, porque para él ese dispositivo
   es un host legítimo.
2. **Se releen cada latido**, no solo en el boot: el dueño asigna el rol `kitchen` a la segunda
   impresora en Ajustes y eso tiene que imprimir sin cerrar y abrir la aplicación.
3. **El alta ENCIENDE el drenaje.** El socket no se abre hasta que el hub ha confirmado al menos un
   rol: el drenaje no reintenta una negativa de configuración, así que conectar un tick antes
   gastaría su único intento en `print.host_not_registered` y este equipo no drenaría nunca más.
4. **La cadencia la manda el hub** (`heartbeatSeconds` de la respuesta), y `refreshed: 0` en el
   latido no es un error: significa «aquí no alojas nada» y dispara un alta nueva en vez de latir
   contra la nada para siempre.

> **Por qué el alta es automática y no un botón.** hub#342 imaginó una pantalla («este equipo
> imprime: recibos / cocina / barra») que nunca se hizo. Pero el gesto que recogería es un hecho que
> el equipo ya sabe —qué impresoras alcanza—, y el drenaje ya exige esa misma condición por su
> cuenta (`peripherals.detect()`), así que el botón le pediría al dueño que confirmara algo que la
> app conoce. **Lo que sigue faltando es la otra dirección**: la pantalla de COBERTURA que le dice
> al dueño «nadie está imprimiendo lo de cocina» (`GET /api/print/hosts`, que ya devuelve
> `coverage`). Eso es informar, no registrar, y es lo que queda abierto de hub#749.

### La tabla `_print_host`

| Columna | Para qué |
|---------|----------|
| `hub_id`, `device_id`, `role` | **PK compuesta.** El `device_id` es el `X-Device-Id` de ADR-0154: un identificador, nunca una credencial. Desde la v47 el `role` es siempre la `key` **canónica** de la estación: `register` resuelve lo que llegó en vez de guardarlo. |
| `station_id` | La estación que aloja (v47, hub#457). Borrar la estación se lleva la fila: un host de un destino que ya no existe seguiría apareciendo vivo para nada. |
| `label` | El nombre que lee el dueño («Caja del mostrador») en vez de un id opaco. |
| `registered_at`, `registered_by` | Auditoría de **cuándo y quién** puso este equipo a imprimir este rol. Una reconexión **no** los mueve: un parpadeo de red no es una decisión nueva. Mismo criterio que `mode_set_by` en `device-mode`. |
| `last_seen_at` | Últimas noticias. Un latido **y** un re-registro cuentan como noticias. |

**No hay columna `live`, a propósito** — ver la tercera pregunta. Índice `(hub_id, role, last_seen_at)`.

> **La migración es RE-EJECUTABLE (`IF NOT EXISTS` en tabla e índice), y eso no es cosmética.**
> `_hub_system_migrations` guarda **versiones, no esquema**, así que cualquier cosa que borre filas
> de ahí hace que `apply` vuelva a lanzar el SQL contra objetos que ya existen. No es hipotético:
> el fixture de `tests/access_email_backfill.rs` (hub#436) rebobina `version >= 19` para re-probar
> su backfill, y ese `>=` se lleva **toda** migración posterior. Con un `CREATE TABLE` a secas, la
> v20 murió con `42P07` y tumbó **15 tests de la suite de otro** — el fallo aparece lejos de quien
> lo causa, que es lo que lo hace caro. Lo fija su propio test
> (`the_print_host_registry_can_be_applied_twice`), no el de hub#436.
>
> El resto del catálogo **no** tiene esta propiedad (11 `CREATE TABLE` a pelo) y se libra solo por
> accidente de orden, porque está *por debajo* de la v19: [hub#483](https://github.com/ERPlora/hub/issues/483).
>
> **Números: se coge el siguiente POR ENCIMA del máximo, no el más bajo libre.** Esta migración se
> renumeró **dos veces**: nació v19 y chocó con hub#436; pasó a v20 y hub#470 metió una v21 antes de
> que aterrizara, lo que dejó la **v20 libre pero inalcanzable** (por debajo del máximo, así que
> `apply` no la correría nunca). Acabó en **v22**. Y **renumerar es gratis, RENOMBRAR rompe**: el
> número es un accidente del orden de merge, el `name` es el contrato — siempre fue `print_host`.
>
> Corolario para los tests de guarda: «justo antes» es **la anterior del catálogo**, no
> `version - 1`. Con huecos reales (v15, v20) restar uno apunta a un número que no existe.

#### El nombre NO se escribe aquí: es el del DISPOSITIVO (hub#1560)

`label` no lo inventa el registro de impresión. Este hub **ya sabe** cómo llama el negocio a cada
aparato —`hub_trusted_device.name` (hub#494): nace de la plataforma que el aparato anunció en su
primer login online y lo renombra un administrador en Ajustes → Dispositivos—, así que el alta lo
**reutiliza** en vez de abrir un segundo nombre para la misma tablet.

`POST /api/print/hosts` lo resuelve en este orden, y el orden **es** la decisión:

1. **La `label` que mande el cliente**, si manda alguna: lo que sabe quien instala gana.
2. **El nombre que le puso el NEGOCIO** (`Runtime::device_name`, scoped al `hub_id` como toda
   lectura de `hub_trusted_device`, hub#489).
3. **Lo que el propio aparato anuncia** (`default_device_name` sobre el User-Agent: «Chrome ·
   Android»), que es exactamente con lo que nace la fila de `hub_trusted_device` — así las dos
   pantallas dicen las mismas palabras del mismo aparato en vez de dos nombres distintos.
4. `""`, y entonces `register` **conserva** el nombre que la fila ya tenía. Un hub que no sabe nada
   de un dispositivo no borra por eso el nombre que hay en la pantalla del dueño.

El shell entra por el 2/3, no por el 1: `apps/web/src/lib/print-host.ts` manda `{ role }` y nada
más, y un navegador no tiene mejor dato del nombre de la máquina que su propio User-Agent —que el
hub ya lee mejor que él—. Sin esto, hub#1527 («Imprime desde: …») pintaba el `device_id`, que es
`dev_` + 32 hex (`apps/web/src/lib/device.ts`): un código que no distingue una caja de una tablet.

**Y el nombre SIGUE al dispositivo, no se congela en el alta.** Renombrar en Ajustes → Dispositivos
(`PUT /api/devices/:id`, `devices::rename`) reescribe también el `label` de las filas de
`_print_host` **de ese dispositivo y de este hub**: es el único sitio donde un negocio renombra
nada, y sin ese paso la misma tablet contestaría a «Barra» en una pantalla y al nombre viejo en la
otra hasta el siguiente arranque de la app. Un nombre **en blanco no viaja**: en la lista significa
«no le he puesto nombre», y propagarlo devolvería el `dev_…` opaco a la tarjeta de impresoras, que
es justo el defecto que se está cerrando.

### Superficie

**Runtime** (`crates/runtime/src/print_hosts.rs`): `register` · `heartbeat` · `unregister` ·
`list` · `coverage`, envueltas en `Runtime::register_print_host`/`print_host_heartbeat`/
`unregister_print_host`/`print_hosts`/`print_coverage` (scoped al `hub_id` del despliegue).

**HTTP** (`crates/server/src/print.rs`):

| Endpoint | Auth | Contrato |
|----------|------|----------|
| `POST /api/print/hosts` | sesión de usuario | `{ role, label? }` para el `X-Device-Id` **del que llama**. Idempotente. Responde `heartbeatSeconds`: **el hub le dice al host cada cuánto latir**, para que el cliente no fije un intervalo que pueda separarse de la ventana del hub. Un `role` que no resuelve a estación → **`422`** (v47): un equipo no puede quedarse como host vivo de una cola que ningún productor va a llenar. |
| `POST /api/print/hosts/heartbeat` | sesión de usuario | «sigo aquí», para **todos** los roles de ese equipo (una app, una conexión, un latido). `refreshed: 0` es un éxito: significa «aquí no alojas nada, regístrate otra vez». |
| `GET /api/print/hosts` | sesión de usuario | El registro (`live` resuelto) **+ `coverage`** por rol. Cada host trae `name` (lo que pinta toda pantalla); su **`deviceId` solo sale hacia un administrador o hacia ese mismo dispositivo** (hub#2551, ver abajo). Se filtra, no se rechaza: el cajero sigue leyendo quién imprime. |
| `DELETE /api/print/hosts?role=&deviceId=` | usuario para el **propio** equipo; **admin** para el de otro | Retirar de un rol o de todos. |

**Por qué basta una sesión de usuario para escribir:** en las puertas de escritura el sujeto es
siempre el `X-Device-Id` **del que llama** — no hay parámetro para nombrar a otro—, y el gesto real
es «este equipo, el que tengo en la mano, imprime lo de cocina». Tiene que funcionar **al arrancar
la app**, mucho después de que quien configuró el hub se fuera a casa. La excepción es retirar el
equipo de **otro** (la caja sustituida o robada), que sí pide **admin**: es el único gesto que
alcanza a un dispositivo que quien llama no tiene delante, y sin él no habría forma de limpiar una
caja apagada —no puede retirarse sola—. Misma asimetría que `/api/device/mode`. El corte va por el
**sujeto**, no por el parámetro: nombrar el propio equipo explícitamente sigue siendo el propio.

`coverage` devuelve **hechos** (`role`, `waiting`, `liveHosts`, `liveHostLabels`), no una frase: el
texto que lee el dueño («no hay nada imprimiendo los tiques de cocina») lo compone la UI, que es la
capa que sabe traducirlo. El runtime no hornea cadenas visibles.

**El contador Y los nombres** (hub#1527). `liveHosts` responde a la alarma —«nadie recoge lo de
cocina»— y `liveHostLabels` al estado sano, que es el que el dueño abre a diario: con una caja y una
tablet, *«2 dispositivos»* le deja ir a probar cuál de las dos imprime. Los nombres viajaban en el
`hosts[]` de `GET /api/print/hosts` y se perdieron al leer la cola por el dispatcher (hub#1107) —el
hub sabía la respuesta y no cruzaba—. Un nombre **no** es una cadena horneada: es la etiqueta que
escribió el usuario al registrar el equipo, y un host que se registró sin ella se lista por los
**cuatro últimos caracteres** de su `device_id` tras una elipsis (`…e7f8`,
`print_hosts::display_name`): una entrada vacía pintaría «Imprime desde: » y se leería como un
fallo, y el id entero no puede salir.

**El `device_id` no es un nombre: es la prueba de dispositivo de confianza** (hub#2551). Las
puertas de acceso tratan un navegador como la caja de confianza por el `X-Device-Id` que presenta
(`auth_api.rs`, hub#2510), y el hub no distingue al dispositivo de quien copie su id. Hasta
hub#2551 lo enseñaban a cualquier sesión `GET /api/print/hosts` (`deviceId` de todos los hosts),
`GET /api/print/undrained` y `hub.print.coverage` (`liveHostLabels` caía al id): un empleado podía
copiarlo y hacerse pasar por la caja. Ahora `name` (y `liveHostLabels`) es la etiqueta o `…` + la
cola del id en las tres lecturas, y `deviceId` solo viaja hacia un **administrador** (retirar la caja
perdida la nombra, `DELETE …?deviceId=`) o hacia el **propio** dispositivo (ya lo tiene; el alta
`POST` se lo devuelve). `/api/device/mode` no cambia: contesta solo por el dispositivo que llama.

Ambos se resuelven con **un solo `cutoff`**: el recuento y los nombres son la misma pregunta a la
misma tabla, y un host vivo para una lectura y no para la otra pondría un nombre en una estación que
el contador da por descubierta. Los nombres son una **segunda lectura** de `_print_host` y no una
columna del agregado porque juntar cadenas en una fila es `group_concat` en SQLite y `string_agg` en
Postgres —dos dialectos para una respuesta—, y una etiqueta que llevara el separador volvería como
dos dispositivos.

> **No cascadea con `untrust_device`.** Un host de impresión **no** tiene por qué ser un dispositivo
> de confianza (la confianza va del login online; una caja que nunca hizo login cloud debe poder
> imprimir igual), así que el registro **no** cuelga de `hub_trusted_device` como sí hace el
> `device-mode`. La contrapartida es que revocar la confianza de un equipo robado **no** le quita el
> registro de impresión: eso se hace con el `DELETE` admin.

---

## El DRENAJE: quién saca el papel (hub#343)

Con hub#341 la cola existe y con hub#342 el hub sabe **quién** debería vaciarla. Faltaba que
alguien la vaciara de verdad, y ese es el canal `GET /ws/print` — el **primer WS
cliente→servidor** de este hub. `/ws` es *push-only*: reparte eventos y no escucha nunca, por eso
hub#342 tuvo que dejar el latido del host en una puerta HTTP «provisional, hasta que alguien abra
la otra dirección».

### Quién puede conectarse (dos capas, y ninguna sustituye a la otra)

| Capa | Pregunta | Credencial |
|------|----------|------------|
| **Autenticación** | ¿hay una persona con sesión abierta en este hub, en este equipo? | la **sesión del hub** (`hello.session`, el mismo token que viaja como `X-Hub-Session`) |
| **Autorización** | ¿este equipo es host de impresión, y de qué roles? | el registro `_print_host` (hub#342), **releído en cada `claim`** |

La sesión sola no basta porque **por la cola viaja el DOCUMENTO** —nombres, líneas, totales, el QR
fiscal—: el móvil de un camarero con sesión perfectamente válida podría drenar todos los tiques
del negocio. El registro solo tampoco basta: el `device_id` es un **identificador, nunca una
credencial** (ADR-0154) y cualquiera escribe uno.

> **El token viaja en el primer frame, no en la URL.** Un navegador no puede poner cabeceras en el
> handshake de un WebSocket, así que solo quedan el query string y el cuerpo de un frame. El query
> string acaba en cada log de acceso y en cada traza de proxy del camino; el frame, no.

### Las negativas, cada una con su código

| Código | Cuándo |
|--------|--------|
| `print.not_ready` | cualquier frame **antes** de `hello` — un socket sin autenticar no reclama, ni confirma, ni late |
| `unauthenticated` | `hello` sin sesión, o con una que este hub no reconoce (incluida **la de otro hub**: su *mensaje* distingue los dos casos) |
| `print.device_required` | `hello` que no dice qué equipo habla |
| `print.host_not_registered` | equipo que aquí no aloja nada — **también el registrado en otro hub**, cuyas filas no están en este registro |
| `print.role_not_hosted` | host de impresión real estirando la mano hacia el rol de otro |
| `print.frame_too_large` | un par empujando más de 8 KiB por un socket que quizá ni se ha autenticado |

Son **distinguibles a propósito**. Si dos guardas contestaran lo mismo, cualquiera de las dos
podría borrarse y ningún test se enteraría. Y protegen cosas distintas: la primera impide **leer**
tiques ajenos, la segunda impide que la caja del mostrador cierre el trabajo de cocina — que no
filtra un tique, lo **pierde**.

El orden también cuenta: al no registrado se le niega **antes** de mirar el trabajo, para que la
puerta de confirmación no sirva para averiguar qué `jobId` existen.

### Las tres decisiones del drenaje

**1 · El host imprimió el papel y se cayó antes de confirmar → se REIMPRIME.**

Vence el lease, el trabajo vuelve a `pending` y el siguiente host lo saca otra vez. Se elige el
duplicado a sabiendas: la alternativa —dar por entregado un trabajo en cuanto su host se calla— nos
cambia un papel de más por un **cliente que se va sin comprobante** y una comanda que la cocina
nunca recibió. Un duplicado es visible, barato y lo arregla una persona; un tique que nunca existió
no es ninguna de las tres cosas.

Lo que mantiene honesta la elección es que está **acotada y es observable**: cada reimpresión quema
una entrega (`attempts`), `MAX_ATTEMPTS` la mandan a `dead`, y la cuenta sale en el listado.

Y el **cliente paga su parte** para que el duplicado cueste solo lo que debe: `print-drain.ts`
recuerda los tiques impresos cuya confirmación no llegó y **los reconfirma nada más reconectar**,
*antes* de reclamar nada. Sin eso, cada parpadeo de red costaría un tique de más; con eso, solo lo
cuesta un equipo que se murió de verdad. Es seguro porque `jobId` es la clave de idempotencia en las
dos puertas: confirmar dos veces es una confirmación.

**2 · Un trabajo reclamado se retiene 90 s (`DEFAULT_LEASE_SECONDS`), ni más ni menos.**

Fijado por los dos extremos, porque son dos decisiones de producto y ninguna se deduce de la otra:

- **no menos que la ventana de vivacidad** (`HOST_TTL_SECONDS`, 90 s): quitarle el trabajo a un host
  que el hub **todavía da por vivo** sería duplicar el tique sin ningún motivo. Reencolar tiene que
  significar «hemos dejado de creer en este equipo»;
- **no más de tres minutos**: una caja que se colgó con el tique en la mano deja al cliente esperando
  en el mostrador, con la caja de repuesto al lado sin permiso para ayudar.

Un host colgado **retiene UN tique, nunca la cola**: `SKIP LOCKED` reparte, así que todo lo que
va detrás sigue saliendo por la caja de repuesto mientras tanto.

**3 · El lease vencido se recupera cuando alguien pide trabajo, no con un barrendero de fondo.**

`reclaim_expired` existía desde hub#341 y **no lo llamaba nadie**: un host que moría dejaba el tique
clavado en `printing` para siempre. Se arregla llamándolo dentro de `claim`, no con una tarea
periódica — el mismo criterio con el que `print_hosts` **deriva** la vivacidad en vez de guardarla:
un barrendero es una cosa más que puede estar caída, y mientras lo esté el tique sigue clavado sin
que nadie se entere. Una respuesta perezosa es correcta aunque no haya corrido nada.

Corolario: **drenar cuenta como noticias**. Un `claim` con éxito refresca el `last_seen_at` del
equipo; sin eso, una caja imprimiendo a destajo cuyo temporizador de latido se hubiera atascado
saldría en `coverage` como «no hay nadie imprimiendo» con el papel saliendo.

### El protocolo

```text
 cliente                                  hub
   │  {"type":"hello","session":…,"deviceId":…}  ─▶
   ◀─ {"type":"ready","deviceId":…,"roles":[…],"heartbeatSeconds":30}
   │  {"type":"claim","role":"kitchen"}          ─▶
   ◀─ {"type":"job","jobId":…,"role":…,"html":…,"format":…,"attempts":1}
   │      … imprime vía crates/peripherals …
   │  {"type":"done","jobId":…}                  ─▶
   ◀─ {"type":"ack","jobId":…,"confirmed":true}
   ◀─ {"type":"wake","role":"kitchen"}   (acaba de encolarse algo de un rol tuyo)
```

- **`ready` lo dicta el hub, no el cliente**: los `roles` salen del registro y la cadencia del
  latido también (igual que `heartbeatSeconds` en `POST /api/print/hosts`). El cliente nunca
  adivina para qué sirve.
- **`wake` es la razón de que esto sea un socket** y no un *polling*: el hub avisa de que ha
  llegado un tique en vez de que el host pregunte cada segundo. Es un **empujón, no una entrega** —
  lleva el `role` y nada más, tal como lo mandó el productor, solo sin espacios en los extremos
  (`crates/server/src/print.rs:171-174`): vacío si el productor no lo manda, y sin pasar a minúsculas; el documento solo viaja en la respuesta a un `claim`, que es el frame
  que pasa por las guardas. Lo publica `POST /api/print/jobs` **solo en un encolado real**: despertar
  a todo el mundo por un `duplicate` los mandaría a una cola vacía. ⚠️ Viaja por el broadcast
  compartido, y **`/ws` lo reparte a cualquiera: ese canal no pide credencial** — hueco heredado,
  encontrado aquí y registrado en [hub#504](https://github.com/ERPlora/hub/issues/504). Por eso este
  frame lleva el rol y **nada más**.
- **`idle`** (nada que hacer) es una respuesta, no un error.

### El latido SE MUEVE aquí (y la puerta HTTP se queda)

El latido de un host **conectado** viaja por este socket (`beat`), y además cualquier `claim` con
éxito ya cuenta como noticias. `POST /api/print/hosts/heartbeat` **se queda**, y no como resto:
**registrar sigue siendo un gesto HTTP con un actor detrás**, y un host recién registrado —o entre
reconexiones— tiene que poder decir «sigo aquí» sin socket. Lo que se mueve es el latido del host
conectado, que es exactamente lo que hub#342 dejó pendiente.

### Superficie

**Runtime** (`crates/runtime/src/print_drain.rs`): `claim` · `confirm` · `report_failure` ·
`hosted_roles`, envueltas en `Runtime::claim_print_job`/`confirm_print_job`/`fail_print_job`/
`print_roles_of_device`. `print_queue::role_of` (nuevo) es lo que permite comprobar el rol de un
trabajo antes de dejar que alguien lo cierre.

**Server** (`crates/server/src/print_ws.rs`): `GET /ws/print`. El manejador de frames está separado
del bucle del socket a propósito, para que **cada guarda se pruebe sin transporte**; el transporte
se prueba aparte contra un servidor atado de verdad (`crates/server/tests/print_drain_ws.rs`).

**Shell** (`apps/web/src/lib/print-drain.ts`): el bucle del cliente. Cómo un documento se convierte
en papel se **inyecta** (`printJob`) — es la parte que cambia entre una térmica en red y el diálogo
del navegador, y la única que este fichero no puede probar. No reconecta contra una negativa
definitiva (`unauthenticated`, `print.host_not_registered`, `print.device_required`): son hechos de
configuración, no meteorología, y reintentarlos en bucle machacaría el hub escondiendo el problema.

---

## El documento que viaja

> [ADR-0265](../00-overview/decision-log.md#adr-0265), hub#501. **Decisión de Ioan, 2026-08-08.**

Con hub#341/#342/#343 en verde, **no había salido un solo tique por una impresora térmica**. Las dos
mitades hablaban idiomas distintos: la cola guardaba `html` y `crates/peripherals` renderiza
documentos **estructurados** (`erplora_print(printer_id, document_type, data, job_id)` →
`escpos::render_document`). **No hay ninguna entrada que acepte HTML**, así que el host reclamaba el
trabajo y lo confirmaba sin tener con qué convertirlo en papel.

### Las tres decisiones

**1 · El documento viaja ESTRUCTURADO. El traductor HTML→ESC/POS NO se escribe** —ni ahora ni como
puente. Con un dato estructurado el mismo tique se renderiza a **pantalla, 58 mm, 80 mm o PDF**, y se
puede **volver a renderizar mañana** cuando el negocio cambie de impresora; con un HTML congelado
tienes una **foto atada al ancho de impresora de aquel día**, que para el resto de destinos no sirve.
Lo refuerza el requisito de producto: el tique tiene que poder **reimprimirse en cualquier momento** y
**abrirse desde el listado de ventas**.

**2 · No se guarda el documento: se REGENERA desde la venta.** La venta ya está en la BD con todo lo
necesario; guardar el render es duplicar un derivado, y un derivado guardado se desincroniza. En
Object Storage va **sólo lo que debe ser evidencia inmutable**: el **XML fiscal** y, si se quiere, el
**PDF de la factura** — ahí sí importa que el byte sea el que fue, porque lo respalda una obligación
legal. **El tique de pantalla no se almacena.**

> Corolario que conviene no perder: **`_print_queue` NO es un archivo de tiques**, es una cola de
> trabajo. La fila viva lleva el documento sólo porque un trabajo **en vuelo** no puede cambiar
> debajo del host que lo está imprimiendo — que es lo que la idempotencia de `jobId` ya exigía
> («el documento que va a producir la impresora es el que se encoló primero»).

**3 · La fuente de verdad del contenido es el DATO DE LA VENTA, no la pantalla que pidió imprimir.**
Si el tique se regenera desde la venta, quien lo compone lee la venta. Una caja que compusiera sus
propios totales sería una caja que decide lo que dice el tique, **importes incluidos**. Por eso el
constructor es **uno solo**: primera impresión, reimpresión y el tique que se abre en el listado
llaman al mismo, o la copia que tiene el cliente y la que lee el dueño dejan de coincidir. Es
determinista **a propósito**: es la propiedad sobre la que se sostiene «regenerar en vez de guardar».

> **Dónde vive ese constructor: en el módulo `sales`, no en el shell**
> ([ADR-0493](../00-overview/decision-log.md#adr-0493), hub#1921). La primera versión lo puso en el
> shell (`apps/web/src/lib/receipt-document.ts`) y el módulo `sales` creció el suyo para la pantalla
> y la reimpresión (`erp-sales-document` → `printableDocument()`, sales#79/#308). Dos constructores
> del mismo papel se separaron justo como este apartado avisaba: cuando `sales` pasó a guardar el
> dinero en unidades menores y la cantidad en millonésimas (ADR-0400), el del shell siguió copiando
> las filas en bruto y el tique automático salió con **«1000000x … 1190.00» / «TOTAL 1190.00» /
> «Pago: Card»** mientras el botón de imprimir, sobre la misma venta, sacaba «1x … 11.90» / «Pago:
> Tarjeta». Quien sabe lo que significa una venta —unidades, escala de la moneda, etiqueta del pago
> en el idioma del hub, identidad del negocio, número fiscal— es el módulo dueño, así que el
> constructor es el suyo y el shell **se lo pide**: `apps/web/src/lib/sale-document.ts` monta el
> visor fuera de la vista, espera a que cargue su venta, toma `printableDocument()` y lo desmonta.
> Solo usa la superficie pública del visor (`saleId` + `printableDocument()`); en un equipo que
> nunca abrió el TPV carga antes el bundle de `sales` (`loadModuleElement`). Si el papel no se puede
> componer, **no se imprime nada y se avisa** (`sale_document_unavailable` /
> `sale_document_timeout`): mejor sin papel —se reimprime desde la pantalla del tique— que con
> importes falsos. `receipt-document.ts` se borró.
>
> **Y espera al número fiscal y al QR, como la pantalla**
> ([ADR-0494](../00-overview/decision-log.md#adr-0494), hub#1867). Tomar el **primer** documento
> del visor imprimía el tique antes de que existieran la factura y su registro VeriFactu: medido en
> `salon-lucia-and18` (v1.1.28, sales 2.16.69), el papel automático salió en el mismo segundo del
> cobro con `Ticket: 20260919-0004` y **cero** comandos QR, mientras la pantalla —el mismo visor—
> esperaba tras «Emitiendo el tique…» y pintaba `TICKET-2026-000009` con el QR dos segundos
> después. Ahora el shell pone `issuing = true` **antes** de montar el visor (la espera de
> sales#308) y le pide el papel solo cuando `issued()` dice que la espera ha terminado: llegó el QR,
> no va a llegar nada fiscal (sin `invoice`/`verifactu`) o se agotó su tope de 10 s. Con el tope
> agotado el papel **sale igual** —la caja nunca se queda sin tique— y la caja lee
> `print.ticketWithoutFiscal`, que manda a reimprimir desde la pantalla del tique para dar el
> completo. El tope propio del shell (15 s) cubre el del visor; un `sales` sin `issued()` imprime
> su primer documento, como antes, así que hub y módulo se despliegan en cualquier orden.

### El vocabulario es cerrado, y se comprueba en las DOS puntas

`DocumentType::from_wire` mapeaba lo desconocido a `Generic`, así que un `Kitchen` o un `kitchn`
imprimía un volcado sin nombre de pares clave/valor donde la cocina esperaba una comanda: **falla en
silencio y no se descubre hasta que falta el plato**.

> **Cerrado quiere decir que añadir un documento es un cambio de contrato, no que la lista esté
> completa.** La CUENTA previa (`prebill`) faltaba, y con ella el papel que **más veces sale en un
> servicio de restaurante** —una vez antes de cada cobro, o sea más que el tique fiscal—: las dos
> puertas la rechazaban a propósito y el camarero no tenía nada que llevar a la mesa (hub#748). La
> salida NO era reabrir la puerta permisiva; era **añadir el nombre a las dos listas y escribir su
> renderizador**. Añadir un tipo nuevo son siempre tres sitios: `DOCUMENT_TYPES` (cola),
> `DocumentType::parse` + su brazo de `render_document` (dispositivo) y, si es papel térmico,
> `isThermalDocument` en `apps/web/src/lib/print.ts` — esa tercera es la que decide si sin Bridge
> se **encola** o se manda al diálogo del navegador, y `prebill` caía del lado equivocado.

| Guarda | Dónde | Qué contesta |
|--------|-------|--------------|
| `enqueue` rechaza el tipo | `print_queue.rs` (hub) | `invalid_payload` **con el vocabulario en el mensaje** |
| `enqueue` rechaza la forma | `print_queue.rs` (hub) | `document must be a JSON object` |
| `DocumentType::parse` | `escpos.rs` (dispositivo) | `UnknownDocumentType` |
| `render_document` rechaza la forma | `escpos.rs` (dispositivo) | `InvalidPayload` |

Dos capas con **respuestas distintas**, a propósito: si contestaran lo mismo, una podría borrarse y
ningún test se enteraría. La puerta laxa `from_wire` **ya no existe** (hub#578): se quedaba para el
`apps/bridge` 🪦, y hub#340 borró a su único llamador — una puerta permisiva sin cliente solo espera
a que alguien la llame por costumbre, así que hoy `parse` es la única y **ninguna ruta puede
degradar a `Generic` en silencio**. La comprobación de forma existe porque los renderizadores leen
**por clave**: con un array o
una cadena no encuentran nada, toman todos los valores por defecto y cortan **papel en blanco sin
error**.

### La CUENTA (`prebill`) no es un tique, y el papel lo dice (hub#748)

`render_prebill` es un renderizador **propio**, no `render_receipt` con una bandera: lo que la
cuenta no puede llevar es justo lo que un tique existe para llevar, así que compartir el cuerpo
dejaría el mobiliario fiscal a un `if` olvidado de distancia del papel que el camarero entrega.

Lo que se cae, y por qué ([ADR-0141](../adr/ADR-0141.md)):

| Fuera del papel | Por qué |
|-----------------|---------|
| Número de serie (`receipt_id`) | La numeración se consume en `complete_sale`, al COBRAR, no antes. |
| QR VeriFactu (`qr_data`) | Todavía no hay registro de facturación al que apuntar. |
| Método de pago, entregado y cambio | No se ha cobrado nada. |

Se caen **en el renderizador**, no confiando en el productor: un módulo que mande su documento de
tique por error saca una cuenta, nunca un papel que pase por factura. Y el **aviso tiene valor por
defecto** (`notice`, que el productor manda ya traducido): si dependiera de que todos los
productores se acuerden, el día que uno se olvide el cliente se lleva a casa algo que parece una
factura. Eso es un problema legal, no estético.

> **La puntuación tipográfica llegaba al papel como `?`.** La impresora habla cp437 y el codificador
> sustituye lo que no sabe representar, así que el aviso traducido —«Cuenta — no es una factura»,
> con la raya larga que escribe cualquier traductor— salía «Cuenta ? no es una factura». Lo arregla
> `to_printable` en `escpos.rs` (rayas, comillas curvas, puntos suspensivos, espacios raros y `€` →
> `EUR`) porque el texto lo escriben personas en un editor y eso no va a dejar de pasar. El español
> acentuado nunca fue el problema: cp437 lo tiene.

### La leyenda «VERI\*FACTU» va bajo el QR fiscal (sales#327)

El RD 1619/2012 (art. 6.5.b, y 7.5 para la factura simplificada, que es el tique) obliga a poner
**«VERI\*FACTU»** o «Factura verificable en la sede electrónica de la AEAT» junto al QR de toda
factura emitida por un sistema que remite **todos** sus registros. La Orden HAC/1177/2024
(art. 20.1.b) pide además una letra **bien visible, de tamaño similar** al resto de datos. ERPlora
remite siempre: no existe el modo NO VERI\*FACTU, y lo que no sale en el momento sale después como
envío tardío. Por eso la leyenda acompaña **a todo QR fiscal**, sin más condición.

| Superficie | Clave | Quién la pinta |
|------------|-------|----------------|
| Papel térmico | `qr_legend` | `escpos::render_receipt`, justo debajo de `qr_data`, centrada y en negrita |
| Tique en pantalla | `qr_legend` de `ReceiptData` | `<ok-receipt>` (outfitkit), 11 px en negrita, antes de `qr_note` |
| Factura A4 | `qr_legend` de `InvoiceData` | `<ok-invoice>` (outfitkit), 12 px en negrita (el cuerpo de la A4) |
| Papel del navegador | `qr_legend` | `receiptToPrintableHtml` de `sales`, 12 px en negrita |

- Es una **clave propia** y no se mete en `qr_note`: esa nota es un pie de 8 px y cambia cuando la
  AEAT contesta con el CSV. La nota sigue igual y la leyenda se suma a ella.
- **Solo con el QR**: sin `qr_data` el renderizador no la imprime aunque el productor la mande, porque
  sola afirmaría una verificación que el papel no ofrece. La cuenta (`prebill`) no lleva ninguna de las
  dos cosas.
- Se escribe la forma corta, que es igual en todos los idiomas y no pasa por el catálogo i18n
  (`VERIFACTU_LEGEND` en `sales ui/lib/document-mappers.ts`).
- Va **justo debajo** del QR, centrada, y **antes** del segundo QR («pide tu factura», hub#963), para
  que no se lea como el pie de ese otro código. No es una elección de diseño: lo fija la
  especificación de la sede de la AEAT («Detalle de las especificaciones técnicas del código QR de
  la factura», v0.5.0, §3), que además pide letra **igual o superior** a la del resto de datos.
- El rótulo y la posición van en la sección siguiente; el QR del papel del navegador, en la de después
  (sales#340).
- Despliegue en cualquier orden: un hub viejo ignora la clave, y una outfitkit vieja no la pinta. La
  leyenda aparece en cada superficie en cuanto su pieza está desplegada.

### «QR tributario:» encima del QR, y el QR abre el documento (sales#339)

La misma especificación de la sede (v0.5.0, §3, a la que remite la Orden HAC/1177/2024 art. 21.1)
pide dos cosas más: el QR va **siempre precedido** del texto «QR tributario:», **encima** y centrado,
en letra igual o mayor que el resto de datos; y el QR se sitúa **al principio** del documento, antes
de todo lo que escribe el programa. Si hay más QR, el fiscal va primero.

| Superficie | Clave del rótulo | Quién lo pinta y dónde va el bloque |
|------------|------------------|-------------------------------------|
| Papel térmico | `qr_heading` | `escpos::render_receipt`: rótulo en negrita y centrado, QR, «VERI\*FACTU», y **después** el nombre del negocio |
| Tique en pantalla | `qr_heading` de `ReceiptData` | `<ok-receipt>`: el bloque es lo primero de `.paper` (antes del título), rótulo a 11 px en negrita |
| Factura A4 | `qr_heading` de `InvoiceData` | `<ok-invoice>`: el bloque es lo primero de `.sheet`, centrado; el pie se queda con pago y notas |
| Papel del navegador | `qr_heading` | `receiptToPrintableHtml` de `sales`: el bloque `.fiscal-qr` es lo primero de `<body>` (antes de «duplicado» y del título), rótulo a 12 px en negrita |

- **No se traduce**: es el texto literal de la AEAT, igual que la leyenda (`QR_TRIBUTARIO_HEADING`
  en `sales ui/lib/document-mappers.ts`). El PDF de la factura del SaaS aplica el mismo criterio
  (saas#2184).
- **Clave propia y solo con el QR**, como `qr_legend`: sin `qr_data`/`qr` no se pinta el rótulo aunque
  el productor lo mande, y la cuenta (`prebill`) no lleva nada.
- **La posición no depende de la clave**: el renderizador sube el QR fiscal arriba aunque el productor
  sea viejo y no mande el rótulo. Todos los productores de `qr_data` (sales, invoice, el shell) mandan
  el QR fiscal, así que no hay un QR «no fiscal» que se suba por error.
- El **segundo QR** («pide tu factura», hub#963) y el QR **promocional** del tique se quedan al pie:
  el fiscal es el primero, como pide la especificación.
- El visor del módulo `invoice` pinta con `<ok-invoice>` y recibe la posición al actualizar
  outfitkit; su rótulo depende de que su productor mande `qr_heading` (invoice#84).

### El papel del navegador dibuja los QR (sales#340)

Sin impresora térmica (o sin rol asignado) el shell imprime `req.html` en un iframe aislado
(`toBrowser` de `apps/web/src/lib/print.ts`). Ese HTML lo escribe `receiptToPrintableHtml` de `sales`,
y hasta sales#340 no dibujaba ningún QR: anunciaba «escanea para validar» y «VERI\*FACTU» sin nada que
escanear, una factura simplificada sin el QR que exige el RD 1619/2012 (art. 6.5.a y 7.5).

- **El mismo codificador que la pantalla**: outfitkit exporta `qrSvgMarkup(value, { size })` desde
  `@erplora/outfitkit/ok-qr`, el trazado de `<ok-qr>` como cadena SVG autocontenida (negro sobre
  blanco, con zona de silencio). En el iframe no hay custom elements ni scripts, así que el QR viaja
  como marcado; pantalla y papel comparten la función que dibuja los módulos y no pueden discrepar.
- **Bloque fiscal arriba**: «QR tributario:», el QR (132 px = 35 mm, en medio del 30–40 mm que pide la AEAT, como el PDF del SaaS en saas#2185), «VERI\*FACTU» y la nota, como
  lo primero del papel. Sin `qr` no hay bloque, ni rótulo ni leyenda; una nota de CSV sin QR se sigue
  imprimiendo al pie, como antes.
- **El QR de «pide tu factura»** (hub#963) también se dibuja, más pequeño (90 px) y al pie, entre la
  leyenda y el localizador en texto; la URL en texto se queda debajo para teclearla.
- **El QR promocional** (reseñas, redes: `receipt_marketing_url` de los ajustes del TPV, sales#345)
  cierra el papel, detrás del bloque de «pide tu factura», con su nota (`promo_note`) encima y al
  70 % del fiscal (92 px), el mismo orden y tamaño que `renderPromo` de `<ok-receipt>`. Sin
  `promo_qr` no se pinta nada, aunque llegue la nota. La térmica también lo imprime desde hub#2009
  (sección siguiente).
- Orden de despliegue: outfitkit con `qrSvgMarkup` antes de rehornear `sales`, porque el `dist/` del
  módulo la empaqueta (el shell no interviene: es una función, no un custom element).

### La térmica imprime el QR promocional (hub#2009)

El mismo QR promocional del papel del navegador sale en el rollo ESC/POS.

- **Productor** (`sales ui/lib/print-document.ts`, `saleToPrintDocument`): copia `promo_qr`/`promo_note`
  de `saleToReceipt` solo si el negocio configuró la URL; sin ella las claves no existen y el
  documento no cambia. Lo usan el botón de imprimir del visor y la impresión automática al cobrar
  (`saleDocument` del shell → `printableDocument()`). La **factura completa**
  (`saleToInvoicePrintDocument`) las quita: es formal, como la A4.
- **Renderizador** (`crates/peripherals/src/escpos.rs`, `render_receipt`): solo con
  `Fiscal::Ticket` y `promo_qr`, pinta la nota encima y el QR con `qr_sized` a **3 puntos de
  módulo** (el fiscal y el de «pide tu factura» van a 4: el 75 %, el punto entero más cercano al 70 %
  de `<ok-receipt>`). Cierra el papel: después del agradecimiento y antes del corte. Una nota sin
  URL no imprime nada; una factura completa no lo imprime aunque le llegue.
- Orden de merge indiferente: un hub viejo ignora las claves nuevas y un `sales` viejo no las manda.

### Quién saca el papel

`apps/web/src/lib/print-host.ts` es el eslabón que faltaba: resuelve rol→impresora con la **misma**
función que la puerta global (`printerIdForRole`, una sola definición de «cuál es la impresora de
cocina») y entrega `documentType` + `document` a `erplora_print` tal cual.

**No tiene respaldo del navegador, y eso es el punto.** `erplora.print` cae al diálogo de impresión
cuando no hay impresora — correcto para un cajero que acaba de pulsar «imprimir», **incorrecto
aquí**: la comanda llega a una tablet apoyada en una estantería y un diálogo esperando a que alguien
pulse «Imprimir» es un tique que no sale mientras al hub se le dice que sí salió. Todo lo que va mal
se **lanza**: el drenaje lo convierte en `failed`, el hub devuelve el tique al siguiente host y
guarda el motivo en `last_error`.

`bootPrintHost` se arranca en el boot del shell **en todos los dispositivos**: al que no esté
registrado, el hub contesta `print.host_not_registered` y el drenaje **para** (es un hecho de
configuración, no meteorología), así que la PWA de un móvil cuesta un socket rechazado y nada más.

### La migración v25 y las filas que ya estaban

`print_queue_structured_document` añade `document_type` + `document` y **BORRA `html`**: lo que
guardaba es inservible por construcción —sin traductor, ningún host puede sacarlo por papel— y una
columna que el contrato declara muerta es sobre la que alguien vuelve a escribir dentro de seis
meses.

Los trabajos **no terminales** del formato viejo se **dejan muertos con su motivo**
(`print it again from the sale`). La alternativa —dejarlos `pending`— los haría reclamar por un host
que no puede renderizarlos, quemar sus cinco entregas y morir con un error que no explica nada. En la
práctica alcanza a **cero filas** (hoy ningún productor encola de verdad: `sdk.print` sigue siendo
hub#344), y por eso mismo es barato hacerlo bien. Un tique ya `done` **no se re-mata**.

**Re-ejecutable** (regla hub#342/#483): `ADD COLUMN IF NOT EXISTS`, `DROP COLUMN IF EXISTS` y el
`UPDATE` acotado a `document_type = ''` — condición que **ninguna fila nueva puede cumplir**, porque
toda fila nueva pasa por `enqueue`, que exige un tipo del vocabulario. Lo fija
`the_structured_document_migration_can_run_a_second_time`, que deja un tique legítimo esperando
durante el segundo pase y comprueba que sigue `pending`.

> **v25 y no un hueco.** Nació **v24** y se renumeró porque hub#362 reclamó la 24 estando esta sin
> mergear: el máximo del catálogo se re-comprueba **al rebasar**, no al empezar. Las v15 y v20 siguen
> libres e **inalcanzables** — «libre» no es «alcanzable», y cogerlas es el fallo mudo de siempre.
> Renumerar es gratis; **renombrar rompe** (los fixtures rebobinan por `name`).

### La prueba con papel de verdad

`crates/server/tests/print_to_real_printer.rs` recorre la definición de hecho entera contra una
térmica real de la LAN: encolar por `POST /api/print/jobs` → `hello`/`claim` por `GET /ws/print` →
`render_document` → bytes por el socket `:9100` → `done`, y el siguiente `claim` contesta `idle`.

```bash
ERPLORA_SMOKE_PRINT=1 cargo test -p erplora-server --test print_to_real_printer -- --ignored --nocapture
```

`#[ignore]` **y** la variable de entorno: el primero lo mantiene fuera del gate, la segunda impide que
un `--ignored` despistado deje tiques por el suelo. Imprime además **lo que debería poner el papel**,
para que quien lo tiene en la mano compruebe en vez de recordar. Lo que **no** cubre es la frontera
Tauri (el `invoke`): el test hace de host de impresión porque una app Tauri no es un `cargo test`.

---

## La PUERTA del shell: «entregado» tenía cuatro agujeros (hub#862)

> **P0 de QA, 2026-08-12.** App instalada (`com.erplora.app`) sobre un hub real, EPSON TM-T88III en
> `192.168.100.196:9100` **online** y descubierta como `ready`. El «Probar» del módulo `printing`
> —que llama a `erplora_test_print` **por ID**, sin pasar por la puerta— **sacaba papel**. Todo lo que
> pasaba por la puerta (etiqueta, tique al cobrar, cuenta previa) no sacaba nada. Y en tres ventanas
> del log del hub, **cero** `POST /api/print/jobs`: no es que la cola fallara, es que **nadie la
> llamaba**.

La puerta es `apps/web/src/lib/print.ts` (`erplora.print`), la única entrada para todos los módulos.
Los cuatro agujeros eran independientes y los cuatro acababan igual — en el navegador, que **dentro
de la app instalada no imprime nada** — o en nada:

**1 · El tique al cobrar NO pasaba por la puerta.** `apps/web/src/lib/print-on-sale.ts` resolvía él
mismo rol→impresora y llamaba a `peripherals.print` directo. Con la impresora **sin rol** —el estado
de toda impresora recién descubierta— no había `printerId`, los dos `if` salían en falso y la función
**volvía en silencio**: ni Bridge, ni cola, ni navegador, ni aviso, ni una línea en ningún log. Con
`getDevices()` lanzando (PWA en el navegador), un `catch { return; }` hacía lo mismo. Ahora el tique
sale por la puerta —con su `jobId: sale-<id>` y su documento estructurado, que desde hub#1921 es el que
compone el módulo `sales` (ver «El documento que viaja» §3)— y **lo que no se entrega se avisa**
(toast del shell). El cajón sigue pidiendo hardware aquí y ahora (un *kick* ESC/POS no se
puede encolar), pero ya **no arrastra al tique consigo**.

**2 · Faltaba `jobId` ⇒ no se encolaba.** `toQueue()` exigía la clave de idempotencia del caller y sin
ella caía al navegador. Es la única razón por la que el log no tenía ni un `POST`: convertía «el
caller no puso la clave» en «este documento no se imprime en ningún sitio», y sin dejar rastro. **Un
tique duplicado se tira a la basura; uno que no sale no existe** — así que ahora la puerta **acuña una
clave propia** cuando falta. Lo que se pierde es la deduplicación entre reintentos *del caller*, que
es justo lo que solo él puede aportar (`sale-42`, `prebill-o1-3`): traerla sigue siendo lo correcto.

> **Y con ello, la guarda que hace falta:** acuñar la clave dejaba encolable a quien solo trae `html`,
> cuyo documento estructurado es `{}`. La cola lo aceptaría —es un objeto JSON válido— y el
> renderizador, que lee POR CLAVE, cortaría **papel en blanco sin error**. Es el mismo agujero que la
> comprobación de forma de hub#501 cierra un paso más abajo, así que la puerta también lo cierra
> arriba: **sin documento estructurado no se encola**, se va al navegador, que es donde sirve un HTML.

**3 · `isThermalDocument` se dejaba fuera dos documentos que la cola SÍ acepta.** `barcode_label` y
`cash_session_report` están en `DOCUMENT_TYPES` y tienen renderizador, pero la puerta los trataba como
A4 y los mandaba al diálogo del navegador. La lista se enumera ahora **en positivo** y es la mitad
térmica del vocabulario de la cola (`invoice`/`delivery_note` quedan fuera a propósito: no hay rollo
de 80 mm que aguante un folio). Es la misma clase de fallo que `prebill` en hub#748 — **la tercera
lista se olvida porque no está en el mismo fichero que las otras dos.**

**4 · `via:'browser'` se contaba como ÉXITO dentro de la app instalada.** Es lo que hizo el fallo
**invisible durante toda la QA**: en el WebView `window.print()` no imprime, así que la puerta
devolvía «impreso por el navegador» sobre un papel que no existía. Dentro de la app instalada
(`isTauri()`) el respaldo del navegador **ya no se intenta y ya no es éxito**: se devuelve
`via:'none'` con el motivo, para que quien llama **pueda avisar**. `bridge` y `queue` son las dos
únicas vías que significan «entregado»; los callers ya escritos así (`print-comanda`, la cuenta del
TPV) empiezan a decir la verdad sin tocar una línea.

### La A4 en la app de ESCRITORIO: el diálogo del sistema (hub#2006)

El punto 4 dejó la A4 sin salida dentro de la app: `via:'none'`, o —con una térmica con rol— la
factura **en rollo**. Desde hub#2006 la puerta tiene un escalón más, **antes** del Bridge y de la cola:

> documento con `format:'a4'` **y** su `html` + app instalada + alguien delante
> (`fallbackToBrowser` ≠ `false`) → comando del shell **`print_document`** → diálogo de impresión
> **del sistema** (impresora láser o «Guardar como PDF»).

- Contesta **`via:'browser'`**: «entregado a un diálogo de impresión», lo mismo que el navegador con
  una A4 y lo que `sales` ya trata como papel entregado (sales#306). **No** se añade un valor nuevo a
  `via`: el contrato de `PrintResult` no cambia de forma, solo gana un camino que lo cumple.
- En **Android** el mismo comando abre la pantalla de impresión del sistema (`PrintManager`, con
  «Guardar como PDF») por el plugin del shell (hub#2008); la puerta web no cambia.
- Si el shell **no** puede (iOS, o una app antigua sin el comando), la puerta sigue
  **la ruta de siempre** (Bridge → cola → `none`) y, si acaba en `none`, el motivo del shell va en
  `error`. Cero regresiones: donde no hay diálogo, pasa exactamente lo de antes.
- Un térmico (`receipt`, comandas…), una A4 **sin** `html` o la impresión desatendida **nunca**
  abren el diálogo. El tique al cobrar (`print-on-sale`) no manda `html` ni `a4`: no cambia.
- No se genera PDF propio: Tauri 2 ya imprime una webview (`Webview::print`). El cómo, en
  [apps/tauri.md](apps/tauri.md) § «Imprimir una A4 con el diálogo del sistema».

### `queueOnly`: la cola y nada más (hub#2029)

Un documento que **todas** las cajas abiertas reciben a la vez —la comanda de un pedido que no
disparó ninguna caja: API, flujo, pedido online— no puede ir por la impresora de cada una: salía
una hoja por caja. Con `queueOnly: true` la puerta se salta el diálogo A4 y el Bridge y va **directa
a la cola**; todas piden el mismo `jobId`, `ON CONFLICT (hub_id, job_id) DO NOTHING` guarda uno y lo
imprime quien drena la estación. Si la cola no lo acepta, la respuesta lo dice (`via:'none'` en un
documento desatendido), nunca cae a la impresora de al lado. Hoy solo lo usa `print-comanda.ts`.

Como la cola pasa a ser el único camino hasta el equipo con la impresora cuando ese equipo no disparó
el pedido, el **alta de host** cuenta las mismas impresoras que la puerta global: `printerRolesOfDevices`
y `printerIdForRole` comparten `isReachablePrinter` (dirección de red, o Bluetooth con MAC). Una
impresora a la que la puerta puede imprimir pero que el alta no reclama sería una estación que nadie
drena.

### Una impresora nace SIN ROL, y eso apagaba las dos mitades

El descubrimiento registra la impresora, pero el **rol** lo pone alguien a mano en el módulo
`printing`. Hasta que lo haga se suman **dos silencios** que dan el mismo síntoma («está online, el
interruptor está puesto y no pasa nada»):

1. el rol `receipt` no resuelve a ninguna impresora → no sale por hardware;
2. `printerRolesOfDevices` no cuenta una caja sin rol, así que este equipo se da de alta como host de
   **cero roles** → nadie drena la cola → lo encolado tampoco sale.

Red de seguridad en el boot del host (`print-host.ts`): si hay **exactamente una** impresora
alcanzable y **ningún** rol asignado en todo el registro, se le da `receipt` antes del alta — el caso
mayoritario del TPV, una impresora sola es la de tiques. Con **dos** sin rol no se adivina (¿cuál es
la etiquetadora?) y con alguna ya rolada la instalación está configurada: **manos fuera**. El aviso
para los casos ambiguos sigue pendiente, y es de la pantalla de `printing`.

> **Lo que esto enseña del diseño, más allá del bug:** la puerta tenía **tres formas de decir que sí**
> y solo dos eran entregas. Un resultado que mezcla «salió por la impresora», «lo tiene la cola» y
> «abrí un diálogo» en el mismo `via` obliga a **cada caller** a saber cuál de las tres cuenta — y el
> día que uno se equivoca, el fallo no se ve. La regla que queda: **quien no puede entregar, lo dice**.

## Lo que esto NO es (todavía)

Con hub#341, hub#342, hub#343, **hub#501** y **hub#344** la cadena de impresión está **cerrada
4/4** (tique → host → papel → productor, `sdk.print` encola cuando no hay shell, PR hub#602) y
probada con papel de verdad (hub#1593 corrigió esta sección, que seguía dando la 4/4 por
pendiente). Lo que queda es la **pantalla** y sus vecinas — el core ya sirve los datos:

| Issue | Qué falta |
|-------|-----------|
| hub#344 | ✅ **`sdk.print` encola** (PR hub#602; los agujeros que quedaban, en hub#862 arriba). Lo que sigue faltando es la **pantalla de `coverage`** («no hay nadie imprimiendo lo de cocina») con su cadena en inglés y su `es` — y con ella el aviso de «hay una impresora sin rol». |
| ~~[hub#457](https://github.com/ERPlora/hub/issues/457)~~ | ✅ **Hecho** (v47): las estaciones son **filas con id** y las tres puertas resuelven en vez de comparar — ver [la sección de arriba](#las-estaciones-son-filas-no-cadenas-migración-v47-hub457). Un `kitchn` ya no abre una cola: es un `422` que nombra las reales. |
| [hub#987](https://github.com/ERPlora/hub/issues/987) | Lo que la decisión de mercado dejó fuera de hub#457: el mapa **`documentType` → estación** (para que un módulo deje de nombrar el hardware del comerciante), **fallar ABIERTO** y **avisar de lo no drenado**. |
| [hub#502](https://github.com/ERPlora/hub/issues/502) | El *reset* vacía `_print_queue` (ver abajo). |
| ~~[hub#1107](https://github.com/ERPlora/hub/issues/1107)~~ | ✅ **Hecho**: `hub.print.coverage` y `hub.print.jobs` — la cola es legible por un módulo. |
| ~~[hub#1108](https://github.com/ERPlora/hub/issues/1108)~~ | ✅ **Hecho**: `requeue` y `discard`, con su puerta admin + `printer` y su superficie SDK. |
| ~~[hub#1532](https://github.com/ERPlora/hub/issues/1532)~~ | ✅ **Hecho** (v58): los dos sellos nombran **el módulo que actuó**, no solo la persona cuya sesión estaba abierta — y el reintento pasa a sellar, que antes no sellaba nada. |
| ~~[hub#1565](https://github.com/ERPlora/hub/issues/1565)~~ | ✅ **Hecho**: el sello se **lee** por las dos puertas del listado, solo para `hub.administer`, con el **nombre** de la persona y los buckets cerrados del más reciente al más antiguo ([ADR-0448](../00-overview/decision-log.md#adr-0448)). La pantalla, en [printing#36](https://github.com/ERPlora/printing/pull/36). |
| [printing#28](https://github.com/ERPlora/printing/issues/28) | **La pantalla.** El core ya sirve los cuatro datos y los dos gestos; pintarlos es del módulo `printing`, y hasta que lo haga la cola sigue sin verse en mitad de un servicio. |
| ~~[hub#525](https://github.com/ERPlora/hub/issues/525)~~ | ✅ **Hecho en el módulo** (sales#347): la lista de Ventas lleva «Reimprimir» en cada fila, que regenera el tique desde la venta con el visor oculto del módulo y lo manda como duplicado; abrirlo ya estaba en la fila. El runtime genérico no sabe nada de ventas, como pedía la fila. |

La **pantalla** que pinta `coverage` para el dueño va con hub#344: el hub ya sabe la respuesta y ya
hay quien drene y saque el papel, así que el aviso ya apuntaría a algo que funciona — lo que le falta
es el productor que llene la cola en el día a día.

🪦 El WS `:12321` del Bridge standalone **ya no existe**: hub#339 retiró `WsBridgeTransport` del
SDK y hub#340 **borró `apps/bridge` del árbol** el 2026-08-08 (verificado —
`git ls-tree origin/develop apps/` devuelve solo `tauri` y `web`). Desde entonces la única forma de
sacar papel es la app instalada drenando esta cola.

> **El *reset*, decidido (hub#343).** El worker de hub#342 dejó la pregunta abierta y la respuesta
> es que las dos tablas **no son la misma clase de cosa**:
>
> - **`_print_queue` es DATO** — tiques de ventas que el reset acaba de borrar. Ahora que existe
>   quien drene, dejarlos ahí significa imprimir fantasmas de un negocio que ya no existe. **Debe
>   vaciarse.**
> - **`_print_host` es CONFIGURACIÓN DEL LOCAL** — qué equipo imprime lo de cocina. Borrarlo
>   obligaría a re-emparejar las impresoras después de cada reset, justo lo que hub#342 evitó a
>   propósito al hacer que el registro sobreviva a un reinicio del runtime. **No se toca.**
>
> Implementar el vaciado abre una **sección nueva** en el contrato de `ResetSelection` (plan, informe,
> UI y su i18n), así que no entra de rebote en el drenaje: [hub#502](https://github.com/ERPlora/hub/issues/502).
