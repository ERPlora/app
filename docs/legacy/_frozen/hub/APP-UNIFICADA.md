> Copia histórica de `erplora-old/architecture/_frozen/hub/APP-UNIFICADA.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../../README.md).

# App unificada — una sola app, tres formas de empaquetar (Opción B)

> **Estado:** decisión tomada por el humano (2026-06-17). Diseño = columna del humano;
> esta redacción = columna IA. Decisión registrada como **ADR-0050** en
> [`architecture/00-overview/decision-log.md`](architecture/00-overview/decision-log.md).
> Tareas de implementación en [`TODO.md`](TODO.md) (item #1).

## 0. La idea en una frase

**El front, el back (runtime Axum) y el bridge son las mismas piezas en todos lados.** Lo único
que cambia entre entornos es **cómo se empaquetan y cómo el front llega al back/bridge** — siempre
por **HTTP (+WS para eventos)**, nunca por IPC de Tauri en el camino de datos. El IPC de datos de
Tauri **desaparece**.

## 1. Por qué (el problema que resuelve)

Hoy Tauri usa un camino híbrido para los datos: `webview → invoke (IPC) → handler Rust → reenvía
por HTTP a 127.0.0.1:8787 → Axum` (ver [hub/apps/tauri/src-tauri/src/lib.rs:238](hub/apps/tauri/src-tauri/src/lib.rs#L238)).
Es **lo peor de dos mundos**: el puerto loopback ya está abierto (pagas su superficie) **y** encima
hay una capa IPC que obliga a una rama `IpcTransport` en el frontend. Eso impide la meta de "una
sola app".

La salida limpia (**Opción B**): el front habla **HTTP+WS directo** al runtime en todos los
entornos. En Tauri el runtime es un servidor loopback embebido; en cloud es ECS. El frontend es
idéntico byte a byte.

## 2. Las tres formas de distribuir la MISMA app

| Forma | Empaquetado | Front → Back | Bridge (hardware) |
|---|---|---|---|
| **Cloud (hub/aws)** | imagen Docker en ECS (back + front servidos) | HTTP+WS → ECS (Aurora) | bridge **standalone** instalado aparte (solo si el cliente tiene hardware) |
| **Tauri desktop** | un binario: front + back + bridge embebidos | HTTP+WS → `127.0.0.1:8787` loopback | bridge **embebido** en el shell (mismo crate `erplora-peripherals`) |
| **Tauri Android** | APK: front + back + bridge embebidos | HTTP+WS → `127.0.0.1:8787` loopback | bridge **embebido** en el shell |

Reglas de eje (de [hub/CLAUDE.md](hub/CLAUDE.md) §core architecture, sin cambios):
`single ⟹ Tauri` (forzado) · `web-pwa ⟹ cloud` (forzado) · `Tauri ⇏ single` (puede ser cliente cloud).

## 3. El bridge: mismo canal en PWA y en Tauri

**Clave para "una sola app": el front habla con el bridge por el MISMO canal en los dos casos**
(no `invoke`, porque en PWA no existe). Opciones de canal, ambas válidas:

- **HTTP API en `127.0.0.1:PUERTO`** (recomendado por simplicidad; el front consulta estado por
  polling). No necesita WebSocket.
- **WebSocket** (`ws://localhost:12321`, lo que ya hay en [hub/apps/bridge/src/main.rs](hub/apps/bridge/src/main.rs)):
  añade eventos push (impresión terminada, impresora caída/recuperada) sin polling.

En **Tauri** el shell arranca **el mismo servidor del bridge** dentro del proceso (reusa
`erplora-peripherals`), en vez de exponer handlers `invoke`. En **PWA** el bridge es un proceso
aparte instalado. El frontend conecta a `localhost:PUERTO` en ambos → código idéntico.

