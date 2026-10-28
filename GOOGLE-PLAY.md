# ERPlora → Google Play (canal Android único de cliente)

**Decisión vigente (ADR-0180, 2026-08-01, confirmada por ADR-0196, 2026-08-03):** Android es
la **MISMA app Tauri** de este repo, bajo la identidad única **`com.erplora.app`**
(ADR-0160). ADR-0180 revierte el §3 de ADR-0160 —que había sacado el Tauri-Android de
[`tauri-release.yml`](.github/workflows/tauri-release.yml)— y devuelve el build aquí:
job **`build-android`**. ADR-0196 remata la dirección: **la app Kotlin de
`ERPlora-Bridge-android` muere**, y con ella el bridge como proceso aparte.

> Una **release** es un push a `main` con una versión nueva en `src-tauri/tauri.conf.json` (README,
> «CI/CD»); ya no hay tags `v*` a mano: el tag lo crea la propia release al terminar.

> ✅ **El job `publish-play` ya existe** en este workflow y el AAB sale firmado del pipeline. Lo
> que falta es trámite, no cableado — ver «Estado real» aquí abajo. Seguimiento: hub#984 (hub#308
> se consolidó ahí).

Sigue vigente la decisión de canal (2026-07-16): Android de cara al cliente se distribuye
**SOLO por Google Play** (UE; confianza; todos los terminales objetivo tienen Play). La
landing no ofrece APK: el endpoint de descarga del SaaS hace **redirect 302** a Play
(saas#707).

## Estado real (2026-08-19, verificado en la consola)

| Pieza | Estado |
| --- | --- |
| Cuenta de desarrollador | **Organization account** ✅ — exenta del closed testing obligatorio (12 testers/14 días), puede ir a producción directa |
| Ficha `com.erplora.app` | Creada; app id `4974793613374910910` |
| Primera subida manual | Hecha (registra la upload key). En la consola hay `1001003 (1.1.3)` en *Closed testing - Alpha* |
| Formularios | **Los 10 completos** y en «Ready to send for review»: content rating, target audience 18+, privacy policy, ads, data safety, health apps, government apps, financial features, advertising ID, sign in details |
| Cuenta para el revisor | Declarada («ERPlora reviewer account», usuario + contraseña + instrucciones) |
| Envío a revisión | ⚠️ **Nunca se había hecho.** `Submission activity` estaba vacío y los 14 cambios llevaban desde el 14/08 guardados sin enviar. La app seguía en `Draft` como `com.erplora.app (unreviewed)` |
| `PLAY_PACKAGE_NAME` / `PLAY_SERVICE_ACCOUNT_JSON` | ❌ No existen (tampoco en `ERPlora/app`) → `publish-play` sale **skipped** en cada release |
| `RELEASE_CHANNELS_PENDING` | `store,play,s3` → el gate deja pasar la release en VERDE (avisando) aunque no publique en esos canales |

**Orden que queda**, y no se salta ninguno:

0. **Antes de enviar nada a revisión, correr la guardia de la cuenta del revisor** (hub#1718):

   ```bash
   python3 scripts/ci/play-reviewer-preflight.py
   ```

   Entra con `PLAY_REVIEWER_EMAIL`/`PLAY_REVIEWER_PASSWORD` del `.env` de la raíz —las mismas
   credenciales que recorre Google— y hace el mismo camino que el revisor en la plataforma nueva
   (desde el 2026-10-14: saas-next en `erplora.com`, hub-next en `https://<slug>.erplora.com`):
   login de allauth headless (`/_allauth/browser/v1/auth/login`, con CSRF), lista de negocios
   (`/api/hubs/businesses`), `/readyz` del hub, el salto «Abrir» (`/auth/login` del hub →
   `/hub-login/` del SaaS → `/auth/callback`) y las apps que ve dentro (`/api/me`). **Para el envío**
   si la cuenta no entra, si no tiene negocio, si `PLAY_REVIEWER_HUB` nombra un hub que no es suyo o
   que el SaaS no deja abrir, si el hub no responde `status: ok`, si no le abre sesión o si dentro
   no ve ningún módulo. Todos son el mismo rechazo: el revisor no ve la app. **Sale en rojo si
   faltan las credenciales**, nunca en verde por no poder comprobarlo, y en rojo `code_required` si
   la cuenta entra con su contraseña pero erplora.com le pide el código que manda por correo
   (`login_by_code`, o `verify_email` si el correo aún no está verificado): el revisor de Google no
   puede leer ese buzón, así que la cuenta tiene que estar marcada como cuenta de revisión en el
   SaaS (ERPlora/saas#132). La guardia solo llama al login (`…/auth/login`), que nunca da de alta a
   nadie —el alta es otra puerta, `…/auth/signup`, que no toca—, tenga el SaaS el alta abierta o no
   (app#7). `/readyz` de hub-next es del servicio, no de un hub: qué hub existe y se puede abrir lo
   dice el SaaS. Si falla, se arregla el hub o la variable y se vuelve a correr: **no se envía con
   esto en rojo.** Con el canal de CI encendido (pasos 3 y 4) la corre además el job `publish-play`
   antes de subir cada release (hub#1888).

1. Enviar a revisión el lote de la Alpha cerrada (botón «Submit N changes for review»).
2. Aprobada: crear release de **Producción** y enviarla también. Hasta que Producción esté LIVE,
   `play.google.com/store/apps/details?id=com.erplora.app` responde **404**.
3. Service account (invitada en Play Console con «Release to testing tracks») → secret
   `PLAY_SERVICE_ACCOUNT_JSON` + Variable `PLAY_PACKAGE_NAME` **en el repo `ERPlora/app`**
   (`gh secret set -R ERPlora/app PLAY_SERVICE_ACCOUNT_JSON < service-account.json`,
   `gh variable set -R ERPlora/app PLAY_PACKAGE_NAME --body com.erplora.app`): las Variables de la
   organización con visibilidad privada NO llegan a un repo público. En el MISMO repo: Variables
   `PLAY_REVIEWER_EMAIL` y `PLAY_REVIEWER_HUB` + secret `PLAY_REVIEWER_PASSWORD` (los valores del
   `.env`). Mientras `play` siga en `RELEASE_CHANNELS_PENDING`, `publish-play` **no corre** aunque
   todo esto esté puesto (app#7): el orden entre ellas da igual y ninguna release se pone roja por
   Play. Con el canal encendido, el job corre la guardia del paso 0 antes de subir: sin las tres
   sale en rojo `missing_credentials` (hub#1888); sin `PLAY_SERVICE_ACCOUNT_JSON`, en rojo
   nombrándolo.
4. Quitar **`play`** de `RELEASE_CHANNELS_PENDING` cuando el paso 0 salga en verde con la cuenta del
   revisor (marcada como cuenta de revisión en el SaaS, ERPlora/saas#132, y con su negocio activo).
   Es lo que enciende el canal: desde la release siguiente `publish-play` corre, y un fallo suyo pone
   la release en rojo, como cualquier canal vivo. Una release que ya salió sin Play no se repite: se
   publica con la siguiente versión, o, si la run sigue a medias, `gh run rerun <run> --failed`.
5. Mergear el cambio del SaaS que retira el APK de Android (`store_url_for`), **solo cuando el
   paso 2 esté LIVE**: antes mandaría al usuario a un 404 de Google.

## Requisitos

- **Cuenta de ORGANIZACIÓN verificada.** Las cuentas org están **EXENTAS** del requisito de
  closed testing (12 testers/14 días, solo cuentas personales) → **producción directa**.
- **AAB obligatorio + Play App Signing**: Google custodia la app signing key; nuestra
  keystore (alias `erplora`, ADR-0053) actúa de **upload key** (registro por-app; reseteable
  vía soporte). Verificar expiración (`keytool -list -v`, > 22-oct-2033) + copia fuera de
  GitHub.
- **Target API 36** desde el 31-ago-2026 (apps nuevas y updates).
- **La PRIMERA subida del AAB es MANUAL** en Play Console (requisito de Google) y es la que
  **registra la upload key**.
- **versionCode monotónico**: `major*1e6 + minor*1e3 + patch` → **1.0.0 = 1000000** (la serie
  arranca en 1.0.0, ADR-0160). Lo fija el Gradle, no el que calcula Tauri (ADR-0180 §4): en
  Play es una puerta de un solo sentido.
- **Los permisos los dicen los dos `AndroidManifest.xml`** (el del plugin, en
  `crates/tauri-plugin-erplora-android/android/src/main/`, y el de `src-tauri/gen/android`): son la
  fuente, consúltalos en vez de fiarte de una lista aquí. Los que exigen algo en Play Console, en
  «Formularios de Play que dependen de un permiso». El servicio en primer plano que ADR-0196 §2
  había retirado volvió con hub#2307.

## Formularios de Play que dependen de un permiso

Un permiso nuevo en el manifiesto puede exigir rellenar algo en Play Console **antes** de enviar a
revisión la versión que lo lleva: sin ello Google la rechaza o la para. Lo hace Ioan en la consola.

- **Micrófono** (`RECORD_AUDIO` + `MODIFY_AUDIO_SETTINGS`, para dictar al asistente del Hub, flujo
  `BASE-F94` del Hub): *App content › Data safety*, tipo **Audio › Voice or sound recordings**.
  - **Collected: sí.** El audio sale del dispositivo: la página lo graba (`getUserMedia`, dos minutos
    como mucho), se lo manda al hub del negocio (`base.assistant.transcribe`), el hub a erplora.com
    (`/internal/speech/transcribe`) y erplora.com al proveedor de transcripción (`whisper-1` de OpenAI
    por defecto, `ASSISTANT_SERVICE_PROVIDER`). El texto vuelve a la caja del asistente y la persona lo
    lee antes de enviarlo.
  - **Shared: no.** El proveedor transcribe por cuenta de ERPlora (encargado del tratamiento), y Play no
    cuenta como «compartir» lo que se pasa a un proveedor de servicios.
  - **Processed ephemerally: no** (la respuesta prudente). Ni el hub ni erplora.com guardan el audio: la
    operación del hub no abre su base de datos (`database=False`) y erplora.com lo pasa de memoria al
    proveedor sin escribirlo (`assistant/media.py`). Pero cuánto lo retiene el proveedor lo fijan sus
    condiciones de API, no nuestro código: solo se marca «sí» con retención cero por contrato.
  - **Required or optional: optional.** Solo se graba cuando la persona pulsa el micrófono; el TPV vende
    sin él.
  - **Purpose: App functionality.**
  - **La política de privacidad tiene que decir lo mismo** (Play contrasta las dos): que el dictado manda
    el audio a un proveedor de transcripción, y cuál.

  `RECORD_AUDIO` no está en el formulario de permisos sensibles (*Permissions declaration*): no hay
  nada más que declarar. El manifiesto lo acompaña de `android.hardware.microphone` con
  `required="false"`; sin eso Play ocultaría la app a los equipos sin micrófono.
- **Servicio en primer plano** (`FOREGROUND_SERVICE_SPECIAL_USE`, hub#2307): *App content › Foreground
  service permissions*, uso `specialUse` con su descripción y un vídeo (hub#2421).

## Cableado

| Pieza | Dónde |
| --- | --- |
| Build del AAB | este repo, [`tauri-release.yml`](.github/workflows/tauri-release.yml) job `build-android` |
| Subida a S3 (AWS, OIDC) | ídem, job `upload-s3` |
| Publicación en Play | ídem, job **`publish-play`** — gateado por la Variable `PLAY_PACKAGE_NAME` (vacía = saltado) y saltado también mientras `play` esté en `RELEASE_CHANNELS_PENDING` (app#7) |
| Redirect 302 a Play | SaaS `apps/public/downloads.py::store_url_for` + setting `GOOGLE_PLAY_APP_ID` |

## Lo que YA está hecho

No se repite en «Orden que queda» porque no queda: se deja escrito para que nadie lo rehaga.

1. **Ficha única `com.erplora.app`** creada en Play Console.
2. **Primera subida manual** del AAB al track `internal` — es la que registra la upload key
   (keystore ADR-0053) y activa Play App Signing. Con ella se quemó el `versionCode` `1000000`.
3. **Formularios**: Data safety · content rating IARC · privacy policy · target audience · ads ·
   screenshots + feature graphic 1024×500 + icono 512×512. Los 10, completos.
4. **Job `publish-play`** cableado en este repo, con el AAB firmado y verificado (`jarsigner`) y
   `changesNotSentForReview: false` (hub#984) para que la release envíe a revisión de verdad.

⚠️ **La promoción `internal` → `production` sigue siendo MANUAL, y automatizarla no es solo
cuestión de confianza.** `publish-play` sube con `status: completed`, o sea **rollout al 100% de
golpe**: poner `PLAY_TRACK=production` creyendo que se remata la automatización mandaría cada release,
sin probar, a **todas las cajas a la vez**. Para automatizarla hace falta antes un rollout
escalonado (`status: inProgress` + `userFraction`), que hoy no está cableado.

## Verificación

- `bundletool build-apks --mode=universal` + instalar en dispositivo real: la webview carga el
  Hub y la impresora de **red** imprime (TCP:9100). El Bluetooth Classic SPP que ADR-0180 había
  retirado **volvió** para Android (ADR-0204, hub#388, `BluetoothSpp.kt` en
  `crates/tauri-plugin-erplora-android`): verificar también contra una impresora `bluetooth:{mac}`
  emparejada.
- ADR-0180 deja la validación en **dispositivo real sobre una LAN con impresora** como puerta
  —el emulador no puede darla, su red es NAT— junto con la actualización OTA por encima del
  APK instalado.
- Producción: instalar desde Play en dispositivo limpio; la siguiente release llega como
  update OTA de Play.

## Riesgos conocidos

- Los permisos de runtime son **parte del producto**, no empaquetado (ADR-0180 §2): sin
  `ACCESS_LOCAL_NETWORK` el descubrimiento devuelve `[]` —indistinguible de «no hay
  impresoras»— y sin `POST_NOTIFICATIONS` la comanda entra en cocina sin avisar. Ninguno de
  los dos da error. Dónde se pide cada uno, a día de hoy:
  - **Notificaciones** (hub#1732): al darse de alta este equipo como **puesto de impresión** y,
    de reserva, antes de la **primera comanda** (una pantalla de KDS sin impresora no se da de
    alta). Lleva **frase propia antes** del diálogo de Android, y la respuesta se recuerda antes
    de abrirlo, así que no se vuelve a preguntar sola. Si se deniega, la pantalla **Sistema**
    dice que los avisos están apagados y ofrece volver a pedirlo o abrir los Ajustes.
  - **Red local**: al descubrir o imprimir, desde la pantalla de **Impresión** del módulo
    `printing`. Sale **en frío**, sin frase previa, y Android lo redacta como «buscar …
    dispositivos cercanos», que suena a rastreo — pendiente en hub#1773 (fuera de hub#1732:
    el disparo vive en el `module-sdk` y mover su contrato arrastra el espejo del toolkit).
  - **Micrófono**: al pulsar el micrófono del asistente. No lo pide la app sino el WebView (wry,
    `RustWebChromeClient.onPermissionRequest`), sin frase previa: el toque es el contexto. Si se
    niega, el hub dice que el micrófono está denegado y el dictado no empieza; tras dos «no» Android
    deja de preguntar y solo se reactiva en los ajustes del sistema. El micrófono y la ubicación solo
    se conceden a páginas de ERPlora (`WebPermissionGate.kt`): una página ajena cargada en la ventana
    recibe un «no» aunque el permiso de Android esté concedido.
