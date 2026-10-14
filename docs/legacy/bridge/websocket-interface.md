> Copia histórica de `erplora-old/architecture/bridge/websocket-interface.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

> 🪦 **HISTORIA — este WebSocket ya no existe.** ADR-0196 retiró el bridge standalone y hub#340 lo
> borró del árbol (2026-08-08): no hay servidor en `:12321`, ni handshake, ni token de
> emparejamiento, ni `hub/apps/bridge/src/{main,auth}.rs`. Lo que **sí sobrevive** es el
> **vocabulario** —comandos, `printer_id`, tipos de documento, códigos de error—, que hoy viaja por
> los `invoke` in-process del shell Tauri. Fuente viva:
> `hub/crates/peripherals/src/protocol.rs` + [`../hub/apps/tauri.md`](../hub/apps/tauri.md).
>
> Cargar este doc solo para entender de dónde viene ese vocabulario; **no** para diseñar nada nuevo.

Este doc especifica el **contrato JSON Hub↔Bridge sobre WebSocket local** (puerto 12321): los
comandos que el Hub envía y los eventos que el Bridge responde. Es la capa de transporte que se monta
sobre la lógica de hardware ([hardware-protocol.md](hardware-protocol.md)); la implementación Android
es un espejo Kotlin del mismo formato ([android-app.md](android-app.md)). **Fuente única de verdad**:
`hub/crates/peripherals/src/protocol.rs`. `hub/static/js/bridge.js` consume este formato sin cambios.

---

## Endpoints del servidor local

El servidor escucha en el puerto **12321** (`BRIDGE_WS_PORT`). En `apps/bridge` (Axum) bindea
`127.0.0.1:12321`; la app Android bindea `0.0.0.0:12321` (Chrome en Android usa a veces otra ruta de
loopback).

| Método | Ruta | Propósito |
|--------|------|-----------|
| `GET` | `/status` | Health check — el navegador detecta el Bridge con timeout corto (~500ms) |
| `GET` (upgrade) | `/ws` | WebSocket: comandos (`Command`) y eventos (`Event`) en JSON — **gateado**, ver abajo |

### Autenticación del handshake `/ws` (ADR-0050, `hub/apps/bridge/src/auth.rs`)

`GET /ws` **no** es un upgrade sin credencial: el handshake pasa por `BridgeAuth` con dos barreras,
**fail-closed** salvo modo dev explícito:

1. **Allowlist de `Origin`** — solo orígenes de confianza (loopback, `tauri://`, el dominio del
   tenant vía `BRIDGE_ALLOWED_ORIGINS`). Fuera de la lista → **403** y no abre el WS.
2. **Token de sesión compartido** (`BRIDGE_TOKEN`, inyectado en ECS/empaquetado o generado
   aleatorio en el primer arranque y persistido en `BRIDGE_TOKEN_FILE`) enviado como
   `Authorization: Bearer`, `X-Hub-Session` o `?token=`. Token ausente o incorrecto → **401** y no
   abre el WS.

`BRIDGE_DEV=1` desactiva solo la barrera de token (queda la allowlist de `Origin`) — **nunca en
producción**; sin él, `from_env` siempre resuelve un token (fail-closed por defecto). Verificado con
tests de `main.rs` (`ws_upgrade_rejected_without_token`→401, `ws_upgrade_rejected_with_bad_token`→401,
`ws_upgrade_rejected_with_forbidden_origin`→403, `ws_upgrade_rejected_with_rebinding_host`→403).

### Contrato de `GET /status` (bridge#10)

El **contrato mínimo común** que consume el Hub (`apps/web/src/lib/bridge-client.ts`,
`detectBridge()`) es:

```jsonc
{ "ok": true, "version": "<semver>" }
```

Ambas líneas **DEBEN** incluir esas dos claves con esos tipos (`ok: bool`, `version: string`); el
resto de claves del body son **informativas** y pueden variar por plataforma:

- **Rust** (`apps/bridge`): `{ "ok": true, "version", "service": "erplora-bridge", "devices": <n>,
  "watchdog": true }`.
- **Android** (`BridgeServer.kt`): `{ "status": "ok", "version", "printers": <n>, "scanner": false,
  "devices": <n>, "watchdog": <bool> }` — **todavía sin la clave `ok`**, por lo que
  `detectBridge()` lo reporta offline; pendiente en el repo Android
  (ERPlora-Bridge-android, issue de paridad de `/status`).

Al abrir el WS, el Bridge emite un evento `status` inicial (con `scanner: false` en red-only).

---

## Comandos (Hub → Bridge)

Discriminados por la clave `action` (Rust: enum `Command`, `#[serde(tag = "action")]`).

