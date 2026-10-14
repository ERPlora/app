> Copia histórica de `erplora-old/architecture/hub/apps/tauri.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../../README.md).

# Hub · `apps/tauri` — Shell cliente fino de escritorio (Bridge-webview, ADR-0159)

> ⭐ **ADR-0196 (2026-08-03) — este shell es AHORA el único camino al hardware.** Supersede
> [ADR-0159](../../adr/ADR-0159.md) §2 y §3: mueren la app Kotlin y `apps/bridge`
> (WS `:12321`), y en el SDK queda `isTauri()` → `invoke` in-process, sin `WsBridgeTransport`.
> Se añaden Android e iOS a las plataformas, cae el *foreground service*, y la **cola de impresión
> pasa a vivir en el Hub** (cualquier dispositivo encola; el que tiene la app y ve la impresora
> drena).
>
> **As-built (2026-08-08, hub#339): en el SDK ya está hecho.** `WsBridgeTransport` no existe;
> `makeBridgeTransport()` es `isTauri() ? IpcBridgeTransport : UnavailableBridgeTransport`, y este
> último contesta `{online:false}` a `detect()` y rechaza con `hardware_unavailable` todo lo demás.
> O sea: **este shell no es «el camino preferente» al hardware, es el único que hay** — sin la app
> instalada no se imprime un tique térmico, que es el precio explícito de ADR-0196. La retirada de
> `apps/bridge`, el pairing y las credenciales de máquina sigue **pendiente (hub#340)** — comprueba
> con `ls hub/apps/bridge/`.

> Cargar este doc antes de editar `hub/apps/tauri/`.

> ⭐ **[ADR-0159](../../adr/ADR-0159.md) (2026-07-26) — el shell vuelve como CLIENTE FINO.**
> La retirada de [ADR-0154](../../adr/ADR-0154.md) (2026-07-25) eliminó el **producto Hub
> Local** (runtime embebido + SQLite + offline) — eso **sigue muerto**. Lo que 0159 recupera es solo el
> **empaquetado**: una app de escritorio (Win/macOS/Linux) cuya **webview carga el onboarding del SaaS
> y después la PWA del hub cloud** (`hub_url`), con el **hardware local por `invoke`** sobre
> `crates/peripherals`. Sin runtime local, sin BD local, sin entitlement gate.
>
> **Estado: IMPLEMENTADO** (hub#225, mergeado 2026-07-26): `apps/tauri` resucitado del historial y
> adelgazado a cliente fino (`frontendDist: ../shell-dist`, captura `?shell=1`, `forget_hub`); e2e
> verificado en Mac (07-27). El as-built pre-0154 vive en el historial git.

> ⭐ **[ADR-0160](../../adr/ADR-0160.md) (2026-07-29) — identidad única `com.erplora.app`.**
> El `identifier` de Tauri pasa de `com.erplora.hub` a **`com.erplora.app`** (una sola app instalable
> por plataforma; en Play muere también `com.erplora.bridge`). El **Tauri-Android se retira** de
> `tauri-release.yml` — Android vive solo en la app Kotlin (`ERPlora-Bridge-android`). Escritorio:
> **Win+Linux por CI, macOS solo build local**. Serie de versiones **1.0.x**, release por tag `v*`.
> ⚠️ **Enmendado por [ADR-0180](../../adr/ADR-0180.md) (2026-08-01):** Android vuelve a
> `tauri-release.yml` como la MISMA app, y macOS entra en CI. El `versionCode` lo fija el Gradle con
> la fórmula de este ADR, no el que calcula Tauri (en Play es puerta de un solo sentido).
> Cambiar el identifier cambia el directorio de datos (pre-lanzamiento: sin migración).

## Qué es (modelo 0159)

Una sola ventana Tauri v2 que:

1. **Primer arranque** → `WebviewUrl::External(<onboarding del SaaS>)`: registro/login (el hub free
   se auto-crea al registrarse, señales allauth → `provision_free_hub`) y elegir/abrir el hub.
2. **Captura de `hub_url`** → cuando la navegación llega a un origen **de hub** llevando el marcador
   `?shell=1`, el shell persiste ese ORIGEN como `hub.url` y entra en «modo app». Qué cuenta como
   «de hub» no lo decide el marcador: ver
   «[La captura de `hub_url`](#la-captura-de-hub_url-quién-puede-pedir-ser-recordado-adr-0243-hub335)».
3. **Arranques siguientes** → carga `hub_url` directo. La sesión de la PWA (`X-Hub-Session`,
   localStorage de la webview) persiste → sin re-login hasta caducar.
4. **Hardware** → la PWA detecta `window.__TAURI__` (`withGlobalTauri: true`) → `isTauri()` →
   `IpcBridgeTransport` del module-sdk → handlers `invoke` del shell → `crates/peripherals`
   (red 9100 · SPP en Android · cola RAW del SO por USB en escritorio, §«printer_id»).
   **Sin bridge WS, sin token de máquina, sin device-code.**

Login en la webview: **v1 = login normal de la PWA** (email o PIN). Fast-follow (Fase 3 de 0159):
token de usuario efímero emitido por el SaaS al cerrar el onboarding + boot-path de la PWA que lo
canjea con el `POST /api/auth/cloud` existente (session-exchange del runtime).

## Qué se CONSERVA del apps/tauri retirado (reuso, no reinventar)

- Los **handlers `invoke`**: **13** en total —10 de hardware (`erplora_*`, incluido
  `erplora_notify`) + 3 del shell (`device_context`, `forget_hub` y, desde hub#475,
  `open_external_url`)—. Este doc decía «15 handlers de hardware»; contados en
  `generate_handler!` el 2026-08-01, no lo eran (`erplora_print`, `erplora_discover_printers`,
  `erplora_open_drawer`, `erplora_test_print`, `erplora_get_devices`, `erplora_set_device_*`,
  `erplora_bridge_status`, …) + `build_peripherals_state` (DeviceRegistry + PrintQueue + Watchdog) —
  independientes del runtime, delegan en `crates/peripherals`.
- El **ACL de origen remoto**: `build.rs` (`AppManifest::commands` → autogenera `allow-<cmd>`) +
  `capabilities/*.json` (`remote.urls`). Sin esto Tauri rechaza todo `invoke` desde la PWA remota.
  **Hecho** en hub#334 → ver «[La frontera de orígenes](#la-frontera-de-orígenes-adr-0221-hub334)».
  ⚠️ Este bullet pedía además meter los orígenes cloud en la CSP `connect-src` de `tauri.conf.json`:
  **eso era falso** y no se ha hecho (ADR-0221) — esa CSP no llega nunca a una página remota.
- El **pipeline de empaquetado**: `msix/Package.appxmanifest` + `scripts/pack-msix.ps1` (MS Store,
  MSIX sin firmar, [ADR-0421](../../adr/ADR-0421.md)), `scripts/gen-tauri-icon.py`, workflow de release. **Se añade build
  macOS** (hoy no se compila en CI).

## Qué se ELIMINÓ (era el producto Hub Local, muerto por 0154) — HECHO

**Estado: completado y verificado el 2026-08-07 (ADR-0247, hub#336).** Esta lista se escribió como
tarea pendiente; el código ya no la contenía. Lo que faltaba —y es lo que aporta hub#336— era la
guarda que impide que vuelva: [`shell_surface.rs`](#el-surface-de-comandos-se-verifica-adr-0247-hub336).

- Dep `erplora-server` y todo el runtime embebido: `spawn_embedded_runtime`,
  `embedded_serve_config`, `wait_for_runtime`, `resolve_web_dir`, `LOOPBACK_CSP`, la rama loopback
  `127.0.0.1:8787` de `open_main_window`, SQLite en `app_data_dir`.
- El **`EntitlementGate`** completo (`validate_entitlement`): el entitlement lo aplica el hub cloud
  **server-side**; además su claim `deployment_mode` ya no existe en `EntitlementClaims`
  (saas#838/hub#217) → hoy ni compilaría.
- `frontendDist` apuntando a `../../web/dist`: hoy empaqueta `../shell-dist`, la página degradada.

⚠️ **`crates/cloud-client/src/entitlement.rs` NO se eliminó, y no debe eliminarse.** Verificar la
firma RS256 del entitlement sigue haciendo falta: lo llama la **revalidación híbrida del runtime**
(`crates/server/src/entitlement.rs`, ADR-0114 §6). Lo que murió fue su llamador Tauri, no la
verificación. Su cabecera decía lo contrario hasta hub#336 y era una invitación a borrarlo.

## El surface de comandos se verifica (ADR-0247, hub#336)

Cuatro artefactos describen la **misma** lista de comandos, y cada uno lo escribe una mano distinta:

| Artefacto | Lo escribe | Qué hace |
| --- | --- | --- |
| `build.rs` → `AppManifest::commands` | nosotros | hace que `tauri-build` emita `allow-<cmd>` |
| `permissions/autogenerated/*.toml` | `tauri-build` | los permisos emitidos, **commiteados** |
| `src/lib.rs` → `generate_handler!` | nosotros | los comandos que **existen** |
| `capabilities/*.json` | nosotros | qué origen recibe qué `allow-<cmd>` |

`build.rs` pedía la sincronía **en un comentario**. Un comentario no es una guarda, y la deriva
falla **en silencio y en dos direcciones opuestas**:

- **declarado y no implementado** → se emite un `allow-<cmd>` y una capability se lo concede a un
  origen para un comando que no existe: superficie de concesión muerta. Es exactamente lo que fue
  `validate_entitlement`, que siguió registrado en `build.rs` mucho después de que su gate se
  borrara — el síntoma que originó esta issue.
- **implementado y no declarado** → no existe `allow-<cmd>`, así que el ACL rechaza el comando
  desde **todo** origen remoto, que es lo único que el shell carga. **Dev lo enmascara** (un origen
  local sin app-manifest no pasa por el gate): se publica en verde y la caja **no imprime nunca**.

`apps/tauri/src-tauri/tests/shell_surface.rs` cierra el círculo entre los cuatro y fija lo retirado:
sin comandos de entitlement/enrollment, sin dependencia del runtime ni de BD local (ni en
`Cargo.toml` ni por `use` en `src/`), y `frontendDist` = `shell-dist/` con su `index.html`.

⚠️ **Hay un quinto comando que la PWA usa y que NO está en esa lista, porque no es nuestro:**
`plugin:app|version` ([ADR-0260](../../adr/ADR-0260.md)). Es de `core:app`, y
`core:default` lo traía dentro. Eso es lo que hizo posible el canal de actualización: una app
instalada **antes** de hub#400 contesta a la pregunta «¿qué build eres?» sin haberse actualizado.
Pero `core:app` es un plugin del núcleo de Tauri y no admite la segunda puerta: concedido por patrón,
cualquier página de erplora.com leía la build del TPV. **Desde hub#2658** `capabilities/default.json`
ya no concede `core:default`, sino sus juegos uno a uno **sin `core:app`**, del que solo quedan
`allow-register-listener` y `allow-remove-listener` (el botón Atrás de Android llega a la página como
el evento `back-button` de ese plugin, hub#1906; no dice nada de la build). La página pregunta
primero `plugin:app|version` —lo contestan las apps anteriores— y, si se lo niegan, la versión que
lleva `erplora_bridge_status`, detrás de `hub_link`. Recortar los juegos sin mirar sigue siendo una
dependencia invisible (deja de contestar **solo desde un origen remoto**, dev sigue bien): lo fija
`apps/tauri/src-tauri/tests/app_update_channel.rs` (el permiso de `erplora_bridge_status`, ningún
`core:app` salvo la escucha, y la escucha del Atrás).

## La frontera de orígenes (ADR-0221, hub#334)

Qué puede accionar el hardware de la caja lo decide **una sola cosa**: `remote.urls` de las
capabilities. La webview navega a páginas que **no** son la app empaquetada, y Tauri solo les deja
llamar a `invoke` si su origen casa con algún patrón declarado. Duele en las **dos** direcciones:
demasiado estrecha → el hardware está MUERTO en producción y no se nota hasta tener un hub real
delante (en dev se navega a loopback); demasiado ancha → lo que corra ahí abre el cajón.

Por eso la frontera va **partida en dos capabilities**, y no en una:

| Fichero | Orígenes | Qué concede | Por qué |
| --- | --- | --- | --- |
| `capabilities/default.json` | `https://*.erplora.com/*` · `http://127.0.0.1:8787/*` · `http://127.0.0.1:5173/*` | los 13 handlers (3 del shell + 10 de hardware) + `erplora-android:default` | es donde vive la PWA del hub; **qué** hub es el TPV lo decide la segunda puerta (hub#2504), también para `erplora-android` (hub#2642) |
| `capabilities/onboarding.json` | `https://erplora.com/*` · `http://127.0.0.1:8001/*` | **solo** `allow-device-context` + `allow-forget-hub` | es la página de ARRANQUE (ADR-0196); nunca imprime |

Las dos piezas que hay que entender:

- **El apex NO lo cubría el comodín.** `https://*.erplora.com/*` exige una etiqueta antes del punto,
  así que `erplora.com` —de donde ARRANCA la app— no casaba con nada y sus `invoke` se rechazaban.
  Declararlo es un acto deliberado, y va **en su propia capability**: `erplora.com` sirve también
  marketing, billing y un checkout de terceros; es la superficie más ancha que carga la ventana, y
  lo que se inyecte ahí no puede abrir el cajón. Se le da la identidad de dispositivo que arrastra
  el login del hub (`X-Device-Id`, sesión única ADR-0154) y la salida que olvida un hub capturado.
  Nada más.
- **El comodín de auras se CONSERVA, no se enumera.** La issue pedía `*.{letra}.erplora.com`. Los
  hubs viven en `{slug}.{aura}.erplora.com` con auras **por letra** en Hetzner y **por número** en
  el fallback AWS (`Server.get_hub_url`), así que enumerar auras dejaría fuera el fallback y
  convertiría «abrir un aura nueva» en «publicar una app nueva a cada TPV». El registrable
  `erplora.com` es nuestro y su DNS es autoritativo en Hetzner DNS desde el 2026-09-21 (antes en
  Cloudflare, ADR-0198, derogada por [ADR-0499](../../00-overview/decision-log.md#adr-0499)); el riesgo residual es el takeover de subdominio, de la misma clase que el
  del apex. **El comodín no distingue negocios**: casa con el hub de cualquier cliente, con `www` y
  con `pre`. Por eso no es la frontera del hardware, sino su primera mitad (siguiente apartado).

Los dos ficheros los ejerce `src-tauri/tests/remote_acl.rs` con el **mismo motor que Tauri usa en
runtime** (`RemoteUrlPattern`, WHATWG URLPattern) y **leyendo el directorio entero**, para que una
capability nueva no se cuele por no estar listada en el test. Guardas verificadas por mutación: el
apex en la capability de hardware, un comodín en blanco (`https://*/*`) y una permission de hardware
en `onboarding.json` ponen el test en rojo.

### La segunda puerta: solo el hub ENLAZADO maneja el equipo (hub#2504)

Un patrón no puede nombrar al único negocio al que pertenece este TPV: el hub se elige en tiempo de
ejecución, en el SaaS, mucho después de compilar el binario. Con solo el ACL, cualquier página de
`*.erplora.com` —otro negocio, la web pública, el SaaS de pruebas— abría el cajón e imprimía en el
TPV de otro. `src-tauri/src/hub_link.rs` pone una segunda puerta **delante del despachador de las
órdenes de la aplicación** (`run()` entrega a Tauri `hub_link::guard(app_commands())`):

- **Estado `HubLink`** (gestionado): el origen del hub **enlazado**, el de la página que enseña la
  ventana y el de la última página que **terminó de cargar** (`landed`, desde `on_page_load` con
  `PageLoadEvent::Finished`). Lo enlaza la app por su cuenta —el `hub.url` recordado al arrancar, el
  destino de un enlace `erplora://hub/…` **solo si es el ya enlazado o la persona contesta «Abrir»**
  (arranque en frío, en caliente y segunda instancia: `cold_start_link`, `open_hub_from_link`; ver
  §«Un enlace a otro negocio pregunta antes») — o el SaaS, con `?shell=1`. Se desenlaza en `forget_hub` y
  cuando el vigilante olvida un hub borrado.