> Consecuencia: **se eliminan los handlers `invoke` de hardware** (`erplora_print`,
> `erplora_discover_printers`, etc. en [hub/apps/tauri/src-tauri/src/lib.rs:551+](hub/apps/tauri/src-tauri/src/lib.rs#L551))
> y los de datos (`erplora_query`/`erplora_command`). En Tauri solo quedan, si acaso, comandos
> `invoke` para cosas genuinamente nativas sin equivalente HTTP (keychain, device_id, ciclo de vida).

### PWA "despierta" al bridge local
Una web **no puede lanzar un ejecutable arbitrario**. Pero **sí** puede abrir una app instalada por
**protocolo propio / deep link** (`erplora-bridge://…`) que el bridge registra en el SO (como
Zoom/Spotify "abrir en la app"). Patrón: el bridge se instala con auto-arranque y escucha en
localhost; la PWA conecta directo; si está caído, usa el deep link para levantarlo (fallback, no
flujo normal).

## 4. Viabilidad por pieza (veredicto)

- **Cloud front↔back por HTTP** → ✅ ya es así, cero riesgo.
- **Tauri todo embebido (desktop)** → ✅ viable, estándar y probado.
- **Tauri todo embebido (Android)** → ✅ viable con la config de §5.
- **PWA + bridge aparte (desktop)** → ✅ viable; navegadores permiten `http(s)`/`ws` a localhost
  desde páginas HTTPS (localhost = "secure context").
- **PWA + bridge aparte (móvil)** → ⚠️ combo flojo: Android necesita *foreground service*; iOS
  Safari restringe localhost. **Recomendación: en móvil distribuir Tauri** (todo embebido), y dejar
  el bridge-aparte para PWA en **desktop**.

## 5. Gotchas de Android (los 4 a no olvidar)

1. **Cleartext a localhost bloqueado por defecto** (API 28+). Hay que añadir un
   `network_security_config.xml` que permita cleartext a `127.0.0.1`/`localhost`, cableado en el
   `AndroidManifest` que genera Tauri. Sin esto, el HTTP loopback (datos) y el del bridge fallan
   **en silencio** en el webview.
2. **mDNS necesita `MulticastLock`** (+ permiso `CHANGE_WIFI_MULTICAST_STATE`) para descubrir
   impresoras. El fallback de escaneo de subred ya existe en [hub/crates/peripherals/src/discovery.rs](hub/crates/peripherals/src/discovery.rs)
   y funciona sin eso (más lento).
3. **Permiso `INTERNET` + misma WiFi** que las impresoras para abrir sockets TCP a la LAN.
4. **Backend `single` (SQLite embebido) en Android** corre Axum+SQLite en el móvil (aplica #1). Con
   backend `cloud` los datos van a HTTPS (ECS) sin problema de cleartext.

## 6. Seguridad — esto AÚN NO está y es core del humano

Con un puerto loopback abierto (datos y/o bridge), **cualquier web que el usuario visite puede
conectar a `localhost` y mandar a imprimir / leer datos** (clase de ataque conocida: CVEs de
bridges/impresoras en localhost). El token de app cubre el camino de datos, pero:

- **El bridge NO valida origen ni token hoy** ([hub/apps/bridge/src/main.rs:139](hub/apps/bridge/src/main.rs#L139),
  `ws_upgrade` acepta cualquier conexión). Hay que añadir **validación de `Origin` + token de
  sesión** en el handshake, en los dos modos (embebido y standalone).
- **El puerto de datos** debe seguir exigiendo el token de sesión (`X-Hub-Session`) ya existente, y
  validar `Origin`/`Host` para cortar CSRF/DNS-rebinding desde el navegador.

> Esto es **seguridad de comandos / core → columna del humano**. La IA puede redactar tests y
> boilerplate del validador una vez el humano defina el contrato.

## 7. Qué implementar (resumen; detalle en TODO.md item #1)

1. **Frontend:** un solo transporte HTTP+WS; eliminar la rama `IpcTransport` del SDK
   (`hub/packages/module-sdk`) y del `bridge-client.ts`. Un solo `bridge-client` que apunta a
   `localhost:PUERTO`.
2. **Tauri shell:** arrancar el bridge embebido (servidor, reusando `erplora-peripherals`) en vez
   de los handlers `invoke` de hardware; quitar `erplora_query`/`erplora_command` (el front va
   directo al loopback). Mantener solo `invoke` nativo imprescindible.
3. **Bridge:** endurecer auth (Origin + token) en `apps/bridge` y en el modo embebido.
4. **Android:** `network_security_config.xml` (cleartext localhost) + `MulticastLock` + permisos.
5. **Deep link:** registrar esquema `erplora-bridge://` para que la PWA despierte el bridge.
6. **Docs:** sincronizar `architecture/hub/apps/*` + `auth.md` con esta decisión (ADR-0050).

## 8. Decisiones aún abiertas (preguntar, no decidir — columna del humano)

1. **Canal del bridge:** ¿HTTP API (polling) o WebSocket (push)? (§3 — recomendado empezar simple
   con HTTP, subir a WS solo si los eventos push lo justifican).
2. **Puerto y descubrimiento** del bridge embebido vs standalone (mismo puerto fijo vs negociado).
3. **Forma exacta del token del bridge** (reusar `X-Hub-Session` vs token propio de bridge).
4. **¿Backend `single` en Android** entra en el alcance inicial o solo `cloud + Tauri` primero?