| `action` | Campos | Efecto |
|----------|--------|--------|
| `get_status` | — | Versión del Bridge + impresoras cacheadas |
| `discover_printers` | — | Re-escanea la red; responde `printers` — o `error` con `code: "local_network_permission_denied"` si el SO no dejó mirar (ADR-0235: una lista vacía diría lo contrario) |
| `print` | `printer_id`, `document_type`, `data`, `job_id?` | Renderiza + imprime un documento |
| `open_drawer` | `printer_id`, `pin` (default 2) | Kick ESC/POS del cajón por el socket de la impresora |
| `test_print` | `printer_id` | Página de prueba |
| `send_notification` | `title`, `body` | Notificación a nivel de SO (sin evento de ACK). En `apps/bridge` vía `notify-rust` (Linux DBus / macOS / Windows Toast) en `spawn_blocking`; si la plataforma no la soporta (headless, sin DBus) degrada a log sin panic (bridge#9). En el **sidecar Tauri** esta capacidad la cubre el **shell** (plugin de notificaciones de Tauri), no el crate. En **Android** sigue siendo solo log (pendiente allí) |
| `get_devices` | — | Contenido del registro de dispositivos |
| `set_device_role` | `mac`, `role` (`receipt`\|`kitchen`\|`bar`\|`label`) | Asigna rol; responde `devices` |
| `set_device_name` | `mac`, `name` | Renombra; responde `devices` |
| `remove_device` | `mac` | Elimina del registro; responde `devices` |

---

## Eventos (Bridge → Hub)

Discriminados por la clave `event` (Rust: enum `Event`, `#[serde(tag = "event")]`).

| `event` | Campos | Cuándo |
|---------|--------|--------|
| `status` | `version`, `printers[]`, `scanner` | Al conectar y ante `get_status` |
| `printers` | `printers[]` | Tras `discover_printers` |
| `devices` | `devices[]` | Tras `get_devices` / mutaciones de registro |
| `device_recovered` | `device` | El watchdog re-localiza un dispositivo offline |
| `device_lost` | `device` | El watchdog detecta un dispositivo caído |
| `print_complete` | `job_id?` | Impresión OK (asíncrono, tras procesar la cola) |
| `print_error` | `job_id?`, `error` | Impresión fallida (printer_id/payload inválidos, o reintentos agotados) |
| `drawer_opened` | `printer_id` | Cajón abierto |
| `error` | `message`, `code` | Comando inválido / error de periférico |

Códigos de `error` emitidos por `apps/bridge`: `bad_command` (JSON no parseable a `Command`),
`peripheral_error` (fallo de la capa de hardware).

### Fan-out de eventos asíncronos (bridge#8)

En `apps/bridge`, `print`/`test_print` **encolan** el trabajo en la `PrintQueue` (reintentos:
3 intentos, backoff 2s) y no responden inline: el `AppState` tiene un bus
`tokio::sync::broadcast<Event>` al que se puentean los `JobOutcome` del worker de la cola y los
`WatchdogEvent` del watchdog. Cada conexión WS se suscribe al bus (`tokio::select!` entre comandos
entrantes y bus), de modo que `print_complete` / `print_error` / `device_recovered` /
`device_lost` llegan a **todas** las conexiones abiertas, no solo a la que envió el comando. Un
cliente retrasado (`RecvError::Lagged`) pierde los eventos más antiguos pero la conexión sigue.

---

## Estructuras de datos

```jsonc
// PrinterInfo
{ "id": "network:192.168.1.50:9100", "name": "Network Printer (192.168.1.50)",
  "type": "network", "status": "ready", "paper_width": 80, "mac": "AA:BB:CC:DD:EE:FF" }

// Device (registro persistente)
{ "mac": "AA:BB:CC:DD:EE:FF", "ip": "192.168.1.50", "port": 9100, "name": "...",
  "role": "receipt", "type": "network",
  "first_seen": "2026-06-09T10:00:00", "last_seen": "...", "status": "online" }
```

- `PrinterInfo.type` y `Device.type` son siempre `"network"` en red-only (Rust). El espejo Android
  añade `"bluetooth"` (capacidad nativa).
- `status` de impresora: `ready | busy | error | offline`. `status` de dispositivo: `online | offline`.
- `paper_width` en serde es snake_case (espejo del Bridge Python).

---

## Diferencias por la decisión red-only (§2.7)

Respecto al protocolo Python original, la línea **Rust elimina**:

- la acción `toggle_keyboard` y el evento `keyboard_toggled` (teclado virtual),
- el evento `barcode` (el escáner HID lo gestiona el SO/navegador como teclado),
- los esquemas `usb:` y `bluetooth:` de `printer_id` (queda solo `network:{ip}:{port}`).

El campo `scanner` del evento `status` se conserva siempre `false` por compatibilidad con `bridge.js`.

> La app **Android** todavía declara `toggle_keyboard`, `barcode` y `keyboard_toggled` en su
> `Protocol.kt` (más `bluetooth:`), por capacidad de la plataforma. Es la divergencia conocida entre
> ambas implementaciones — ver [android-app.md](android-app.md).

---

## Checklist de paridad Rust ↔ Android (bridge#10, verificada 2026-06-11)

Verificación clave-a-clave de los comandos/eventos **compartidos** entre `protocol.rs` +
`apps/bridge/main.rs` (Rust) y `Protocol.kt` + `BridgeServer.kt` (Android). Revalidar al tocar
cualquiera de los cuatro ficheros.

### Comandos compartidos

| `action` | Claves | Paridad | Matices |
|----------|--------|---------|---------|
| `get_status` | — | ✅ | — |
| `discover_printers` | — | ✅ | — |
| `print` | `printer_id`, `document_type`, `data`, `job_id?` | ✅ | Android defaultea `document_type:"receipt"` y **genera** `job_id` (UUID) si falta; Rust exige `document_type` y deja `job_id: null` si falta |
| `open_drawer` | `printer_id`, `pin` | ⚠️ | Android **ignora `pin`** del mensaje (siempre kick pin 2, aunque `CashDrawer.open` soporta 2/5); Rust lo respeta (default 2) |
| `test_print` | `printer_id` | ✅ | — |
| `send_notification` | `title`, `body` | ⚠️ | Rust muestra notificación de SO (bridge#9); Android solo loggea (pendiente allí). Ninguno responde evento |
| `get_devices` | — | ✅ | — |
| `set_device_role` | `mac`, `role` | ✅ | Android admite `role: null` (lo persiste como `JsonNull`); Rust espera string |
| `set_device_name` | `mac`, `name` | ✅ | — |
| `remove_device` | `mac` | ✅ | — |

### Eventos compartidos

| `event` | Claves | Paridad | Matices |
|---------|--------|---------|---------|
| `status` | `version`, `printers[]`, `scanner` | ✅ | `scanner` siempre `false` en Rust |
| `printers` | `printers[]` | ✅ | `PrinterInfo` idéntico (`id/name/type/status/paper_width/mac?`); Android añade `type:"bluetooth"` como valor extra |
| `devices` | `devices[]` | ✅ | `Device` idéntico; con rol sin asignar Rust **omite** `role` (`skip_serializing_if`) y Android emite `role: null` — `bridge.js` trata ambos como ausente |
| `device_recovered` / `device_lost` | `device` | ✅ | — |
| `print_complete` | `job_id` | ✅ | Rust puede emitir `job_id: null` (print sin `job_id`); Android siempre string (lo genera) |
| `print_error` | `job_id`, `error` | ✅ | Ídem `job_id` |
| `drawer_opened` | `printer_id` | ✅ | — |
| `error` | `message`, `code` | ✅ | Mismas claves; **valores** de `code` distintos por línea: Rust `bad_command`/`peripheral_error`, Android `parse_error`/`unknown_action`/`missing_param`/`drawer_error` — el Hub no debe hacer matching exacto de `code` compartido |

### Divergencias intencionales (NO «corregir»)

- Android **conserva** `toggle_keyboard`, evento `keyboard_toggled`, evento `barcode` y el esquema
  `bluetooth:{address}` de `printer_id` — capacidad nativa de plataforma, eliminados de la línea
  Rust por la decisión red-only (§2.7).
- Android bindea `0.0.0.0:12321` (Chrome Android) vs `127.0.0.1` en Rust.
- Codificación de texto ESC/POS: ISO-8859-1 (Android) vs CP437 (Rust).

---

## Ejemplo de sesión

```mermaid
sequenceDiagram
    participant Hub as Hub (bridge.js)
    participant Br as Bridge (WS :12321)

    Hub->>Br: GET /status (¿hay bridge?)
    Br-->>Hub: 200 { ok: true, version }
    Hub->>Br: WS upgrade /ws (Origin + Authorization/X-Hub-Session/?token=)
    Note over Hub,Br: Origin fuera de allowlist → 403; token ausente/incorrecto → 401 (ADR-0050)
    Br-->>Hub: 101 Switching Protocols
    Br-->>Hub: { event:"status", version, printers:[], scanner:false }
    Hub->>Br: { action:"discover_printers" }
    Br-->>Hub: { event:"printers", printers:[ {id:"network:…:9100", …} ] }
    Hub->>Br: { action:"print", printer_id, document_type:"receipt", data:{…}, job_id }
    Br-->>Hub: { event:"print_complete", job_id }
    Hub->>Br: { action:"open_drawer", printer_id, pin:2 }
    Br-->>Hub: { event:"drawer_opened", printer_id }
```