- **La orden pasa** si es de las abiertas (`device_context`, `forget_hub`, `shell_retry`: lo que el
  onboarding y la página empaquetada necesitan) o si el ORIGEN de la página de la ventana
  (esquema + host + puerto) es el enlazado **y es la página que terminó de cargar**. Si no, se
  rechaza con el código `not_the_linked_hub` sin ejecutarse. **Falla cerrado**: sin estado
  gestionado o con una URL ilegible, nada pasa. Toda orden nueva nace cerrada.
- **Por qué la espera a la carga**: la puerta lee `webview.url()` porque Tauri comprueba su ACL
  contra el `Origin` de la petición pero no se lo entrega al handler. Y esa URL cambia al
  **empezar** la navegación (`URL` de WKWebView es la provisional; `Source` de WebView2 cambia antes
  de `ContentLoading`), mientras la página que la pidió sigue ejecutando JS hasta que la nueva se
  compromete: sin la espera, otro negocio podía hacer `location = <hub enlazado>` y pedir el cajón
  acto seguido. `follow` cierra la puerta en toda navegación permitida a otro origen y `landed` la
  abre al terminar la carga; una navegación que no llega a cargar deja el hub sin hardware hasta la
  siguiente que cargue. Android emite `Finished` en `onPageFinished`; wry no distingue plataformas,
  así que se espera siempre a `Finished` y no a `Started` (en Android, `onPageStarted` puede llegar
  con la página anterior aún viva).
- **El enlace también tiene dueño**, o la puerta quedaría a una navegación de ser inútil: cualquier
  página podría navegar a su propia dirección con `?shell=1` y convertirse en «el hub».
  `HubLink::follow` (llamado desde `on_navigation`, después del veredicto de Play) solo enlaza un
  `?shell=1` si la ventana **sale de una página de entrada** (el origen del SaaS; en desarrollo
  también el de `ERPLORA_SHELL_URL`) o cae en el hub ya enlazado. `shell_capture_origin` sigue
  siendo la regla de **qué** se puede recordar; `follow` añade **quién** lo elige.

