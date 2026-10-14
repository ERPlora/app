# Documentación histórica de la app (`docs/legacy/`)

Copias **históricas** de documentos del repo de arquitectura del producto antiguo,
`erplora-old/architecture` (antes `ERPlora/architecture`), tomadas el **2026-10-14** de su rama `main`
en el commit `2113309c11a7201c8f61700d13008ba55f61db26` (2026-10-09, el último de la rama, comprobado
con `git ls-remote`).

Están aquí para que no se pierda lo que se sabía de la app **fuera de `apps/tauri`** cuando la app
salió del monorepo `erplora-old/hub`: el código de este repo cita ADR por número (`ADR-0196`,
`ADR-0204`…) y docs del repo de arquitectura (`architecture/hub/apps/tauri.md`…), y sin estas copias
esas citas no llevarían a ningún sitio. **Describen el producto antiguo** (Hub en Rust con su shell
`apps/web`, el SaaS antiguo y el Bridge); la referencia vigente es erplora-next y sus `PLAN*.md` (la
app, en la ronda 18: `PLAN.md` §23). No se mantienen ni se corrigen: lo que cambie se decide en los
PLAN y en el `README.md` y el `WORKFLOW.md` de este repo.

## Cómo se copió

- **Misma ruta**: `architecture/<ruta>` → `docs/legacy/<ruta>`. Una cita del código como
  `architecture/hub/apps/tauri.md` está en [`hub/apps/tauri.md`](hub/apps/tauri.md).
- **ADR**: en el origen son secciones de un solo `00-overview/decision-log.md` (unas 34 600 líneas y
  600 ADR). Cada una se extrae tal cual a `adr/ADR-NNNN.md`, y su cabecera dice de qué líneas sale.
  Las decisiones del formato nuevo, un fichero por decisión y sin número
  (`00-overview/decision-log.d/<fecha>-<título>.md`), van a `adr/` con su nombre.
- Cada copia empieza con una cabecera de 3 líneas; lo demás es el texto original. Solo cambian los
  enlaces relativos que apuntaban a otro fichero copiado o a una ADR copiada
  (`decision-log.md#adr-0196`, o `[ADR-0159](…/decision-log.md)` sin ancla): 117 enlaces, que ahora
  llevan a la copia.
- El resto de enlaces se deja como estaba y **aquí está roto**: 51 a docs no copiados
  (`hub/apps/web.md`, `hub/auth.md`, `hub/module-system.md`, `../../../hub/CLAUDE.md`…) y 45 al
  `decision-log.md` entero o a ADR no copiadas (`#adr-0308`…). Se leen en `erplora-old/architecture`,
  en el commit de arriba.
- Este repo es público: antes de copiar se comprobó que no hubiera secretos, contraseñas ni datos
  personales (solo nombres de secrets y de variables).

## Lo copiado

### Contexto y diseño