- **El plugin de Android, detrás de la misma puerta (hub#2642)**: Tauri no manda
  `plugin:erplora-android|…` al `invoke_handler` de la app, sino al del plugin, así que
  `hub_link::guard` no lo ve. Como el plugin es nuestro, `tauri_plugin_erplora_android::init(gate)`
  recibe la puerta y la corre delante de su propio manejador; `run` lo registra con
  `hub_link::android_plugin()` = `init(linked_hub_only)`, la misma función que `guard` usa para las
  órdenes cerradas. Ninguna orden del plugin está abierta (permisos, escucha, salir, ajustes). Un
  rechazo devuelve `true` («atendida») **a propósito**: en móvil, una orden de plugin que el lado
  Rust no atiende cae al Kotlin (`plugin::mobile::run_command` en `webview/mod.rs` de Tauri 2.11),
  que la ejecutaría.

- **El plugin de avisos, envuelto (hub#2658)**: `tauri-plugin-notification` no es nuestro y no
  acepta una puerta, así que `run` lo registra con `hub_link::notification_plugin()`, que lo envuelve
  en `hub_link::Gated`: un `Plugin` que delega nombre, `initialize`, `initialization_script` y los
  ganchos en el original y, en `extend_api`, corre `linked_hub_only` antes; un rechazo contesta
  `not_the_linked_hub` y devuelve `true` por la misma razón que el de Android. Así
  `plugin:notification|register_listener` (oír los toques de los avisos) es solo del enlazado.
  La página del hub se suscribe y pide la versión **al arrancar**, antes de que la ventana dé la carga
  por terminada (la puerta aún contesta `not_the_linked_hub` tras llegar desde el SaaS):
  `whileTheHubLands` (`apps/web/src/lib/device.ts`) reintenta solo ese rechazo, tras el `load` y
  durante unos 8 s, y cualquier otro error vuelve a la primera.
- **`plugin:app` no se puede envolver** (es del núcleo): ver «quinto comando» arriba; la versión pasa
  a `erplora_bridge_status`.

Lo que **no** cubre, con su issue: un enlace `erplora://hub/…` acepta como destino también `www`/`pre` (hub#2645), aunque desde hub#2644
solo enlaza otro negocio tras confirmarlo la persona; y el binario de producción acepta el bucle local
(hub#2643).

Tests: `src/hub_link.rs` (unidad + ida y vuelta por IPC sobre `MockRuntime`, con un despachador
falso y la petición como página empaquetada para medir la puerta sin el ACL; y, para hub#2642, el
plugin de Android real tal como lo registra `run`, con sus cinco órdenes concedidas a la página) y
`the_app_wires_the_linked_hub_gate` en `src/lib.rs` (los puntos de llamada, también
`.plugin(hub_link::android_plugin())`). Decisión:
[la segunda puerta del hardware](../../adr/2026-10-08-el-hardware-de-la-app-solo-para-el-hub-enlazado.md).

### La salida al navegador del sistema (ADR-0255, hub#475)

La frontera de arriba dice quién puede accionar el hardware. Esta dice **a dónde puede mandar al
usuario** la página que la ventana carga — y hasta hub#475 no existía, porque **no se podía salir**:
`window.open` desde la PWA abría **nada** (la webview no crea ventanas y ningún plugin `shell`/
`opener` estaba declarado). El helper `openExternal` del web app lo usaba para **8 llamadas /
10 botones**, y 9 de esos 10 **cobran**: la compra de módulos que **ADR-0114 §4 sacó de la app
precisamente por las políticas de Google Play** fallaba, en silencio, dentro de la app de Play.

| Pieza | Qué es |
|---|---|
| `tauri-plugin-opener` | plugin oficial: `ACTION_VIEW` en Android, el opener del SO en escritorio |
| `open_external_url` (`src/lib.rs`) | **nuestro** comando: comprueba y delega. Es lo único que la página puede llamar |
| `external_browser_url` (`src/lib.rs`) | el predicado: qué dirección se acepta |
| `allow-open-external-url` en `capabilities/default.json` | se concede **solo** a la PWA del hub; el onboarding no lo tiene |

**El plugin se enlaza por su API de Rust, y ninguna capability concede `opener:*`.** Los comandos
del plugin aceptan **cualquier** dirección y dos de ellos (`open-path`, `reveal-item-in-dir`)
direccionan el **sistema de ficheros** de la máquina del TPV: `opener:default` sería un cambio de una
palabra, con pinta de fontanería, que entrega a toda página que la app carga un lanzador de URLs,
rutas y esquemas arbitrarios. Lo fija `tests/remote_acl.rs`
(`no_capability_hands_a_page_the_raw_opener_plugin`).

**Qué acepta `external_browser_url`** — `https` en `erplora.com` (**el ápice**) o en cualquier hub
(`is_hub_domain`, auras por comodín), `https` en `checkout.stripe.com` **por host exacto** (hub#1914:
la ampliación del asistente es su propia suscripción de Stripe, ADR-0033, y el SaaS contesta con la
página alojada de Stripe sin ninguna de erplora.com delante; sin él, «Ver planes» del asistente solo
podía fallar dentro de la app, así que navegaba la ventana del TPV y dejaba al dueño sin vuelta), más
`http` a loopback para desarrollo. Ningún otro host de Stripe ni comparación por sufijo
(`tests/external_open.rs`, `stripe_is_admitted_by_its_exact_checkout_host_only`). Nada más: ni `file:`
(el SO ejecuta lo que tenga asociado), ni `javascript:`, ni esquemas propios (arrancan **otro**
programa), ni credenciales en la URL (`https://support%40evil.example@erplora.com/` es host nuestro
y página de nadie).

⚠️ **El ápice entra aquí y NO en las otras dos fronteras, a propósito.** `is_hub_domain` /
`trusted_hub_origin` (ADR-0221/0243) contestan «¿esta página abre el cajón / es el hogar de este
TPV?»; esta contesta «¿mando ahí el navegador del usuario?». El ápice **es** el SaaS —donde viven
los 8 destinos— y una visita no concede nada: la página se abre en el navegador, con su propio
sandbox, sin `invoke` y sin camino de vuelta a la app.

**No se usa `"inAppBrowser"`** (la Custom Tab de Android) a propósito: eso es *chrome* que hospeda
nuestra app, y lo que hace falta es el navegador **como aplicación aparte** — el pago tiene que
ocurrir **fuera** de la app.

### Guardar un fichero donde el usuario lo encuentre ([ADR-0259](../../adr/ADR-0259.md), hub#480)

La frontera de arriba manda al usuario a una **página**. Esta escribe **bytes** que la página ya
tiene, y son dos cosas distintas: los ficheros que descarga el hub —un documento de `/files`, la
copia de seguridad, el PDF de una factura— se piden **con la sesión del hub**, así que un navegador
aparte no puede ir a buscarlos. Mandar el navegador no era una opción; lo que faltaba era **sitio**.

| Pieza | Qué es |
|---|---|
| `save_download` (`src/lib.rs`) | el comando: comprueba, guarda y **devuelve dónde quedó** |
| `save_target` | **una sola** decisión: ¿carpeta, colección pública de Android, o nada? |
| `reachable_downloads_dir` | ¿hay carpeta de Descargas que el **usuario** pueda abrir en esta plataforma? |
| `download_file_name` | reduce lo que mande la página a **un solo nombre de hoja** |
| `free_download_path` | `nombre (2).ext`: guardar dos veces deja **dos** ficheros (escritorio) |
| `DownloadPublisher` (Kotlin, `crates/tauri-plugin-erplora-android`) | el `MediaStore` de Android: publica en Descargas y dice el sitio |
| `allow-save-download` en `capabilities/default.json` | se concede **solo** a la PWA del hub |

**La página nombra un FICHERO; el shell elige el SITIO.** Ese reparto *es* la frontera: el sitio lo
decide `save_target` y lo único que aporta la página es el último componente del nombre, ya
filtrado. `download_file_name` rechaza separadores (`../../.ssh/authorized_keys`), los dos puntos
(`informe.pdf:oculto.exe` es un *alternate data stream* de NTFS — un fichero que no sale en ningún
listado), los caracteres de control (un NUL trunca el nombre en la llamada al sistema: lo que se
escribe no es lo que se nombró), los nombres de solo puntos y lo que ningún sistema de ficheros
acepta.

**Se devuelve DÓNDE QUEDÓ, y no es decoración.** Dentro de la app instalada no hay barra de
descargas, ni notificación, ni botón de Descargas: si el comando no dice dónde quedó el fichero, no
lo dice nadie. Es la misma regla de ADR-0255 —un camino que no se puede completar se dice— aplicada
al que **sí** se completa. En escritorio eso es la ruta absoluta; en Android es `Download/<nombre>`,
porque ahí **no hay ruta que enseñar** (ver abajo).

#### Android: guardar NO es escribir en una ruta ([ADR-0307](../../adr/ADR-0307.md), hub#499)

⚠️ El `download_dir()` de Tauri en Android es `getExternalFilesDir(DIRECTORY_DOWNLOADS)` —
`/storage/emulated/0/Android/data/com.erplora.app/files/Download`— y **Android 11 cerró
`Android/data`** a la app Archivos del sistema y a todo gestor de ficheros de terceros. Escribir ahí
y contestar «guardado en …» sería el fallo mudo de hub#475 con un mensaje de éxito encima. Por eso
`reachable_downloads_dir` sigue diciendo que **en Android esa carpeta no vale**, y por eso
`save_target` contesta por **plataforma antes que por ruta**: Android **sí** devuelve una carpeta,
así que una comprobación que solo mirase si vino path no saltaría nunca.

La puerta buena no es una carpeta: es la **colección pública de Descargas**. Un `insert` en
`MediaStore.Downloads.EXTERNAL_CONTENT_URI` devuelve un `content://`, los bytes salen por su
`OutputStream` y el fichero cae en las mismas Descargas donde escribe cualquier navegador — visible
en Archivos y en cualquier gestor de ficheros. **No pide ningún permiso** (la app es dueña de la
fila que acaba de crear), que es justo por lo que se eligió frente a
`getExternalStoragePublicDirectory`, que exige `WRITE_EXTERNAL_STORAGE` y por tanto un diálogo.

Cómo viaja el fichero: el shell lo deja **en su propia caché** y le pasa a Kotlin la **ruta**, no
los bytes. Un export de 5 MB ya cruzó el `invoke` una vez como base64; copiarlo otra vez al heap de
una tablet para cruzar JNI es la copia que sí se nota. El fichero de staging se borra siempre,
también si la publicación falla.

Se publica **pendiente** (`IS_PENDING = 1`) y solo se hace visible cuando ha entrado el último byte:
una copia de seguridad a medias en Descargas es peor que ninguna, porque parece la copia que el
negocio tiene. Y el nombre se **relee** de la fila: si ya existía, MediaStore pone el suyo
(`hub (1).zip`), así que guardar dos veces deja dos ficheros —igual que en escritorio— y lo que se
le dice al usuario es el fichero que de verdad existe.

⚠️ **`save_download` es `#[tauri::command(async)]`, y eso no es estilo.** `run_mobile_plugin`
despacha al *main looper* de Android y **bloquea** esperando respuesta; un comando síncrono corre en
ese mismo hilo, así que el TPV se quedaría colgado para siempre justo en el botón que se lleva su
copia de seguridad.

**Lo que sigue rechazado, a propósito:**

- **Android < 10 (API 24–28).** No hay colección pública; la única vía es
  `getExternalStoragePublicDirectory` + `WRITE_EXTERNAL_STORAGE`, que es un permiso de **runtime** y
  por tanto otro diseño (un diálogo que el guardado tiene que esperar). Kotlin rechaza con el código
  `downloads_unreachable`, que la página traduce a la frase de siempre —«abre tu negocio en un
  navegador»— que ahí sigue siendo verdad y accionable. El `minSdk` del proyecto es 24; subirlo a 29
  cerraría esto de un plumazo y es decisión de producto, no de este cambio.
- **iOS.** El mismo agujero, y necesita su propio trabajo nativo
  (`UIDocumentPickerViewController` o la hoja de compartir). Hoy no se compila iOS.

Una sola palabra recorre la cadena entera: `DownloadPublisher.DOWNLOADS_UNREACHABLE` (Kotlin) →
`[código] - mensaje` (Tauri) → `Error::DownloadsUnreachable` (plugin Rust) →
`ShellError::DownloadsUnreachable` → `download.noPlaceToSave` (`save-download.ts`). Hay tests en los
dos extremos que fallan si alguien la escribe distinto.

**Ni capability nueva.** `save_to_downloads` **no** se registra como comando invocable desde la
webview: la única puerta sigue siendo `save_download` del shell, con su `allow-save-download`. Y
**`external_browser_url` no cambió**: ninguno de estos destinos es una dirección.

### Imprimir una A4 con el diálogo del sistema (hub#2006)

Dentro de la webview `window.print()` no imprime nada (hub#862), así que una factura A4 no tenía
cómo llegar a una impresora láser ni a «Guardar como PDF». **No se convierte HTML→PDF en Rust**:
Tauri 2 ya imprime una webview de forma nativa (`Webview::print` de wry — `NSPrintOperation` en
macOS, el diálogo de WebView2 en Windows). Lo que pone el shell es una ventana cuyo **único**
contenido es el documento.

| Pieza | Qué es |
|---|---|
| `print_document` (`src/lib.rs`) | el comando: toma el `html`; en escritorio abre la ventana, en Android lo pasa al plugin (`print_html`, hub#2008); en otra plataforma contesta `native_print_unsupported` |
| `open_print_window` (`src/native_print.rs`) | ventana `print-<n>` que carga `erplora-print://localhost/<n>` (Windows/Android: `http://erplora-print.localhost/<n>`) y, al terminar de cargar, llama **una vez** a `print()` |
| `PrintDocuments` | los documentos en memoria por id; se sirven hasta que su ventana se destruye (un 404 imprime en blanco) |
| `PRINT_DOCUMENT_CSP` | `default-src 'none'` sin fuente de scripts: el documento **no ejecuta código** |
| `allow-print-document` en `capabilities/default.json` | se concede **solo** a la PWA del hub |

**El documento viene de una página remota y se enseña en una ventana de la app**, así que se
contiene por tres lados: la CSP de su respuesta no deja correr ningún script; **ninguna capability
nombra la ventana `print-*`** (sin IPC: `tests/native_print.rs` lo fija); y la ventana no puede
navegar fuera de su documento (`print_window_may_navigate`). La ventana queda abierta detrás del
diálogo como vista previa y se cierra como cualquier otra; su documento se va con ella.

⚠️ **`print_document` es `#[tauri::command(async)]`**: crear una ventana desde un comando síncrono
bloquea el hilo principal en Windows.

**Verificado en macOS** (2026-09-24) con una sonda sobre `open_print_window`: se abre la hoja de
impresión nativa («Page 1 of 1», selector de impresora, menú PDF), el `<script>` del documento **no
corre**, y «Save as PDF» produce un PDF A4 de una página con el texto de la factura. Windows queda
sin verificar en máquina real (el camino de wry es `window.print()` en WebView2).

**Android (hub#2008).** wry no implementa `print` en Android, así que el shell no abre ventana:
`print_document` valida el documento (`check_print_document`, mismos límites) y llama a
`ErploraAndroid::print_html` del plugin `crates/tauri-plugin-erplora-android`. En Kotlin,
`HtmlPrinter` pinta el `html` en un `WebView` **propio y fuera de pantalla** (JavaScript apagado, sin
acceso a ficheros, sin puente, sin navegar: la misma frontera que la CSP del escritorio) y, al
terminar de cargar, llama a `PrintManager.print` con `createPrintDocumentAdapter` y **A4
preseleccionado**: la pantalla de impresión del sistema, con todas las impresoras que conoce el
dispositivo y «Guardar como PDF». El nombre del trabajo (y del PDF) es el `<title>` del documento,
saneado como nombre de fichero (`HtmlPrinter.jobNameOf`). El `WebView` se retiene hasta el
`onFinish` del adaptador. Resuelve al abrirse la pantalla, como en escritorio.

⚠️ `invoke.resolve()` sin datos llega a Rust como JSON `null`: la respuesta se lee como
`serde::de::IgnoredAny`. Leída como `Empty`, la pantalla se abría y la llamada devolvía **error**
(visto en el emulador). `bluetooth_print` tenía el mismo patrón y se lee igual (hub#2024).

**Verificado en el emulador** (Pixel Tablet API 36, `-gpu host`, 2026-09-24) invocando
`print_document` desde la webview: pantalla del sistema con «ISO A4», «Save as PDF» y «All
printers…»; el `<script>` del documento no corre; «Save to PDF» propone «Factura F-2026-0001» y deja
un PDF real en Descargas; cancelar dos veces seguidas no tumba la app; un documento vacío se rechaza
con `print_document_refused`.

### La CSP del shell NO lleva los orígenes cloud

`app.security.csp` de `tauri.conf.json` **solo** se aplica a los assets que Tauri sirve desde
`frontendDist` por su propio protocolo (`manager::AppManager::get_asset` → `manager::set_csp`,
leído en tauri 2.11.5). Una webview que navega a `https://erplora.com` o a un hub lleva la CSP que
**ese servidor** manda; la del config no la toca. Meter ahí los orígenes cloud —lo que pedía este
doc— ensancharía la página empaquetada a cambio de nada.

Lo que la CSP sí gobierna es `shell-dist/index.html`, la página degradada, y ahí había un fallo
silencioso: su único control era un `onclick` inline y **CSP no tiene ni hash ni nonce para un
manejador inline**, así que el botón «Reintentar» no hacía nada. Ahora se cablea desde un
`<script nonce="__TAURI_SCRIPT_NONCE__">` (el token que Tauri sustituye al servir el asset,
`tauri_utils::assets::SCRIPT_NONCE_TOKEN`). `src-tauri/tests/shell_csp.rs` fija el contrato:

- `default-src 'self'`, `object-src`/`frame-src` a `'none'`, `base-uri 'self'`;
- ni `'unsafe-inline'` ni `'unsafe-eval'` en `script-src`;
- **`form-action` sigue SIN declarar, a propósito** — no hereda de `default-src`, y fijarlo es lo que
  rompe la cadena de redirects del login federado con Google;
- ningún origen `erplora.com` dentro de la propia CSP;
- la página degradada no trae manejadores inline ni tira de ningún asset remoto.

#### …y la que sí gobierna la app la manda el HUB ([ADR-0308](../../00-overview/decision-log.md#adr-0308))

Corolario de lo de arriba, y conviene decirlo entero porque es contraintuitivo: como la ventana
navega a `https://<hub>.erplora.com`, **la política que corre el 99 % del tiempo en la app instalada
la emite el runtime del hub**, no este config. Se documenta en
[`web.md` §La CSP servida](web.md#la-csp-servida-adr-0308-hub708).

Hasta el 2026-08-11 esa cabecera **no existía** (hub#708), así que la app instalada corría de hecho
**sin CSP** salvo en su página degradada — que era la única que tenía una. ⚠️ Esa coletilla decía
«y la única que no importa»: **desde hub#1716 es falsa**. La página degradada dejó de ser un brazo
muerto y pasó a ser la pantalla que ve el usuario cada vez que se cae la red (§ siguiente), así que
su CSP gobierna la única pantalla alcanzable de la app en ese momento — y es la que hace que el
botón «Reintentar» necesite el `nonce` para existir.

⚠️ **Y por eso el `connect-src` del hub lleva `ipc:` y `http://ipc.localhost`.** El `invoke` de Tauri
no es magia del webview: es un `fetch` a `ipc://localhost/<cmd>`
(`tauri/src/ipc/protocol.rs`, leído en 2.11.5), que en Windows y Android se reescribe a
`http://ipc.localhost/<cmd>`. Como el documento es remoto, ese `fetch` lo filtra la CSP **del hub**.
Si alguien la endurece ahí quitando esas dos fuentes, **el TPV deja de imprimir en la app instalada**
—cajón y descubrimiento incluidos— en silencio y sin que ningún test del navegador se entere. El
guard vive en `crates/server/tests/cloud_csp.rs`
(`the_installed_app_can_still_reach_the_hardware`), no aquí, porque es allí donde se decide.

## Cuando se cae la RED: la ventana tiene su propia pantalla y vuelve sola (hub#1716)

El shell es un cliente fino: **todo** lo que el usuario ve lo sirve un origen remoto. Una navegación
que no aterriza no es un caso límite — en un bar o un salón la línea se cae cada pocos días.

**Lo que había hasta hub#1716.** `shell-dist/index.html` existía, pero `open_main_window` solo
llegaba a ella desde el brazo `Err(_)` de `initial.parse::<tauri::Url>()` —una rama cuyo propio
comentario decía que no debería pasar—, y `on_navigation` devolvía `true` sin mirar nunca si la
carga había aterrizado. Todo fallo real acababa en lo que pinta la webview de la plataforma:

- macOS (WKWebView): **ventana en blanco total**, sin mensaje, sin logo y sin control;
- Android (Chromium): la página gris `ERR_INTERNET_DISCONNECTED` / `ERR_NAME_NOT_RESOLVED`.

Una ventana de Tauri no tiene barra de direcciones ni botón de recarga, así que de ninguna de las
dos se sale: la caja está muerta hasta matar la app desde el sistema operativo.

**Por qué se SONDEA y no se escucha un callback de fallo.** No existe. Tauri 2 ofrece
`on_navigation` (dispara ANTES de la carga) y `on_page_load` (`Started`/`Finished`), y «finished» no
es evidencia portable: el WebView de Android dispara `onPageFinished` también para su propia página
de error, justo en la plataforma donde se midió el síntoma. Lo que sí es portable es preguntarle a
la red nosotros (`src-tauri/src/connectivity.rs`).

La regla es de **TRANSPORTE, no de HTTP**: cualquier respuesta —200, 404, 502— significa que hay red
y que lo que pinta la página es asunto del origen (un 404 es problema de `should_forget_hub`). Solo
un fallo de transporte (DNS, connect, timeout) significa que lo que el usuario mira no podemos ser
nosotros.

**Qué cuesta.** Nada mientras hay red. El guardia sondea una vez poco después de arrancar, una vez
por navegación remota, y después duerme en un `Notify` **sin ningún temporizador**; solo hace
polling —con backoff 2/4/8/16/30 s— mientras enseña la página offline, que es exactamente cuando
alguien está esperando a que vuelva la red. Importa: esto corre en tablets con datos móviles.

⚠️ **El debounce de dos fallos casi significa «nunca».** `OFFLINE_STRIKES = 2` evita arrancar al
usuario de una página que funcionaba por un roam de wifi o un portal cautivo, pero la primera
versión decidía el siguiente sondeo mirando **solo la pantalla actual**: tras el primer fallo la
ventana seguía en `Target`, así que el guardia volvía a dormirse y el segundo strike no llegaba
nunca. Medido con la app real: la ventana seguía en blanco con el guardia armado. El horario lo
decide `next_probe_delay(screen, failures)` y **un strike en el marcador siempre programa el sondeo
que lo confirma**.

**La página.** `shell-dist/index.html` va empaquetada entera —marca incluida, porque es justo la que
se pinta cuando no hay red para pedir nada— y lleva sus dos catálogos inline: cadena `en` fuente +
su traducción `es` (ADR-0055/0199), con `es` por defecto como el shell del hub. El copy dice
explícitamente que es la **red** y no un fallo de la aplicación, y que no se ha perdido nada.

**El botón «Reintentar» no recarga: pregunta al shell.** Un `location.reload()` en esta página
recarga ESTA página — el único control de la única pantalla alcanzable llevaba a sí mismo. Ahora
invoca el comando `shell_retry`, que es el único lado capaz de sondear la red y navegar la ventana.
🔐 Por eso `shell_retry` vive en su propia capability `degraded.json` **sin bloque `remote`**
(local-only): un comando que navega la ventana principal no puede quedar al alcance de un origen
remoto, o cualquier página que cargue la caja —o cualquier cosa inyectada en ella— podría llevársela
donde quisiera.

Contrato fijado por `src-tauri/tests/shell_offline.rs` (marca, copy que culpa a la red en las dos
cadenas, paridad de claves `en`/`es`, el reintento que llama al shell, el guardia armado en
`open_main_window` y la capability local-only) y por el `#[cfg(test)]` de `connectivity.rs`, que
además conduce una `WebviewWindow` real sobre el mock runtime de Tauri: la regresión se afirma como
un **movimiento** de la ventana, no como un booleano.

> Esto es la **mitigación visible**, no continuidad sin conexión. Operar offline (pm#119) sigue
> fuera del MVP: aquí la caja no vende sin red, pero la persona ve de quién es la pantalla, entiende
> qué pasó y la app vuelve sola en cuanto hay línea.

## Qué páginas del SaaS sigue la copia de Google Play (ADR en borrador «La copia de Google Play solo sigue, dentro de su ventana, las páginas del SaaS que no cobran», hub#1915)

La app arranca en el SaaS (`/shell/`) y enseña dentro de su ventana el login, el alta y la cuenta.
Hasta hub#1915 `on_navigation` devolvía `true` sin mirar, así que esa ventana podía llegar a
**cualquier** página de erplora.com: la portada con los precios, el marketplace público, el panel
con Facturación. En la copia de Google Play cada una es una puerta a pagar fuera de Play — motivo de
rechazo — y una sola vuelta del QA de Android encontró seis, tapadas a mano en sus plantillas
(hub#1900, hub#1910, saas#2083, saas#2084, saas#2085 y el «Log Out» y el banner de cookies de la
revisión de saas#2088). El siguiente enlace de la web abre la séptima.

**Decide la app.** `src/navigation.rs` (`navigation_verdict`) juzga **cada** navegación antes de que
cargue, y en la copia `play` (`distribution_channel`) solo sigue estas páginas del SaaS:

| Prefijo | Para qué |
|---|---|
| `/shell/` | el arranque de la app: elegir, abrir o crear el hub |
| `/account/`, `/register/`, `/accounts/` | entrar, darse de alta, verificar el correo, recuperar la contraseña, 2FA, reautenticar, salir |
| `/auth/` | el login con Google y los pases de un solo uso al hub |
| `/post-login/` | el enrutador tras el login; su destino se juzga al redirigir |
| `/start/`, `/enter/`, `/businesses/`, `/invitation/`, `/hub/starting/` | el primer hub gratis, despertar uno dormido, entrar, elegir, unirse y la espera mientras arranca |
| `/legal/`, `/cookies/` | Google Play exige la política de privacidad accesible desde la app; el banner de cookies enlaza a sus ajustes |
| `/dashboard/profile/…` **solo** con `?surface=account` (+ `/dashboard/profile/deleted/`) | la cuenta sola, sin el panel (ADR en borrador «La puerta de la app a la cuenta de erplora.com…», hub#1900, saas#2084) |

Todo lo demás del SaaS se **rechaza**: la ventana se queda donde estaba y sale un `alert` con
«Esta página no está disponible en la aplicación» (`en` fuente + `es`, elegido como la pantalla sin
conexión). La portada `/` no se rechaza: lleva al **arranque de la app** (`/shell/`), porque el SaaS
manda ahí sin preguntar —el POST de «Log Out», el logo y la «←» del login— y quedarse en una página
ya sin sesión sería peor. Las demás copias (`direct`, `msstore`) siguen todos los enlaces como antes.

- **Qué es «el SaaS».** Los hosts de producción **por nombre** —`erplora.com`, `www.erplora.com` y
  `pre.erplora.com`, que sirven la web entera y tienen forma de hub— con cualquier esquema, puerto o
  punto final (`erplora.com.`: Django lo acepta), más el SaaS configurado (`ERPLORA_SAAS_URL`) por
  **origen**, para que un hub local en otro puerto siga siendo un hub.
- **Qué no se vigila.** Los hubs (llevan su propia guardia, `no-purchase-steering.test.ts` del web),
  el loopback, la página empaquetada y los hosts ajenos: los destinos de pago que motivan esto se
  alcanzan a través del SaaS, y el SaaS es lo que la lista cubre.
- **La marca de la cuenta se lee como la lee el SaaS**: `request.GET.get` de Django devuelve el
  ÚLTIMO valor, así que `?surface=account&surface=panel` es el panel y se rechaza.
- **Orden dentro del manejador.** El veredicto va primero y todo lo que no sea `Allow` devuelve
  `false`: una página rechazada no se captura como `hub_url` ni pasa a ser el destino del guardia de
  conectividad, que navega por programa (y eso no lo ve ningún manejador). La respuesta —navegar o
  el `alert`— sale fuera del manejador, en el runtime async: en Android `on_navigation` corre dentro
  de `shouldOverrideUrlLoading`, en el hilo de UI y bajo el cerrojo de wry.

**Límites, dichos.** Esto gobierna la **ventana** de la app, no el navegador del sistema: lo que
sale por `openExternal` (`external_browser_url`, que acepta cualquier página del ápice) sigue a cargo
de la guardia del web (`no-purchase-steering.test.ts`) — así salen las puertas de hub#1900 y
hub#1910. En Android el gancho solo ve navegaciones del marco principal por GET (las redirecciones
incluidas); un `POST` no pasa, pero la redirección que lo sigue sí. Y **no ve la navegación por
htmx** del panel (`hx-get` + `hx-push-url`): eso es una petición en la página, no una navegación, y
la cubren las plantillas del SaaS (saas#2084). Las dos barreras nativas que faltan: hub#1918.

Contrato fijado por `src-tauri/tests/play_navigation.rs` (la política página a página, con el
control positivo de lo que vende; y que `open_main_window` pregunta el veredicto de ESTA copia antes
de capturar o vigilar, y solo sigue `Allow`) y por el `#[cfg(test)]` de `lib.rs`, que conduce una
`WebviewWindow` del mock runtime: la portada se afirma como un **movimiento** a `/shell/` y el
rechazo como su ausencia.

## La captura de `hub_url`: quién puede pedir ser recordado (ADR-0243, hub#335)

`hub.url` es **a qué hub apunta esta instalación**: lo que la ventana abre a pantalla completa, sin
barra de direcciones y bajo el título «ERPlora», en cada arranque en frío a partir de ese momento.
Por eso el fichero es una frontera de confianza y no un caché.

Lo escribía **solo** el marcador `?shell=1`. Y el marcador dice «recuérdame», no «soy de fiar»: lo
lleva la URL a la que se navega, y el shell **no bloqueaba ninguna navegación** (`on_navigation`
devolvía siempre `true`, solo observaba; desde hub#1915 rechaza páginas del SaaS, y solo en la copia
de Play). Así que cualquier `https://…/?shell=1` al que llegara la
ventana —un redirect abierto del SaaS, una inyección en el checkout de terceros que carga esa misma
ventana, o un enlace que el usuario toca— se quedaba grabado como el hub del TPV, **para siempre y
sin un solo error**. La suite lo tenía además **fijado**: un test afirmaba que
`https://hub.example.com:8443/?shell=1` se capturaba, con el puerto bien conservado.

**El destino se comprueba, no solo se parsea**, y con la MISMA regla que ya aplicaban las otras dos
mitades del mismo problema:

| Quién decide | Fichero | Qué decide |
|---|---|---|
| `capabilities/*.json` (ADR-0221) | `default.json` · `onboarding.json` | qué origen puede accionar el hardware (primera puerta, por patrón) |
| `hub_link::guard` / `HubLink` (hub#2504) | `src/hub_link.rs` | de esos orígenes, cuál: **solo el hub enlazado** |
| `resolve_deep_link` (ADR-0225) | `src/lib.rs` | qué destino puede abrir un enlace |
| **`shell_capture_origin` / `load_hub_url`** (esta) | `src/lib.rs` | **qué origen se puede RECORDAR** |

La regla, una sola: **un hub nuestro** (`<etiqueta>[.<etiqueta>…].erplora.com`, auras por comodín) o
**loopback de esta máquina** (desarrollo). El ápice `erplora.com` queda fuera —es de donde ARRANCA
la app, no un hub—; los tres predicados salen ahora del mismo `is_hub_domain`.

Que las tres coincidan importa **en las dos direcciones**, y el test
`the_shell_only_remembers_origins_that_can_drive_the_till` (en `tests/remote_acl.rs`) las contrasta
origen por origen contra las capabilities reales:

- **recordado pero no autorizado** → el TPV arranca en una página cuyos `invoke` rechaza el ACL de
  Tauri: no puede ni pedir `device_context`, así que parece que va y no imprime;
- **autorizado pero no recordado** → nunca entra en «modo app» en un hub del que se fía del todo, y
  pasea al usuario por el onboarding en cada arranque.

**Se revalida al LEER, no solo al escribir.** Filtrar únicamente la captura protege a los TPV que
todavía no han capturado nada; el que ya corrió una build sin el filtro lleva el origen ajeno en el
fichero, y una actualización que se lo vuelva a creer no arregla justo el caso que importa. `hub.url`
es además texto plano en el directorio de datos del usuario: nada impide editarlo. Rechazarlo
devuelve `None`, que es «arranca en el onboarding» — la única pantalla de la que se puede salir.

### Las cuatro preguntas del ciclo de vida

- **Primera vez (aún no hay hub)** → no hay `hub.url`: `initial_url_for` cae al onboarding del SaaS
  (`{saas}/shell/`). El hub gratis se auto-crea al registrarse (ADR-0154).
- **Varios hubs / cambiar de hub** → `/shell/?choose=1` lista los del usuario y hace 302 al elegido
  **con el marcador**, así que la captura que graba el hub B es la misma que grabó el A: sobrescribe.
  La salida desde dentro del hub es el `invoke` `forget_hub`, que borra la captura y devuelve la
  ventana al onboarding. Lo llaman el frontend al recibir un **410** del Cloud (sin `choose`) y el
  control **«Cambiar de negocio»** de la barra superior, tras confirmarlo (`choose: true` →
  `/shell/?choose=1`; hub#447). Las dos salidas pasan por `signOutAndForgetHub`
  (`apps/web/src/lib/change-hub.ts`, hub#2503) y el **orden es el contrato**: (1) para la escucha de
  avisos de Android mientras la página sigue siendo el negocio enlazado (después la app rechaza la
  orden, hub#2642); (2) cierra la sesión **esperando** a que se borren los tokens de erplora.com y a
  que el hub conteste la revocación (`POST /api/auth/logout`), con un tope de 5 s para que un hub
  mudo no retenga a nadie; (3) solo entonces `forget_hub`, porque su navegación corta cualquier
  petición en vuelo. Antes de hub#2503, `forget_hub` iba solo y la sesión, el nombre, el correo y
  los tokens se quedaban en el almacenamiento del origen.
- **El hub guardado ya no existe** (purga: hoy los hubs se borran de verdad, con su BD) → al
  arrancar, un `HEAD` en segundo plano; **solo** 404/410 olvidan (`should_forget_hub`). Caído, sin
  red o sin autenticar **no** son «no existe»: ante la duda se conserva, porque olvidar de más obliga
  a rehacer el onboarding y olvidar de menos solo deja una pantalla fea que se arregla sola.
- **Un `hub_url` arbitrario inyectado desde la página** → es lo que esta sección cierra.

⚠️ **Dominio propio**: `Hub.access_url` prefiere el `HubCustomDomain` primario, así que
`/shell/open/<id>/` puede redirigir a `https://pos.minegocio.com/?shell=1`. Ese origen no está —ni
puede estar— en ninguna `remote.urls`, así que **ya hoy** carga sin un solo `invoke` concedido
(ADR-0159 lo dejó escrito: «custom domains sin hardware en v1»). Con esta regla tampoco se recuerda:
el arranque cae al onboarding y, con un solo hub, `/shell/` redirige de nuevo allí —mismos clics—,
pero el fallo deja de esconderse tras un «modo app» que no lo era. Lo que sigue roto es el hardware:
**hub#448**.

### Las páginas del SaaS dentro de la app cierran sus puertas a la web pública (saas#2083, saas#2085)

Antes de capturar un hub, la ventana enseña páginas **del SaaS**: entrar, las de contraseña y 2FA,
`/shell/` y `/shell/create/`, y desde ellas las públicas que hacen falta (Términos, Privacidad y el
detalle de cookies del banner de consentimiento, que una instalación nueva ve en la primera
pantalla). En el navegador enlazaban a la portada (la flecha «←», el logo, «Back to Home»), el logout
aterrizaba en ella y las públicas llevan el menú con Precios; `/shell/` enlazaba al panel, y entrar
sin `?next=` lleva al panel por el enrutador post-login. Dentro de la app eso era la web comercial
con los precios y el aviso de pre-lanzamiento, y un panel con Facturación: una puerta de compra fuera
de la tienda (la misma política que hub#479/hub#756 cierran en el hub).

- **La señal es el puente del propio shell**, no una marca nueva: Tauri inyecta
  `__TAURI_INTERNALS__` (y `__TAURI__`, `withGlobalTauri`) al **inicio de cada documento**, también
  en `erplora.com` (wry: `addDocumentStartJavaScript` en todo WebView moderno). Un parcial
  (`templates/partials/app_shell_head.html`) lo mira en el `<head>` de las **dos** bases del SaaS
  —`base.html` y `public/public_base.html`— y pone `html.in-app-shell` antes del primer pintado.
  Si el puente llega tarde (WebView sin scripts de inicio de documento: `evaluateJavascript` en
  `onPageStarted`, sin orden garantizado), vuelve a mirar en `DOMContentLoaded` y en `load`. Sin ida
  y vuelta, sin cookie y **sin build nueva**: las apps ya instaladas lo llevan.
- **Tres marcas.** `data-web-only` se esconde dentro de la app (la flecha, el menú, el footer y el
  aviso de pre-lanzamiento de las públicas). `data-app-only` se esconde en el navegador, y un campo
  de formulario deshabilitado con esa marca se activa dentro de la app: así el logout aterriza en
  `/account/login/?next=/shell/` y el login sin `next` vuelve a `/shell/`, sin cambiar lo que hace el
  navegador. `data-app-back` vuelve atrás en el historial (el «←» que ganan las públicas en la app).
  Los enlaces dentro del texto legal pasan a texto plano en la app.
- **Corta por «dentro de la app», no por canal.** A diferencia de `planUpgradeIsOfferable`/
  `managementIsOfferable` (que sí distinguen Play), aquí se esconde en toda instalación: en
  escritorio tampoco hay botón atrás, y la salida gestionada a la cuenta es la del hub, que abre el
  navegador del sistema.
- ⚠️ **Contrato con el shell**: el SaaS depende de que la ventana inyecte el puente en su origen.
  Tauri lo hace en toda página que carga (el ACL de `remote.urls` solo decide qué `invoke` se
  aceptan, no si el puente existe); cambiar la ventana a algo que no sea Tauri devolvería las
  puertas.
- **Guardarraíl** (`tests/unit/test_app_shell_hides_web_doors.py`, saas). **Mira:** cada `href` de
  todas las plantillas de `templates/account`, `templates/socialaccount`, `templates/mfa` y las de
  `/shell/`, pintadas **con** el banner de cookies; cada `<form>` de esas páginas, enviado como lo
  enviaría la app, juzgado por **dónde aterriza** su redirección; y cada página pública permitida,
  renderizada y sometida a la misma regla (más que cargue la señal y ofrezca el «←»). En Chromium
  real, servido con la cabecera CSP real (y con `style-src` sin `'unsafe-inline'`), comprueba que
  con el puente —también tardío— se esconde lo marcado y sin él no. **No mira:** navegaciones que
  construye el JavaScript (los interruptores de la página de cookies se comprueban aparte, por
  nombre), formularios que postean a su propia página, ni adónde vuelve Google tras su OAuth (que
  además no funciona en una WebView embebida).

## El deep link `erplora://` y su fallback (ADR-0225, hub#345)

Es lo que hace que un enlace del SaaS —«Open Terminal», el correo de invitación— **abra la app** en
vez del navegador (ADR-0196 §7). Tiene dos mitades y la segunda es la que duele: el navegador **no
puede saber** si la app está instalada. Pide el esquema al sistema y, si nadie lo atiende, no pasa
absolutamente nada: sin error, sin evento y sin nada que el usuario pueda interpretar. Un enlace sin
fallback es una página que no reacciona.

### El contrato (esto es lo que construye el SaaS — saas#1147)

```
erplora://hub/<host>
```

- **`<host>` es el HOST COMPLETO del hub** (`demo.a.erplora.com`), no el slug suelto. El ADR-0196
  escribió `<slug>`, pero el slug **no basta**: los hubs viven en `{slug}.{aura}.erplora.com` y la
  app no conoce el aura, así que con `demo` a secas no hay URL que construir. El SaaS ya tiene el
  host (`Server.get_hub_url`); mandarlo entero es la única forma de que el enlace sea autosuficiente.
- **Sin path, sin query, sin fragmento.** Lo que venga detrás se ignora **y** el enlace se rechaza si
  hay más de un segmento (ver más abajo). El destino se reconstruye **solo** desde el host, así que
  ningún parámetro puede colar una segunda URL.
- **La app navega a `https://<host>/?shell=1`** — el `?shell=1` es el contrato de captura de
  ADR-0159 ya existente: es lo que hace que el shell **recuerde** ese hub como `hub.url`. Sin él, el
  enlace funcionaría una vez y se olvidaría en el arranque siguiente. No hay persistencia nueva.
- **Fallback del navegador**: `https://<host>/` (el hub, que en navegador es plenamente usable) o la
  página de descarga cuando la app es **obligatoria** — un tique térmico no se imprime desde un
  navegador (ADR-0196 §5).

### La frontera de confianza: el enlace se RESUELVE, no se abre

Un enlace es lo único que deja que una página **de fuera** dirija la app, y la ventana a la que la
dirige es la que puede abrir el cajón (ADR-0221). Por eso `resolve_deep_link` no abre lo que el
enlace dice: lo **resuelve**, y solo resuelve un hub nuestro.

- El destino admisible es **el mismo** que autoriza el hardware: `*.erplora.com` (≥1 etiqueta) o
  loopback en desarrollo. Que las dos fronteras coincidan importa en las dos direcciones: un destino
  que las capabilities no confían no puede alcanzarse por enlace, y un destino que sí confían no
  puede quedar fuera — abriría un TPV que parece bien y no imprime.
- Las **auras van por comodín y no se enumeran** (letra en Hetzner, número en el fallback AWS):
  enumerarlas convertiría «abrir un aura nueva» en «publicar una app nueva a cada TPV».
- **El ápice queda fuera.** `erplora.com` sirve marketing, billing y un checkout de terceros; ya está
  excluido a propósito de la capability de hardware y tampoco es alcanzable por enlace.
- El host se lee **en crudo**: nada de percent-decoding, solo etiquetas DNS ASCII en minúscula. Sin
  eso, `demo%2Ea%2Eerplora%2Ecom` o un homógrafo cirílico volverían a abrir todos los caminos.
- **Una gramática que la app no entiende se deja al fallback.** Si mañana el enlace crece una página
  (`erplora://hub/<host>/pos/table/4`), una app anterior **rechaza** en vez de entenderlo a medias:
  abrir la home descartaría en silencio la página pedida, mientras que rechazar devuelve el clic al
  navegador, que sí conoce la URL https completa y aterriza donde tocaba.

### El fallback, los tres finales

`apps/web/src/lib/deep-link.ts` es la implementación **canónica** (el SaaS repite el mismo algoritmo;
un helper, no N copias):

1. **App instalada** → el sistema la trae delante; la página pierde el foco (escritorio) o se oculta
   (móvil) y el fallback **se cancela**. Si no se cancelase, el usuario volvería de la app y se
   encontraría el navegador también navegado.
2. **App no instalada** → no pasa nada, así que a los **800 ms** se navega al fallback. Es el caso
   por el que existe todo esto.
3. **Navegador que bloquea el esquema** → la asignación lanza; esperar 800 ms a algo que ya falló
   solo retrasa el único final útil, así que se cae al fallback en el acto.

Detalles que no son cosméticos:

- **`location.replace`, no `assign`**: con entrada de historial, «Atrás» vuelve a la lanzadera y
  vuelve a disparar el enlace — un bucle sin salida por el único botón del que el usuario se fía.
- **El estado de la página al DISPARAR el temporizador es la última palabra** (`document.hidden`): en
  una pestaña de fondo los temporizadores van estrangulados y el fallback puede saltar mucho después
  de que la app ya se llevara al usuario.
- ⚠️ **La página lanzadora DEBE pintar además un enlace visible al fallback.** «Se abrió la app» es
  una heurística —un alt-tab del usuario la dispara igual—, así que el fallback automático no puede
  ser la única salida.

### El cuarto final: no hay a dónde ir ([ADR-0250](../../00-overview/decision-log.md#adr-0250), saas#1233)

Los tres finales de arriba suponen que **existe un destino**. Un hub sin aura asignada no tiene
dirección ninguna (`Hub.assigned_access_url` contesta `""`, ADR-0245), y la lanzadera leía en su
lugar la **conjetura** de `Hub.access_url`. Resultado: dos callejones encadenados — un
`erplora://hub/{slug}.erplora.com` que la app **acepta** (valida la forma, no la existencia) y
resuelve a nada, y detrás un fallback al mismo host inexistente.

**El fallback no ayuda aquí: empeora.** Su trabajo es cazar el silencio de la app; apuntado a un
host que no resuelve, sustituye una página que podía explicar por el error de DNS del navegador,
que es el final **menos legible** de todos. Así que sin dirección **no se emite ninguno de los
dos** y el `<script>` de la lanzadera **no se renderiza**.

**La lanzadera deja de lanzar y pasa a explicar.** De las cuatro salidas posibles para un botón que
no puede hacer su trabajo —esconderlo, deshabilitarlo, 404, o llevar a una pantalla que lo cuente—
solo la última deja al dueño sabiendo más que antes de pulsar:

| Salida | Por qué no |
|--------|------------|
| Esconder el botón | La ausencia tampoco se explica: «he pagado, ¿dónde está mi terminal?» |
| Deshabilitarlo | No hay dónde poner el motivo: es un icono de un menú de fila y en la tablet del TPV no hay *hover* que revele un tooltip |
| 404 | Falso —el hub existe y es suyo— y se lee como «el producto está averiado», justo lo que ADR-0245 se negó a mandar |
| **Pantalla que lo cuenta** | ✅ El botón **se queda** y la página de destino responde |

**Y distingue las dos causas, porque el SaaS puede distinguirlas.** `deployment_status ==
"deploying"` es una promesa de que algo sigue corriendo y terminará: se dice que espere y que le
llegará un correo. **Cualquier otro** estado sin dirección (`error`, y también un `active` que
reportó éxito y no dejó nada que abrir) significa que **nadie** la está preparando, y ahí la salida
es una persona (`/dashboard/help/support/`), no un temporizador. Decirle «unos minutos» a ese dueño
compra silencio ahora y una llamada mucho peor después. El *spinner* sigue esa misma línea: solo lo
pinta el estado en el que algo se está moviendo de verdad.

### Registro: un esquema que nadie registró es un enlace muerto

Y falla en silencio: el usuario pulsa y no pasa nada. Cada plataforma lo lee de un sitio distinto,
así que hay un test por sitio (`src-tauri/tests/deep_link.rs`):

| Plataforma | Dónde se declara | Quién lo escribe |
| --- | --- | --- |
| Windows · Linux | `plugins.deep-link.desktop.schemes` de `tauri.conf.json` | el **bundler**, al instalar (clave del registro / handler `.desktop`) |
| macOS · iOS | `CFBundleURLTypes` del bundle | el bundler, desde la misma clave del config |
| Android | `<intent-filter>` con `VIEW` + `BROWSABLE` + `<data android:scheme>` | `gen/android/…/AndroidManifest.xml` (a mano: `autoVerify` es solo para App Links `https`) |
| Android (**otra vez**) | `plugins.deep-link.mobile` de `tauri.conf.json` | el propio plugin, que si no **tira el enlace** |

⚠️ **Android lo declara DOS veces, y hace falta.** El intent-filter solo lleva la URL hasta el
plugin; el plugin la tira ahí. Leído en `tauri-plugin-deep-link` 2.4.9 (`DeepLinkPlugin.kt`):
`isDeepLink` devuelve **`false` de entrada** si `plugins.deep-link.mobile` está vacío, y si no, solo
casa el `scheme`/`host` configurados. Con `desktop.schemes` a secas Android nos entrega el intent y
el plugin lo descarta — **en silencio y solo en Android**, que es donde más cuesta verlo. De ahí
`"mobile": [{ "scheme": ["erplora"], "host": "hub" }]`: `host` es la **acción** de la gramática, así
que nombrar otra filtraría justo los enlaces que emitimos.

Y la entrega, que es otra cosa que el registro:

- **`tauri-plugin-single-instance` (solo escritorio)**: Windows y Linux contestan a un enlace
  **lanzando el ejecutable otra vez** con la URL como argumento. Sin esto, pulsar un enlace con la
  app abierta arrancaría un **segundo TPV encima del primero** (dos ventanas, dos colas de impresión,
  dos watchdogs). El cfg va escrito por-SO y **no** como `cfg(desktop)`: ese alias lo emite
  `tauri-build` para rustc y Cargo no lo conoce al resolver dependencias — descartaría el crate en
  silencio.
- **Arranque en frío** → la URL llega por argumentos (`deep_link_from_args` → `cold_start_link`).
  Si lleva al hub **enlazado** gana a todo (`initial_url_for`) y no corre el chequeo de vida del
  recordado, que termina navegando al onboarding y se llevaría por delante lo que se acaba de pedir.
  Si lleva a **otro** hub (o no hay ninguno enlazado), la ventana arranca donde lo haría sin enlace
  —el recordado, con su chequeo, o el SaaS— y, con la ventana ya abierta, la app pregunta
  (`open_hub_from_link`, abajo). Pulsar «Open Terminal» del hub B en un TPV del A sigue llevando al B,
  pero tras «Abrir»: antes de hub#2644 lo hacía sin preguntar, y lo mismo hacía cualquier correo o
  página que mandase el enlace.

### Un enlace a otro negocio pregunta antes (hub#2644)

Un enlace `erplora://hub/…` lo puede mandar cualquiera (un correo, una web, un mensaje), y enlazar su
destino le da el hardware del TPV y lo convierte en el negocio con el que arranca la app. Por eso
`open_hub_from_link` solo navega directo al hub **ya enlazado**; a cualquier otro (también en una
instalación sin enlazar) le pregunta a la persona con un **diálogo del sistema**, nunca de la página
—la página que enseña la ventana puede ser la que mandó el enlace—: «Open <host> on this device?» /
«¿Abrir <host> en este dispositivo?», con lo que se entrega (impresora, cajón, lector de tarjetas;
arranque) y «Abrir»/«Cancelar». Solo «Abrir» enlaza y navega; cualquier otra salida —cancelar,
cerrar, Atrás, un diálogo que no se puede enseñar, un error del plugin— deja el TPV como estaba.

- **Quién pregunta** es estado gestionado (`link_consent::AskBeforeLinking`), registrado en `run()`
  **antes** de crear la ventana para que el arranque en frío ya lo tenga. Sin él, el enlace se
  rechaza (falla cerrado). Los tests ponen ahí una persona con guion sobre `MockRuntime`.
- **Escritorio**: `rfd::AsyncMessageDialog` (nivel aviso, hijo de la ventana `main`), mostrado desde
  el hilo principal y esperado en otro. **No** `tauri-plugin-dialog`: su `init` sustituye
  `window.alert`/`window.confirm` en **todas** las páginas por órdenes que las páginas del hub no
  tienen concedidas, y su `confirm()` volvería como promesa (verdadera). El idioma sale de
  `sys-locale`: español salvo que el sistema diga otro (la regla de la página sin conexión).
  «Cancelar» va **primero**, que es el botón por defecto (Intro en macOS y Windows): el lector de
  códigos de un TPV teclea un Intro tras cada lectura. Los tres sistemas contestan con la etiqueta
  pulsada y solo la de «Abrir» abre.
- **Android**: `askToOpenHub` del plugin propio (`AlertDialog`), llamado solo desde Rust
  (`ErploraAndroid::ask_to_open_hub`, bloqueante, en un hilo propio). No está en `build.rs` ni en los
  permisos por defecto, así que ninguna página puede invocarla (y la puerta de hub#2642 la cerraría
  igual). Viajan las dos lenguas y Kotlin elige la del dispositivo (`HubQuestion`); faltan palabras →
  rechaza, y un rechazo es un no.
- **iOS** (no se construye): no hay diálogo, y un enlace a otro hub es un no.
- **App ya abierta** → `on_open_url` (evento en macOS/iOS/Android; reinyectado por single-instance en
  escritorio) → `handle_deep_link`. Un enlace que no resuelve **no hace nada**: el fallback es del
  navegador que lo lanzó, y navegar «a algo» sería peor que quedarse quieto.
- **`erplora://notice?id=&path=` no es un deep link de navegación** (hub#2409): es el enlace con el
  que un aviso de Windows relanza la app. `on_second_launch` y el arranque en frío lo contestan
  **antes** que `handle_deep_link` (`notice_tap::answer_link`, que guarda el toque para que la página
  lo reclame), `on_open_url` lo ignora y `resolve_deep_link` lo rechaza como cualquier otra
  gramática que no sea `hub`.

## Android SÍ va por Tauri (ADR-0180)

> Este apartado decía lo contrario hasta el 2026-08-01. Se invierte con
> **[ADR-0180](../../adr/ADR-0180.md)**.

Android es **la misma app Tauri**, no una app aparte. La premisa que sostenía la app Kotlin —que
Tauri-Android perdía el **Bluetooth SPP** y el **foreground service**— se cae con el alcance real
del producto (un webview que imprime): el SPP **se retira** por decisión de producto (las
impresoras de red cubren el norte) y el FGS es innecesario con el hardware in-process.

> ⭐ **Enmienda ADR-0204 (ejecutada el 2026-08-13, PR hub#922)**: la retirada del SPP se
> revirtió PARCIALMENTE — vuelve **solo en Android**, como transporte del plugin
> (`BluetoothSpp.kt`, `printer_id: bluetooth:{mac}`), sin FGS y sin `BLUETOOTH_SCAN` (emparejar
> sigue siendo de los ajustes de Android). Detalle en
> [`peripherals.md`](../crates/peripherals.md) §2.7; validación en hardware real: hub#924.
> Del mismo lote: ajuste **«Arrancar al iniciar sesión»** (`tauri-plugin-autostart`, solo
> escritorio, OFF por defecto, estado leído siempre del SO — hub#389) y salida de usuario
> **«Cambiar de negocio»** (`forget_hub` con `choose`, navega a `/shell/?choose=1`; el camino
> 410 queda intacto — hub#447; desde hub#2503 los dos cierran antes la sesión).

Medido antes de decidir, en emulador API 37 y contra hardware real:

| Riesgo | Resultado |
|---|---|
| ¿llega el `invoke` a una página REMOTA? | **sí**, en `erplora.com` y en `{slug}.a.erplora.com` |
| ¿el ACL cubre `{slug}.a.erplora.com`? | **sí** — `https://*.erplora.com/*` cruza las dos etiquetas |
| descubrimiento + ESC/POS + notificación | **funcionan** end-to-end |

**Los permisos de runtime son parte del producto**, no empaquetado: sin `ACCESS_LOCAL_NETWORK`
(API 37+) el descubrimiento devuelve `[]` —indistinguible de «este local no tiene impresoras»— y
sin `POST_NOTIFICATIONS` (API 33+) la comanda entra en cocina sin que nadie se entere. **Los dos
fallan en silencio.** El plugin `crates/tauri-plugin-erplora-android` sabe pedir los dos
(`checkPermissions`/`requestPermissions`, alias por permiso) y nunca al arrancar — pero **el plugin
no pide nada por su cuenta: pide lo que alguien le pida**, y ahí estaba el agujero. Ese «alguien»
solo puede ser la página del hub enlazado ya cargada: otra página de erplora.com recibe
`not_the_linked_hub` sin que Android enseñe nada (hub#2642, «La segunda puerta»).

**Quién los pide de verdad, as-built (hub#1732, hub#1773):**

| Permiso | Quién lo pide | Cuándo | ¿Frase nuestra delante? |
|---|---|---|---|
| `POST_NOTIFICATIONS` | el **shell** (`apps/web/src/lib/notification-permission.ts`) | en los dos momentos en que hay alguien delante: al darse de alta este equipo como **puesto de impresión** (`bootPrintHost` → `onRegistered`) y al **abrirse una sesión** en él (login, o arranque con la sesión ya abierta; `askWhenSomeoneSignsIn`, hub#2306) — **solo si el hub tiene activo un módulo del que el shell avisa** (`NOTICE_SOURCE_MODULES`, hoy `kitchen` y `appointments`, hub#2046/#2168, o cualquiera con contador en la campana, hub#2306) y, de reserva, antes del **primer aviso** (comanda, cita o contador) | **sí** — hoja propia, y la respuesta se apunta ANTES de abrir el diálogo del sistema |
| `ACCESS_LOCAL_NETWORK` | lo dispara el **`module-sdk`** (`IpcBridgeTransport.ensurePermissions()`), pero la hoja la pone el **shell** (`apps/web/src/lib/local-network-permission.ts`) | al descubrir, imprimir, sacar la hoja de prueba o abrir el cajón — cualquier operación que use la LAN | **sí** (hub#1773) — hoja propia, y la respuesta se apunta ANTES de abrir el diálogo del sistema |

Hasta hub#1732, el **único** llamador de la vía de notificaciones era el aviso de comanda, así que
un equipo que no fuera cocina no preguntaba **jamás**: Android da por `denied` un permiso que nunca
mostró, y el sistema **no distingue** «denegado» de «sin preguntar» mientras no haya `USER_SET`.
Por eso el alta como puesto de impresión es un disparo —es el momento en que este equipo pasa a ser
el que **recibe** el aviso, y hay alguien delante configurándolo—; desde hub#2306 lo es también el
**inicio de sesión** (un KDS o una tablet de WhatsApp sin impresora no se dan de alta), y el primer
aviso queda solo de reserva. Si la respuesta es que no,
el shell **corta** el aviso antes de llamar a `sendSystemNotice()`: esa llamada pediría el permiso
ella sola, con el diálogo pelado de Android en medio de un servicio. Lo que ve el usuario en su
lugar es la fila de **Sistema › tu impresora**, que dice que los avisos están apagados y ofrece
volver a pedirlo o abrir los Ajustes.

**Solo se pide si hay algo que avisar (hub#2046).** Hasta hub#2168 el único aviso que mandaba el
shell era el de comanda, así que en un salón (sin `kitchen`) el alta pedía un permiso que nunca se
usaba.
`warnIfThereIsSomethingToTell` refresca los módulos activos y solo pregunta si alguno está en
`NOTICE_SOURCE_MODULES` o pone un contador en la campana; sin saber qué hay instalado, no pregunta. La tarjeta de «avisos apagados»
de **Sistema** sigue la misma regla. Añadir un aviso nuevo con nombre propio es añadir su módulo a esa lista; un contador de campana no
necesita nada (hub#2306, abajo).

**Avisos de citas (hub#2168).** `apps/web/src/lib/appointment-notice.ts`, gemelo del de comanda,
escucha `appointments.appointment.created` y `.cancelled` y lanza un aviso del sistema **solo si el
marco no lleva `clientInstance`**: lo que tecleó un mostrador (este u otro) ya lo sabe quien está
en él; el aviso es para la reserva por WhatsApp, la web, un flujo o la cancelación del cliente. El
texto sale del catálogo (`appointmentNotice.*`, `en` + `es`) con el día y la hora que ya resuelve
`appointments.appointments.get` en la zona del negocio (appointments#151); si la cita no se puede
leer, avisa con lo que trae el evento. Misma puerta de permiso que la comanda (`askToWarn` +
`shouldSendNotice`), así que `appointments` entra en `NOTICE_SOURCE_MODULES`.

**Avisos de la campana (hub#2303).** `apps/web/src/lib/bell-notice.ts` lanza un aviso del sistema
cada vez que **sube** un contador del bloque `bell` de cualquier módulo (cliente de WhatsApp
esperando, …), con la etiqueta del propio módulo y el total; misma puerta de permiso. `appointments`
queda fuera porque su reserva ya avisa arriba. Detalle del disparo en
[`../notification-bell.md`](../notification-bell.md).

**Un módulo con contador en la campana también cuenta como algo que avisar (hub#2306).** Un negocio
que solo usa WhatsApp no tenía nada en `NOTICE_SOURCE_MODULES`: ni se le preguntaba al configurar el
equipo ni Sistema le decía que los avisos estaban apagados, y la primera pregunta de Android salía
con el primer cliente esperando. Ahora cuenta todo módulo **activo** que declare un contador que la
campana ejecutaría (con etiqueta y query propia: `bellCounterModuleIds` en `bell-counters.ts`, la
misma regla que el sondeo), sin que el núcleo nombre ningún módulo. Y como esa tablet no suele tener
impresora —así que nunca se da de alta como puesto de impresión—, la pregunta se hace también **al
abrirse una sesión en el equipo** (`askWhenSomeoneSignsIn` en `main.ts`, tras montar la app): alguien
acaba de teclear su PIN delante. `ensureNotificationPermission` sigue preguntando como mucho una vez
por instalación, así que los inicios de sesión siguientes cuestan una lectura de almacenamiento.
Y como el alta y el inicio de sesión pueden llegar **a la vez** (arranque con sesión abierta en un
equipo con impresora), la hoja es **una sola en vuelo** por permiso (`ensureDevicePermission` en
`device-permission.ts`): el que llega tarde se une a la hoja abierta en vez de apilar otra, que con
un segundo «Permitir» gastaría el último diálogo de Android.

**Tocar un aviso abre su pantalla (hub#2305).** Los tres avisos del shell (comanda, citas,
campana) salen por una sola puerta, `apps/web/src/lib/notice-tap.ts` (`createNoticeDoor`, cableada
en `main.ts`), que da a cada aviso un **id** propio, lo manda con `sendSystemNotice` →
`erplora_notify(title, body, id)` y recuerda en memoria (los 50 últimos) a qué pantalla lleva. El
toque vuelve del plugin de notificaciones (`actionPerformed`, con `notification.id`) y la puerta
navega con el router. Destinos: comanda → `/m/kitchen`, citas → `/m/appointments` (la raíz del
módulo, que abre su primera pestaña y sobrevive a un cambio de nombre de pestaña), campana → la
pestaña del contador (`rise.path`). Solo se sigue una ruta que sea pantalla del shell, con la regla
**anclada** de los enlaces del asistente (`SCREEN_PATH`, hub#2204).

- **Por qué un id y no `extra`**: iOS no devuelve el `extra` con el toque; el id vuelve en Android y
  en iOS por igual. El plugin lo guarda como `i32`: un id fuera de rango sale sin id (el aviso se ve
  igual, solo pierde el destino).
- **ACL**: la PWA remota se suscribe con `plugin:notification|register_listener`, así que
  `capabilities/default.json` concede **solo** `notification:allow-register-listener` (los avisos
  siguen saliendo por `erplora_notify`); `onboarding.json` y `degraded.json` no tienen ningún
  permiso `notification:*`. Lo fija `tests/notice_tap.rs`. Es un plugin de terceros, así que se
  registra envuelto (`hub_link::notification_plugin`, hub#2658): solo el hub enlazado se suscribe.
- **Contrato del kernel intacto**: `peripherals.notify(title, body)` del SDK de módulos no cambia —
  ningún módulo lo llama hoy—; la puerta con destino es del shell.
- **Compatibilidad**: una app instalada anterior ignora el `id` y no concede la escucha → el aviso
  sale igual y el toque trae la app al frente, como antes.
- **Escritorio y Android en frío (hub#2360)**: el plugin de notificaciones no reporta clics en
  escritorio y, en Android con la app cerrada, dispara el toque antes de que la página escuche. Por
  eso el **shell guarda el toque** y la página lo **reclama**: en escritorio `erplora_notify` no pasa
  por el plugin, el shell muestra el aviso él mismo (`src/notice_tap.rs`: `mac-notification-sys`
  en macOS, `notify-rust` en Linux, WinRT directo en Windows —ver la viñeta siguiente—, en un hilo
  propio) y, al hacer clic, guarda `{id, path}`
  (`KeptNoticeTap`, gana el último), trae la ventana al frente y emite `erplora://notice-tapped`; en
  Android el plugin propio (`NoticeLaunch.kt`) guarda el toque que **arrancó** la app (solo un toque
  en el cuerpo del aviso) y lo entrega una vez con `takeNoticeTap`. En frío hay dos caminos: sin
  tarea, el intent con el que se crea la actividad (`load`); con el proceso muerto por el sistema y
  la tarea en recientes, la actividad vuelve con su intent de lanzador y el toque entra por
  `MainActivity.onNewIntent` (`NoticeTaps.kt`). Para no repetir un toque viejo al volver por el
  icono a una tarea que nació de uno (Android devuelve aquel intent con sus extras en cada vuelta
  del proceso), el toque del que nació la tarea —id y contenido— se recuerda en las preferencias de
  la app (`com.erplora.notice_taps`). El que entra por `onNewIntent` no se recuerda: Android lo
  entrega una sola vez, y el intent de origen, que `load` lee después, no lo pisa. La página llama
  a `erplora_take_notice_tap` al arrancar y en cada `erplora://notice-tapped` (`claimNoticeTaps`),
  permiso `allow-erplora-take-notice-tap` en `default.json`. Como la página que reclama puede ser **nueva** (su mapa en memoria está vacío),
  el aviso lleva además la ruta en `extra.path`, que solo se sigue si pasa `SCREEN_PATH`; el id
  sigue siendo lo primero (iOS no devuelve el `extra`). iOS no cambia.
- **Windows: el aviso lleva su propio enlace (hub#2409)**. `notify-rust` espera el clic con **un
  solo** `recv()` del hilo que mostró el aviso: cuando el aviso caduca en pantalla y pasa al Centro
  de actividades, ese hilo ya ha terminado y un clic posterior no tiene quién lo reciba (y con la
  app cerrada, tampoco proceso). Por eso en Windows el shell crea el toast por WinRT (crate
  `windows`) con `launch="erplora://notice?id=<id>&path=<ruta>"` y `activationType="protocol"`
  (`notice_link` + `toast_xml`): el clic, venga de donde venga, **lanza la app con ese enlace**. Con
  la app abierta, `tauri-plugin-single-instance` lo reenvía a `on_second_launch`; en frío llega por
  argumentos y `setup` lo contesta tras abrir la ventana. Los dos caminos acaban en
  `notice_tap::answer_link` → el mismo `on_click` que el resto del escritorio (guarda el toque, trae
  la ventana, emite `erplora://notice-tapped`). El `deliver` de Windows devuelve siempre «sin clic»:
  no espera nada. Identidad del aviso: la copia de la Store usa la suya (`CreateToastNotifier()`),
  la instalada por `.exe`/`.msi` el `identifier` de su acceso del menú Inicio, y una build de
  `target/` la de PowerShell (misma regla que el plugin). El enlace es **entrada no confiable**:
  `click_from_args` solo conserva una ruta propia (empieza por `/` y no por `//`) y la página vuelve
  a pasarla por `SCREEN_PATH`. El MSIX declara el protocolo `erplora` en su manifiesto
  (`msix/Package.appxmanifest`), que la copia de la Store no tenía. No verificado aún en un Windows
  real (el banco es `qa-hub-windows`).

**Avisos con la pantalla apagada (hub#2307).** Los tres avisos nacen en la página, así que en
Android solo llegaban con la app a la vista: con la pantalla apagada el sistema congela el proceso y,
aunque siga vivo, Chromium congela la página oculta al minuto. Mientras haya sesión, los avisos no
estén rechazados y el hub tenga algo que avise (`hasNoticeSource`), el shell pide
`plugin:erplora-android|keep_listening` (`apps/web/src/lib/notice-listening.ts`, tras la pregunta de
avisos del inicio de sesión; `on: false` al cerrarla; de nuevo al reactivarlos en Sistema). El
plugin arranca `NoticeListeningService`, un servicio en primer plano `specialUse` con notificación
fija y silenciosa cuyas palabras manda la página (`system.notices.listening*`, `en` + `es`), y
`PageKeeper` deshace el ocultado mientras escucha: `WebView.onResume()` tras `onPause`/`onStop` y
`dispatchWindowVisibilityChanged(VISIBLE)` cuando la ventana deja de verse (lo oye una vista
centinela sin tamaño en el `decorView`: el WebView lo crea wry y `OnWindowVisibilityChangeListener`
pide API 34). Sin push (decisión del 30/09, hub#2307): medido con sesión real, pantalla apagada y
Doze forzado, 4/4 comandas avisadas a la escucha frente a 0/4 sin ella. Decisión:
[En Android, los avisos con la pantalla apagada llegan porque la app se queda a la escucha, sin push](../../adr/2026-09-30-android-escucha-avisos-con-la-pantalla-apagada.md).

**Y el de red local sale por la misma puerta desde hub#1773.** Aquí el llamador no es el shell sino
el `module-sdk`, que pide el permiso **dentro** de `ensurePermissions()` justo antes de la
operación: envolver `discoverPrinters()` desde fuera no sirve de nada —la hoja se pinta y el
diálogo pelado sale detrás igual—. El punto donde el shell sí manda es el **adaptador `TauriBridge`
que él mismo le pasa al SDK** (`apps/web/src/lib/bridge-transport.ts`): ahí se intercepta
`plugin:erplora-android|request_permissions`, se saca `ACCESS_LOCAL_NETWORK` del lote para que pase
por su hoja, y si el dueño dice que sí el lote **entero** (`BLUETOOTH_CONNECT` incluido, ADR-0204)
sale en **una sola** llamada: en Android 17 los dos son «dispositivos cercanos» y el sistema los
redacta igual, así que pedirlos por separado eran dos diálogos idénticos seguidos (hub#1923). El
resto del lote sigue la suerte de la red local: tras un «no» —a nuestra hoja o al diálogo— no se
pide Bluetooth en frío; donde no existe el permiso de red local (API < 37) se pide como antes. Y
las peticiones que llevan la red local van **en cola**: la pantalla escanea al abrir y el botón
llega a la vez, y sin cola las dos leían «aún sin responder» y la hoja se apilaba. Una sola
regla cubre escaneo, impresión, hoja de prueba y cajón, y **no mueve `contracts/kernel/sdk.d.ts`**,
así que no arrastra el espejo del `module-toolkit`.

La mecánica de las dos —leer el estado real, explicarse, y solo entonces dejar preguntar a Android,
una vez— vive en `apps/web/src/lib/device-permission.ts`; cada permiso aporta su identidad: cuál es
y **dónde se apunta su respuesta**. Las memorias son independientes a propósito: una sola haría que
el «ahora no» de los avisos apagase la hoja de la impresora para el resto de la vida de la
instalación. Y la vuelta atrás también es la fila de **Sistema › tu impresora**: cuando el permiso
está denegado dice que la búsqueda está bloqueada y ofrece volver a pedirlo (`force`); si Android ya
no pregunta —tras dos negativas deja de mostrar el diálogo—, la única puerta que queda son los
Ajustes del dispositivo, y las dos tarjetas (avisos e impresora) llevan un botón **«Abrir los
ajustes»** que abre directamente la ficha de ERPlora ahí (hub#1886): comando
`plugin:erplora-android|open_app_settings`, un `ACTION_APPLICATION_DETAILS_SETTINGS` con el
paquete de la app, concedido en `erplora-android:default`. La web va con el hub y el APK con la
tienda, así que una app instalada anterior al comando responde «command not found»: entonces el
botón dice con palabras dónde ir, nunca se queda mudo. Al volver de los Ajustes no hay navegación;
el estado se relee con `visibilitychange`. El escaneo, por su parte, nunca devuelve `[]` mudo desde hub#338:
rechaza con el motivo, y el shell le pone la frase en el idioma del hub.

### Los permisos se declaran DOS veces, y hace falta (ADR-0241, hub#337)

`gen/android` es un proyecto **generado**, y ahí vivía la única declaración. Medido con
tauri-cli 2.11.2 en este árbol —no leído en un changelog—: `cargo tauri android init` **respeta** el
`AndroidManifest.xml` que ya existe, pero si **falta** escribe la plantilla de Tauri —`INTERNET` y
nada más—, sin los cuatro permisos de hardware y sin el intent-filter de `erplora://`. O sea: el
riesgo no es «alguien reejecuta init», es «el fichero no está cuando init corre» (un `rm -rf
gen/android` para desatascar Gradle, un merge que se lo lleva). Y la pérdida es **muda**: el APK
compila, instala y arranca; solo dejan de existir las impresoras.

La copia duradera vive por eso **fuera del proyecto generado**, en el módulo Android del plugin
(`crates/tauri-plugin-erplora-android/android/src/main/AndroidManifest.xml`): el **manifest merger**
de AGP funde los `<uses-permission>` de una librería en la app al construir, y `crates/` no lo toca
ningún generador. Verificado en un APK real: con el manifest de la app **reducido a `INTERNET`**,
el manifest fusionado sigue declarando los cinco permisos, y el informe del merger los marca
`MERGED from [:tauri-plugin-erplora-android]`.

| Dónde | Fichero | Qué aporta |
|---|---|---|
| Proyecto generado | `gen/android/app/src/main/AndroidManifest.xml` | lo que lee primero quien abre el proyecto |
| Módulo del plugin | `crates/tauri-plugin-erplora-android/android/src/main/AndroidManifest.xml` | la copia que **sobrevive** a regenerar `gen/android` |

Es el mismo patrón que el deep link (§*Registro*): en Android hace falta declararlo dos veces, y
la segunda es la que salva. **Tres guardas** lo sostienen, y ninguna es de texto plano —leen los
`<uses-permission>` fuera de comentarios, porque estos manifests **explican en prosa** cada permiso
que declaran—: `apps/tauri/src-tauri/tests/android_local_network.rs` (las dos copias),
`crates/tauri-plugin-erplora-android/src/lib.rs` (lo que la política **pide** está **declarado** —
pedir un permiso no declarado se responde DENEGADO al instante, sin diálogo y para siempre) y el
job de release, que lee los permisos **del APK construido** con `aapt2 dump permissions`, que es el
único sitio donde el manifest fusionado existe de verdad.

`printer_id` tiene **tres** variantes, y cada una vive donde puede: `network:{ip}:{port}` en todas
las plataformas, `bluetooth:{mac}` **solo en Android** (ADR-0204) y `usb:{queue}` **solo en
escritorio** (ADR-0441 — la cola RAW del SO, `lp -o raw`; el driver lo pone el fabricante, no
nosotros). La afirmación «`network:` en **todas** las plataformas (§2.7 red-only)» que vivía aquí
dejó de ser cierta con ADR-0204 y lo es aún menos ahora. En Android no existe `arp`, así que
`device_key` cae al par IP:puerto; en escritorio usa la MAC. Las dos ramas de red están ejercidas
contra una impresora real en `crates/peripherals/tests/hardware_smoke.rs`.

Lo que el shell hace con cada una: la de red **encola** (`PrintQueue`, con reintentos); las otras
dos van **directas** al transporte (fase 1), porque la cola lleva un destino de socket y su política
de reintentos está escrita alrededor de un `connect` TCP. En los tres casos el fallo vuelve al
llamante y el print host marca el trabajo `failed`.

**Lo que NO está hecho:** la validación en **dispositivo real sobre una LAN con impresora** —el
emulador tiene red NAT y no la alcanza— y la actualización OTA por encima del APK Kotlin
instalado. Hasta eso, el repo Kotlin sigue vivo. En USB, lo que falta es **Windows** (hub#1269:
spooler RAW, necesita máquina real) y el **rol** de una cola USB (hub#1536: la clave del registro
se deriva de MAC o `ip:port`, y una cola no tiene ninguna). El descubrimiento y la impresión USB
reales se validan a mano con `cargo run -p erplora-peripherals --example usb_queue_probe`, porque
en CI no hay ninguna impresora enchufada.

### La ventana reserva el hueco de la barra de estado (hub#1719)

`MainActivity` pide **edge-to-edge**, que con `targetSdk = 36` no es una elección: desde Android 15
el sistema coloca la ventana **detrás** de las barras lo pida la app o no. Pedirlo es media parte
del contrato; la otra media —**reservar** la franja que ocupan— faltaba, así que el webview
empezaba en el borde físico de la pantalla y **cualquier** página cargada dentro se pintaba encima
del reloj. QA lo reprodujo dos veces el 2026-09-09 (emulador `Pixel_10_Pro` / Android 16, APK
1.1.21): la flecha de volver de la pantalla de identificarse que sirve el SaaS caía sobre el
«19:45» del sistema, y la capa de búsqueda del TPV se comía la barra entera.

🔴 **`env(safe-area-inset-*)` NO es el arreglo en Android.** En un WebView de Android esos valores
describen el **recorte de la pantalla** (el notch), no las barras del sistema: en un móvil sin
notch valen **cero** por muy bien que la página los use. Por eso el hueco se reserva en la
**ventana** y no en una hoja de estilos — que además es lo único que cubre a todas: la app sirve
páginas del SaaS, del hub y de cualquier módulo instalado, así que un arreglo por plantilla habría
que repetirlo en todas y aun así se le escaparía la siguiente.

Se reserva `max(systemBars().top, displayCutout().top)`: la barra de estado se puede **ocultar** —y
entonces reporta 0— mientras que el recorte está tallado en el panel y no se va nunca; quedarse con
uno solo deja contenido bajo el otro. Los insets se **devuelven sin consumir**, para que la página
de abajo siga recibiendo los de abajo y los del teclado.

El hueco se pone en el **content view de la actividad**, no en el WebView, y la diferencia se ve en
modo oscuro: la franja que queda arriba la pinta el fondo de la vista que lleva el padding, y el
del WebView es un **blanco opaco** que ningún tema toca. Como `enableEdgeToEdge` pone los iconos de
la barra en claro cuando el sistema está en oscuro, padear el WebView cambiaría «reloj tapado por
una flecha» por «reloj blanco sobre blanco». El content view es transparente, así que la franja
enseña el fondo de ventana, que es `DayNight` y gira con el mismo ajuste que los iconos.

Y ese fondo de ventana es **el color de la cabecera del shell**, no el blanco de MaterialComponents
(hub#1903): el tema declara `android:windowBackground = @color/status_bar_strip`, que vale
`#f6f7f9` en `values/` y `#0b0c0e` en `values-night/` — el `--ion-toolbar-background` de
`apps/web/src/theme/variables.css` en claro y en oscuro. El reloj se apoya sobre la cabecera, como
en cualquier app nativa, sin la banda blanca que medía el driver sobre v1.1.26 (`(255,255,255)`
encima de `(246,247,249)`). La guarda compara los dos ficheros, así que la cabecera no puede cambiar
de color sin que la franja la siga; y `test-shell.yml` se dispara también con ese `variables.css`.
Es **estático**: sigue el modo del teléfono, no el modo forzado desde la app, la paleta del negocio
ni un cambio de modo con la app abierta (`configChanges` incluye `uiMode`) — eso lo abre hub#1911.
Ojo al editar esos XML: un comentario XML no admite `--`, y `aapt2` rechaza el fichero en el job de
release, que es el primero que compila `:app`.

Mismo patrón que los permisos: lo duradero vive **fuera del proyecto generado**, en el módulo
Android del plugin (`SystemBarInsets.kt`) — que además **se compila y se prueba en cada PR**,
porque `gen/android/app` no lo compila nadie hasta la release. Lo frágil es la llamada, que sí vive
en el generado; de que siga ahí se ocupa la guarda.

| Dónde | Fichero | Qué aporta |
|---|---|---|
| Módulo del plugin | `crates/tauri-plugin-erplora-android/android/…/SystemBarInsets.kt` | cuánto se reserva, y que una vista ausente sea un aviso en el log y no una caída al arrancar — 6 tests JVM que corren en cada PR |
| Proyecto generado | `gen/android/app/…/MainActivity.kt` | la llamada, en `onCreate` sobre `android.R.id.content` (el WebView todavía no existe ahí: lo crea el lado Rust dentro de `Rust.onActivityCreate`) |
| Tema | `gen/android/app/src/main/res/values{,-night}/{themes,colors}.xml` | el color de la franja: `windowBackground` = `@color/status_bar_strip`, el de la cabecera en claro y en oscuro (hub#1903) |
| Guarda | `apps/tauri/src-tauri/tests/android_status_bar_inset.rs` | que la llamada siga ahí, que el helper cubra el recorte y no consuma, y que la franja lleve el color de la cabecera sin que nada la tape (hub#1903) |

## Referencias

- **Decisión:** [decision-log — ADR-0159](../../adr/ADR-0159.md) (enmienda de
  empaquetado a ADR-0154; retoma «el shell ES el bridge» de ADR-0050).
- **Frontera de orígenes / CSP:** [decision-log — ADR-0221](../../adr/ADR-0221.md)
  (hub#334) · `src-tauri/capabilities/*.json` · `src-tauri/tests/remote_acl.rs` ·
  `src-tauri/tests/shell_csp.rs`.
- **Captura de `hub_url` (qué origen se puede recordar):**
  [decision-log — ADR-0243](../../adr/ADR-0243.md) (hub#335) · `src-tauri/src/lib.rs`
  (`shell_capture_origin`, `trusted_hub_origin`, `load_hub_url`, `is_hub_domain`) ·
  `src-tauri/tests/remote_acl.rs` (`the_shell_only_remembers_origins_that_can_drive_the_till`).
  Abiertas: hub#447 (no hay control para cambiar de hub) · hub#448 (dominio propio sin hardware).
- **Salida al navegador del sistema:** [decision-log — ADR-0255](../../adr/ADR-0255.md)
  (hub#475) · `src-tauri/src/lib.rs` (`external_browser_url`, `open_external_url`) ·
  `src-tauri/tests/external_open.rs` · `src-tauri/tests/remote_acl.rs`
  (`the_hub_origin_can_leave_for_the_browser`, `no_capability_hands_a_page_the_raw_opener_plugin`) ·
  `src-tauri/tests/shell_surface.rs` (`the_external_opener_is_registered_and_checks_before_it_opens`)
  · consumidor: `apps/web/src/lib/open-external.ts`. Abierta: hub#479 (anti-steering de las
  tiendas).
- **Páginas del SaaS dentro de la app (sin puertas a la web pública):** saas#2083 + saas#2085 (sin
  ADR propio: aplica el anti-steering de hub#479 a las páginas del SaaS) · saas
  `templates/partials/app_shell_head.html` (en `base.html` y `public/public_base.html`),
  `templates/partials/app_back_bar.html`, `templates/account/_logout_lands_on_app_start.html` ·
  `tests/unit/test_app_shell_hides_web_doors.py`.
- **Guardar un fichero:** [decision-log — ADR-0259](../../adr/ADR-0259.md) (hub#480) +
  [ADR-0307](../../adr/ADR-0307.md) (hub#499, el camino nativo de Android) ·
  `src-tauri/src/lib.rs` (`save_download`, `save_target`, `publish_to_android_downloads`) ·
  `src-tauri/tests/save_download.rs` ·
  `crates/tauri-plugin-erplora-android/{src/lib.rs,android/…/DownloadPublisher.kt}` (+ sus tests
  JVM) · consumidor: `apps/web/src/lib/save-download.ts`. Abierto: iOS.
- **Imprimir una A4 con el diálogo del sistema:** hub#2006 (ADR en borrador «En la app de escritorio
  la A4 sale por el diálogo de impresión del sistema, reutilizando `Webview::print` de Tauri») ·
  `src-tauri/src/native_print.rs` · `src-tauri/src/lib.rs` (`print_document`) ·
  `src-tauri/tests/native_print.rs` · `src-tauri/tests/remote_acl.rs`
  (`the_hub_origin_can_open_the_system_print_dialog`) · consumidores:
  `apps/web/src/lib/native-print.ts`, `apps/web/src/lib/print.ts`. Android (hub#2008):
  `crates/tauri-plugin-erplora-android/{src/lib.rs,android/…/HtmlPrinter.kt}` (+ `HtmlPrinterTest`).
- **Caída de red / pantalla propia y recuperación sola:** hub#1716 (dentro de ADR-0159, sin ADR
  propio: no hay decisión de arquitectura nueva) · `src-tauri/src/connectivity.rs`
  (`next_screen`, `next_probe_delay`, `retry_delay`, `remote_target`, `bundled_page_url`, `show`,
  `spawn_connectivity_guard`) · `src-tauri/src/lib.rs` (`open_main_window`, comando `shell_retry`) ·
  `src-tauri/capabilities/degraded.json` · `shell-dist/index.html` ·
  `src-tauri/tests/shell_offline.rs`. Fuera: continuidad sin conexión (pm#119) y el mensaje del
  *web app* al cortar Internet (hub#1743), que es otra superficie.
- **Deep link + fallback:** [decision-log — ADR-0225](../../adr/ADR-0225.md) (hub#345) ·
  `src-tauri/src/lib.rs` (`resolve_deep_link`, `deep_link_from_args`, `handle_deep_link`) ·
  `src-tauri/tests/deep_link.rs` · `apps/web/src/lib/deep-link.ts` (lanzador canónico del navegador)
  · consumidor: saas#1147.
- **Permisos Android + supervivencia a la regeneración:**
  [decision-log — ADR-0241](../../adr/ADR-0241.md) (hub#337) ·
  `crates/tauri-plugin-erplora-android/android/src/main/AndroidManifest.xml` ·
  `apps/tauri/src-tauri/tests/android_local_network.rs` · paso «Verificar permisos de hardware en el
  APK» de `.github/workflows/tauri-release.yml`.
- **Hueco de la barra de estado en Android:** hub#1719 · color de la franja hub#1903 (abierta: seguir al
  tema elegido en la app, hub#1911) ·
  `crates/tauri-plugin-erplora-android/android/…/SystemBarInsets.kt` (+ `SystemBarInsetsTest.kt`) ·
  `apps/tauri/src-tauri/gen/android/app/…/MainActivity.kt` ·
  `apps/tauri/src-tauri/tests/android_status_bar_inset.rs`.
- **UI / PWA:** [`web.md`](web.md) · transporte auto en `packages/module-sdk` (`isTauri()`).
- **Hardware:** [`bridge.md`](bridge.md) · `crates/peripherals`.
- **Entitlement / tiers:** [`../module-system.md`](../module-system.md) §2bis/§4/§5.8 · [`../auth.md`](../auth.md).