| Fichero | Qué cuenta | Citado por |
| --- | --- | --- |
| [`context/tauri.md`](context/tauri.md) | Contexto de la app: cliente fino (ADR-0159) y única app instalable con el hardware (ADR-0196/0204); qué leer antes de una tarea | — |
| [`context/bridge.md`](context/bridge.md) | Lápida del Bridge standalone (borrado en hub#340); remite a `context/tauri.md` | — |
| [`hub/apps/tauri.md`](hub/apps/tauri.md) | **El diseño de la app**: qué es, comandos verificados, frontera de orígenes y hub enlazado, salidas al navegador y al disco, A4, CSP, pantalla sin red, páginas que sigue la copia de Google Play, captura de `hub_url`, `erplora://` y su fallback, Android (permisos, barra de estado) | `WORKFLOW.md` (cabecera y l. 399), `src-tauri/src/lib.rs:226`, `src-tauri/tests/shell_csp.rs:137` |
| [`hub/apps/bridge.md`](hub/apps/bridge.md) | Historia del Bridge standalone en Rust (Axum, WS `:12321`, pairing device-code) que la app sustituyó | `crates/peripherals/README.md` |
| [`hub/crates/peripherals.md`](hub/crates/peripherals.md) | El crate `erplora-peripherals`: los tres transportes, tipos de documento ESC/POS, comanda, idioma y escala del papel, flujo de un trabajo, descubrimiento, constantes y errores | `crates/peripherals/WORKFLOW.md` (cabecera y l. 548) |
| [`hub/print-queue.md`](hub/print-queue.md) | La cola de impresión del Hub antiguo: estados, estaciones, mapa documento → estación, hosts de impresión, drenaje por `/ws/print`, documento que viaja, puerta del shell | `crates/peripherals/WORKFLOW.md` (cabecera y l. 540) |
| [`modules/printing.md`](modules/printing.md) | El módulo `printing` antiguo: ajustes del papel, impresoras que descubre la app y su rol, impresión al cerrar la venta | — |
| [`_frozen/hub/APP-UNIFICADA.md`](_frozen/hub/APP-UNIFICADA.md) | «Una sola app, tres empaquetados» (ADR-0050), ya congelado en el origen | — (su ADR-0050 sí, ver abajo) |
| [`bridge/README.md`](bridge/README.md) | Índice del hardware: las dos encarnaciones del Bridge y los módulos del crate | — |
| [`bridge/hardware-protocol.md`](bridge/hardware-protocol.md) | Lo que sigue vivo del Bridge: ESC/POS por TCP, renderizadores de documento, cajón, cola y descubrimiento | — |
| [`bridge/websocket-interface.md`](bridge/websocket-interface.md) | El WebSocket del Bridge (`:12321`), ya inexistente; su vocabulario (comandos, `printer_id`, errores) sigue en los `invoke` | — |
| [`bridge/android-app.md`](bridge/android-app.md) | La app Kotlin del Bridge para Android, borrada por ADR-0196; el SPP volvió al plugin (ADR-0204) | — |
| [`desktop-mobile/README.md`](desktop-mobile/README.md) | Lápida del producto «Hub Local» (Tauri + SQLite, retirado por ADR-0154) y vuelta del empaquetado Tauri | — |
| [`desktop-mobile/tauri-desktop.md`](desktop-mobile/tauri-desktop.md) | Lo mismo para el escritorio | — |
| [`desktop-mobile/android.md`](desktop-mobile/android.md) | Android vuelve dentro de la app Tauri, no como Bridge Kotlin | — |
| [`desktop-mobile/entitlement-gating.md`](desktop-mobile/entitlement-gating.md) | El gate de arranque propio de la app Tauri gratuita, retirado | — |

### ADR que el código cita por número

| Fichero | Qué cuenta | Citado por |
| --- | --- | --- |
| [`adr/ADR-0033.md`](adr/ADR-0033.md) | El asistente es del core del Hub, no un módulo | `src-tauri/src/lib.rs`, `src-tauri/tests/external_open.rs` |
| [`adr/ADR-0050.md`](adr/ADR-0050.md) | App unificada: tres empaquetados y transporte HTTP+WS directo | `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs` |
| [`adr/ADR-0053.md`](adr/ADR-0053.md) | Keystore de Android compartida, alias `erplora` | `README.md`, `GOOGLE-PLAY.md`, `tauri-release.yml`, `src-tauri/gen/android/app/build.gradle.kts` |
| [`adr/ADR-0055.md`](adr/ADR-0055.md) | Cada módulo trae sus traducciones (`locales/<lang>.json`) | `crates/peripherals/src/{escpos,lib}.rs`, plugin Android `src/lib.rs`, `shell-dist/index.html`, `src-tauri/src/{lib,link_consent}.rs`, `src-tauri/tests/shell_offline.rs` |
| [`adr/ADR-0114.md`](adr/ADR-0114.md) | Monetización por hub: el Hub instala, el SaaS vende | `WORKFLOW.md`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/tests/{external_open,shell_surface}.rs` |
| — | **ADR-0136 no existe**: nunca tuvo entrada en el log, que registró esa decisión como ADR-0421 | `tauri-release.yml`, `scripts/pack-msix.ps1`, `src-tauri/msix/Package.appxmanifest` |
| [`adr/ADR-0141.md`](adr/ADR-0141.md) | `order` mutable → `sale` inmutable | `crates/peripherals/src/escpos.rs` |
| [`adr/ADR-0154.md`](adr/ADR-0154.md) | Cloud-first único: muere el Hub Local (Tauri + SQLite) | `tauri-release.yml`, `MICROSOFT-STORE.md`, `package.json`, `scripts/pack-msix.test.sh`, `src-tauri/{Cargo.toml,build.rs,capabilities/onboarding.json,src/lib.rs}`, `src-tauri/tests/{remote_acl,shell_surface}.rs` |
| [`adr/ADR-0159.md`](adr/ADR-0159.md) | La app como cliente fino: webview del SaaS y del hub, sin BD local | `test-shell.yml`, `shell-dist/index.html`, `src-tauri/{Cargo.toml,build.rs,capabilities/default.json,src/connectivity.rs,src/lib.rs}`, `src-tauri/tests/{deep_link,remote_acl,shell_offline,shell_surface}.rs` |
| [`adr/ADR-0160.md`](adr/ADR-0160.md) | Una sola identidad de tienda, `com.erplora.app` | `tauri-release.yml`, `GOOGLE-PLAY.md`, `MICROSOFT-STORE.md`, `scripts/{pack-msix.ps1,release-version.sh}`, `src-tauri/gen/android/app/build.gradle.kts`, `src-tauri/src/lib.rs`, `src-tauri/tests/android_local_network.rs` |
| [`adr/ADR-0180.md`](adr/ADR-0180.md) | Android es la misma app Tauri; sus permisos de runtime son producto | `GOOGLE-PLAY.md` |
| [`adr/ADR-0196.md`](adr/ADR-0196.md) | Muere el Bridge: una sola app instalable que es el acceso al hardware | `crates/peripherals/{README.md,src/escpos.rs,src/lib.rs,src/queue.rs,src/usb.rs}`, `src-tauri/{Cargo.toml,src/lib.rs,capabilities/onboarding.json,msix/Package.appxmanifest}`, manifiesto Android, `src-tauri/tests/{android_local_network,deep_link,remote_acl}.rs`, `GOOGLE-PLAY.md`, `MICROSOFT-STORE.md`, `WORKFLOW.md` |
| [`adr/ADR-0199.md`](adr/ADR-0199.md) | El código en inglés; la UI traducida desde el inglés | `src-tauri/tests/shell_offline.rs` |
| [`adr/ADR-0204.md`](adr/ADR-0204.md) | Vuelve el Bluetooth SPP (solo Android, en el plugin propio) y el autoarranque de escritorio | plugin Android (`BluetoothSpp.kt`, `ErploraAndroidPlugin.kt`, `PermissionPolicy.kt`, sus tests, manifiesto, `src/lib.rs`, `Cargo.toml`), `crates/peripherals/{README.md,src/*}`, `src-tauri/{Cargo.toml,build.rs,src/lib.rs}`, manifiesto Android, `src-tauri/tests/android_local_network.rs`, `GOOGLE-PLAY.md` |
| [`adr/ADR-0221.md`](adr/ADR-0221.md) | Frontera de orígenes: el SaaS es de donde arranca, no de donde se acciona el hardware | `src-tauri/src/lib.rs`, `src-tauri/tests/{deep_link,external_open,remote_acl}.rs` |
| [`adr/ADR-0225.md`](adr/ADR-0225.md) | `erplora://` se resuelve, no se abre; el fallback del navegador | `src-tauri/src/lib.rs`, `src-tauri/tests/external_open.rs` |
| [`adr/ADR-0255.md`](adr/ADR-0255.md) | La app sale al navegador del sistema | `src-tauri/src/lib.rs`, `src-tauri/tests/{save_download,shell_surface}.rs` |
| [`adr/ADR-0259.md`](adr/ADR-0259.md) | Descargar es guardar el fichero y decir dónde | plugin Android (`DownloadPublisher.kt`, `ErploraAndroidPlugin.kt`, `src/lib.rs`), `src-tauri/src/lib.rs`, `src-tauri/tests/save_download.rs` |
| [`adr/ADR-0276.md`](adr/ADR-0276.md) | Una sola puerta para el `document_type` | `crates/peripherals/src/queue.rs` |
| [`adr/ADR-0381.md`](adr/ADR-0381.md) | Un combo es un grupo de líneas | `crates/peripherals/src/escpos.rs` |
| [`adr/ADR-0394.md`](adr/ADR-0394.md) | Comanda térmica: jerarquía con sangrado, negrita solo para los cambios | `crates/peripherals/src/escpos.rs` |
| [`adr/ADR-0396.md`](adr/ADR-0396.md) | Tique del cliente: el menú en una línea, componentes sangrados sin importe | `crates/peripherals/src/escpos.rs` |
| [`adr/ADR-0421.md`](adr/ADR-0421.md) | MSIX sin firmar para la Microsoft Store (la que el código llama «ADR-0136») | `MICROSOFT-STORE.md` y los tres de ADR-0136 |

### ADR de la app que el código no cita por número

Tratan de código de este repo; se copian para que no se pierda el porqué. Se nombra el fichero que
las aplica y la issue que los une.

| Fichero | Qué cuenta | Lo aplica |
| --- | --- | --- |
| [`adr/ADR-0235.md`](adr/ADR-0235.md) | `discover_printers` dice por qué no hay lista («el sistema no me deja mirar») | `crates/peripherals/src/discovery.rs`, `ErploraAndroidPlugin.kt` (hub#338) |
| [`adr/ADR-0241.md`](adr/ADR-0241.md) | Los permisos de Android se declaran en el plugin, con tres guardas | manifiesto y `src/lib.rs` del plugin, `src-tauri/tests/android_local_network.rs` (hub#337) |
| [`adr/ADR-0243.md`](adr/ADR-0243.md) | `?shell=1`: qué origen puede quedar grabado como el hub de la instalación | `src-tauri/src/lib.rs`, `src-tauri/tests/remote_acl.rs` (hub#335) |
| [`adr/ADR-0247.md`](adr/ADR-0247.md) | Sin entitlement propio; la superficie de comandos se verifica | `src-tauri/tests/shell_surface.rs` (hub#336) |
| [`adr/ADR-0260.md`](adr/ADR-0260.md) | La app comprueba si es la versión publicada; un fallo al comprobar es silencio | `src-tauri/tests/app_update_channel.rs` (hub#400) |
| [`adr/ADR-0307.md`](adr/ADR-0307.md) | En Android, guardar es publicar en Descargas | `DownloadPublisher.kt` (hub#499) |
| [`adr/ADR-0326.md`](adr/ADR-0326.md) | «Impreso» solo lo dice quien puede entregar | `src-tauri/src/{lib,native_print}.rs`, `src-tauri/tests/native_print.rs` (hub#862) |
| [`adr/ADR-0373.md`](adr/ADR-0373.md) | Android solo por Google Play | `GOOGLE-PLAY.md` (hub#984) |
| [`adr/ADR-0441.md`](adr/ADR-0441.md) | La impresora USB por la cola RAW del sistema operativo | `crates/peripherals/src/usb.rs` (hub#1083, hub#1536) |
| [`adr/ADR-0492.md`](adr/ADR-0492.md) | La copia de Google Play solo sigue las páginas del SaaS que no cobran | `src-tauri/src/navigation.rs` (`PLAY_SAAS_PAGES`), `src-tauri/tests/play_navigation.rs` (hub#1915) |
| [`adr/ADR-0509.md`](adr/ADR-0509.md) | Una `invoice` sale por la térmica como factura completa o se rechaza | `crates/peripherals/src/escpos.rs` (hub#2005) |

### Decisiones del formato nuevo (`decision-log.d/`, sin número)

| Fichero | Qué cuenta | Lo aplica |
| --- | --- | --- |
| [`adr/2026-09-30-android-escucha-avisos-con-la-pantalla-apagada.md`](adr/2026-09-30-android-escucha-avisos-con-la-pantalla-apagada.md) | En Android los avisos llegan con la pantalla apagada porque la app se queda a la escucha, sin push | `NoticeListening.kt` y manifiesto del plugin, `workflow/permisos-y-avisos.md` (hub#2307) |
| [`adr/2026-09-30-avisos-escritorio-y-android-en-frio.md`](adr/2026-09-30-avisos-escritorio-y-android-en-frio.md) | El shell guarda el toque de un aviso y la página lo reclama | `src-tauri/src/notice_tap.rs`, `NoticeLaunch.kt`, `NoticeTaps.kt` (hub#2360) |
| [`adr/2026-09-30-avisos-windows-enlace-propio.md`](adr/2026-09-30-avisos-windows-enlace-propio.md) | En Windows el aviso lleva su enlace `erplora://notice` | `src-tauri/src/{lib,notice_tap}.rs`, `msix/Package.appxmanifest` (hub#2409) |
| [`adr/2026-10-01-ficha-de-google-play-publicada-el-interruptor-nace-encendido.md`](adr/2026-10-01-ficha-de-google-play-publicada-el-interruptor-nace-encendido.md) | La ficha de Google Play está publicada: el SaaS manda a Play por defecto | lado SaaS (saas#2430) |
| [`adr/2026-10-07-el-vale-de-anulacion-es-la-misma-comanda-con-cantidades-negativas.md`](adr/2026-10-07-el-vale-de-anulacion-es-la-misma-comanda-con-cantidades-negativas.md) | El vale de anulación es la misma comanda con cantidades negativas, para que las apps viejas lo impriman | `crates/peripherals/src/escpos.rs` (kitchen#168) |
| [`adr/2026-10-08-el-vale-de-un-plato-anulado.md`](adr/2026-10-08-el-vale-de-un-plato-anulado.md) | Vale por plato anulado, con su etiqueta | `crates/peripherals/WORKFLOW.md` (paso 5 de la comanda) |
| [`adr/2026-10-08-el-hardware-de-la-app-solo-para-el-hub-enlazado.md`](adr/2026-10-08-el-hardware-de-la-app-solo-para-el-hub-enlazado.md) | Solo el hub enlazado acciona el hardware | `src-tauri/src/hub_link.rs`, `capabilities/default.json`, `src-tauri/tests/{remote_acl,play_navigation}.rs` (hub#2504) |
| [`adr/2026-10-08-los-plugins-de-terceros-tras-la-puerta-del-hub-enlazado.md`](adr/2026-10-08-los-plugins-de-terceros-tras-la-puerta-del-hub-enlazado.md) | Los plugins de terceros, envueltos en `hub_link::Gated` | `src-tauri/src/hub_link.rs`, plugin Android (hub#2642) |
| [`adr/2026-10-08-un-enlace-a-otro-negocio-pregunta-antes.md`](adr/2026-10-08-un-enlace-a-otro-negocio-pregunta-antes.md) | Un `erplora://hub/…` a otro negocio pregunta antes en un diálogo del sistema | `src-tauri/src/link_consent.rs`, `HubQuestion.kt` (hub#2644) |
| [`adr/2026-10-09-el-fallo-de-la-impresora-propia-vuelve-a-la-caja-con-reintentar.md`](adr/2026-10-09-el-fallo-de-la-impresora-propia-vuelve-a-la-caja-con-reintentar.md) | `erplora_print` espera al papel y devuelve `printer_unreachable` | `crates/peripherals/src/queue.rs`, `src-tauri/src/lib.rs` (hub#2494) |

## Lo que no se copió

### ADR de la app o de la impresión que se quedan en el log

Están en `00-overview/decision-log.md` del origen, en la línea indicada.

- ADR-0001 (l. 98): app Tauri gratis con gate por tiers; retirada por ADR-0154.
- ADR-0019 (l. 638): la impresión al cerrar la venta la dispara el shell web.
- ADR-0080 (l. 2668): dos productos, Local/Tauri y PWA; retirada.
- ADR-0088 (l. 2887): nombres canónicos (Hub Local = Tauri); retirada.
- ADR-0110 (l. 3201): JWT del Bridge local; el Bridge ya no existe.
- ADR-0145 (l. 4536): destino de la comanda por estación; lado servidor.
- ADR-0245 (l. 11321): la dirección de un hub se resuelve, nunca se monta; lado SaaS.
- ADR-0250 (l. 11764): «Open Terminal» sin dirección explica en vez de lanzar; lado SaaS.
- ADR-0262 (l. 12995): el host de impresión drena por un canal propio; lado servidor.
- ADR-0266 (l. 13370): el tique viaja estructurado y no se guarda; lado servidor.
- ADR-0278 (l. 14339): la ficha `/app/` y el 301 de `/bridge/`; lado SaaS.
- Enmienda a ADR-0159 (l. 18685): un 401 del Cloud solo expira la sesión que el Cloud emitió; lado web.
- ADR-0337 (l. 18040): denuncia de contenido de la IA (Microsoft Store 11.16); lado SaaS.
- ADR-0346 (l. 18940): una estación de impresión es una fila con id; lado servidor.
- ADR-0351 (l. 19403): el módulo dice qué imprime y el hub dónde; lado servidor.
- ADR-0448 (l. 27965): quién lee el sello de la cola; lado servidor.
- ADR-0490 (l. 31368): la puerta a la cuenta de erplora.com en todas las copias; lado web.

### Ficheros que solo mencionan la app

Salen al buscar `tauri|google play|microsoft store|msix|escpos|esc/pos|peripherals|bridge|webview|android`
en el origen, pero no tratan de la app.

- `README.md`: índice del repo de arquitectura; enlaza a lo copiado.
- `00-overview/README.md`: índice de la carpeta; nombra el Bridge.
- `00-overview/decision-log.md`: el log entero; de él solo se extraen las ADR de arriba.
- `00-overview/decision-log-index.md`: índice del log (número, fecha, estado y título).
- `00-overview/decision-log.d/2026-10-05-el-hub-se-describe-por-areas-y-la-verificacion-lee-el-codigo.md`: contrato de los `WORKFLOW.md`; nombra el prefijo `HUB_APP`.
- `00-overview/decision-log.d/2026-10-06-el-control-horario-es-un-modulo-aparte-y-la-geovalla-solo-frena-al-movil.md`: fichaje; la ubicación en la app Android, de pasada.
- `00-overview/decision-log.d/2026-10-07-readyz-anonimo-solo-estados.md`: salud del hub; nombra el control previo a Google Play.
- `00-overview/decision-log.d/2026-10-07-una-app-comprada-se-retira-nunca-se-borra.md`: marketplace; Google Play como referencia de mercado.
- `00-overview/glossary.md`: glosario; «Hub Local» retirado.
- `00-overview/lean-north-star.md`: norte del MVP; la instalable es la app.
- `00-overview/mvp-readiness.md`: estado del MVP; filas de periféricos y de la auth del Bridge, superada.
- `00-overview/system-map.md`: mapa del sistema; la app es una caja del diagrama.
- `_experience/cases/sales-224.yml`: caso de QA de ventas; la báscula, de pasada.
- `_experience/cases/sales-230.yml`: caso de QA de ventas; los suplementos en el papel.
- `_frozen/README.md`: índice de lo congelado; una fila sobre `APP-UNIFICADA.md`.
- `_frozen/hub/integration-edge.md`: webhooks del Hub; «Tauri/local» es el Hub Local retirado.
- `_frozen/hub/HANDOFF-adr0049-integration-edge.md`: traspaso de ADR-0049 (webhooks); «no hay broker de Tauri».
- `_routing/routing-matrix.md`: qué contexto cargar; manda a `context/tauri.md`.
- `aws/README.md`: «EventBridge» de AWS, no el Bridge.
- `aws/database.md`: «EventBridge» de AWS.
- `aws/ecs.md`: «EventBridge» de AWS.
- `aws/infrastructure.md`: «EventBridge» de AWS.
- `aws/lambdas.md`: «EventBridge» de AWS.
- `aws/storage.md`: las descargas iban a Hetzner Object Storage; hoy van a S3 (ver el `README.md` del repo).
- `context/hub.md`: contexto general del Hub antiguo; lo de la app son dos líneas que ya cuenta `context/tauri.md`.
- `context/rust.md`: reglas de Rust; nombra `peripherals`.
- `contracts/backend-contract.md`: aclara que sus comandos no son comandos Tauri.
- `contracts/interop-contract.md`: «bridge» en otro sentido.
- `contracts/quantity-contract.md`: «bridge» en otro sentido (báscula).
- `contracts/workflow-contract.md`: dónde vive el `WORKFLOW.md` de la app (`HUB_APP`).
- `hub/README.md`: índice del Hub; la app es `erplora-app`.
- `hub/apps/README.md`: índice de apps; una fila por `tauri.md` y `bridge.md`.
- `hub/apps/web.md`: el shell web. Sus secciones «La salida al NAVEGADOR», «La salida al DISCO», «El canal de actualización de la app instalada», «Hardware: UNA sola puerta» y «Comandos» (líneas 780–992) son la mitad de página del contrato con la app: se leen al portar a hub-next el puente JS.
- `hub/auth.md`: credenciales del Hub; el JWT del Bridge (retirado) y el lector NFC del aparato, de pasada.
- `hub/crates/README.md`: índice de crates; filas de `peripherals` y del plugin Android.
- `hub/crates/cloud-client.md`: el gate de arranque de la app Tauri gratuita, retirado.
- `hub/crates/guest-sdk.md`: enlace a `peripherals.md`.
- `hub/crates/runtime.md`: «en el futuro, `apps/tauri`», del modo local retirado.
- `hub/crates/server.md`: denuncia de contenido de la IA (Microsoft Store 11.16).
- `hub/crates/sync.md`: enlace a `peripherals.md`.
- `hub/crates/wasm-host.md`: enlace a `peripherals.md`.
- `hub/dashboard/README.md`: la píldora del Bridge en el panel.
- `hub/diagrama-general.md`: diagrama general; la app es una caja.
- `hub/kernel-contract.md`: compatibilidad; nombra `tauri.conf.json`.
- `hub/module-system.md`: compras fuera de Google Play; permisos al estilo Android.
- `hub/notification-bell.md`: la campana; el aviso del sistema remite a `apps/tauri.md`.
- `hub/overview.md`: el Hub Local, retirado.
- `hub/public-api.md`: la webview lee el canal de eventos con credencial.
- `hub/public-claims.md`: claims públicos; el QR promocional del tique (también en `hub/print-queue.md`).
- `hub/runtime-dispatcher.md`: `BRIDGE_FUNCTIONS` de SQL, otro «bridge».
- `hub/settings.md`: capabilities «al estilo de los permisos de Android».
- `hub/system-info.md`: la pantalla `/system`; la sonda del Bridge, derogada.
- `hub/versioning.md`: en el monorepo un tag `v*` publicaba también la app; aquí lo explica el `README.md`.
- `module-toolkit/README.md`: analogía con `NewApi` de Android.
- `modules/README.md`: índice; una fila de `printing`.
- `modules/_template.md`: plantilla de módulo; «printer vía bridge».
- `modules/appointments.md`: Linux y Android, de pasada.
- `modules/attendance.md`: el fichaje con ubicación en la app Android.
- `modules/combos.md`: combos en ESC/POS; su regla está en ADR-0381 y ADR-0396, copiadas.
- `modules/customers.md`: el WKWebView de macOS e iOS, de pasada.
- `modules/invoice.md`: la factura completa por la térmica; su regla está en ADR-0509, copiada.
- `modules/invoice_series.md`: `erp_pad`, una «bridge func» de SQL.
- `modules/kitchen.md`: el aviso de urgencia por papel.
- `modules/sales.md`: el documento que lee `escpos::render_receipt`.
- `modules/services.md`: Android, de pasada.
- `saas/README.md`: panorama; nombra el Bridge.
- `saas/api-reference.md`: el `bridge-token`, retirado.
- `saas/assistant.md`: denuncia de contenido de la IA (Microsoft Store 11.16).
- `saas/auth-and-permissions.md`: el `?next=` y el `hub-bridge` del login.
- `saas/dashboard/developer-portal.md`: `ion-form-bridge.js`.
- `saas/dashboard/hubs.md`: `deployment_mode` de escritorio, retirado.
- `saas/error-reporting.md`: denuncia de contenido de la IA (Microsoft Store 11.16).
- `saas/hub-provisioning.md`: enlace a «El cuarto final» de `hub/apps/tauri.md`.
- `saas/marketplace.md`: Google Play sin compras dentro de la app; la app Tauri gratis, retirada.
- `saas/monetization.md`: Google Play sin compras dentro de la app.
- `saas/overview.md`: EventBridge, `ion-form-bridge.js` y la puerta de la cuenta, de pasada.
- `saas/public-portal.md`: su sección «La app instalable: `appdist`» (`/app/`, `/app/download/<plataforma>/`, Google Play) es del SaaS.
- `saas/verifactu-gateway.md`: una fila «Android» en una tabla.
- `workflows/README.md`: los prefijos `HUB_APP` y `HUB_PERIPHERALS`.
- `workflows/alta.md`: el alta; pasos con la app y la impresora.

### Fuera del repo de arquitectura

- **Monorepo `erplora-old/hub`**, en el commit de origen de la app
  (`9dd3821573017ea8e162cf9bbb9eb597739c3aed`, ver el `README.md` del repo): `ARQUITECTURA.md` §2.7
  (hardware; de ahí viene la tabla de transportes de `crates/peripherals/README.md`),
  `workflow/impresion.md` (la mitad servidor de la impresión, `HUB-F190` a `HUB-F207`) y
  `apps/web/WORKFLOW.md` (`HUB_SHELL`). Las citan `WORKFLOW.md` y `crates/peripherals/WORKFLOW.md`.
- **`knowledge/legacy-reference/feature-inventory/bridge/`** de erplora-next (`INVENTORY.md`,
  `LEGACY-DECISIONS.md`, `sections/rust.md`, `sections/android.md`): el inventario del Bridge en Rust
  y en Android, con sus bugs y diferencias. La ruta es desde la raíz de erplora-next: `knowledge/` es
  una carpeta hermana de `app/`, fuera de este repo, así que aquí no se enlaza ni se duplica.
