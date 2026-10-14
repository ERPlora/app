# ERPlora → Google Play (canal Android único de cliente)

**Decisión vigente (ADR-0180, 2026-08-01, confirmada por ADR-0196, 2026-08-03):** Android es
la **MISMA app Tauri** de este repo, bajo la identidad única **`com.erplora.app`**
(ADR-0160). ADR-0180 revierte el §3 de ADR-0160 —que había sacado el Tauri-Android de
[`tauri-release.yml`](.github/workflows/tauri-release.yml)— y devuelve el build aquí:
job **`build-android`**. ADR-0196 remata la dirección: **la app Kotlin de
`ERPlora-Bridge-android` muere**, y con ella el bridge como proceso aparte.

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
| `PLAY_PACKAGE_NAME` / `PLAY_SERVICE_ACCOUNT_JSON` | ❌ No existen → `publish-play` sale **skipped** en cada tag |
| `RELEASE_CHANNELS_PENDING` | `store,play` → el gate deja pasar el tag en VERDE aunque no publique |

**Orden que queda**, y no se salta ninguno:

0. **Antes de enviar nada a revisión, correr la guardia de la cuenta del revisor** (hub#1718):

   ```bash
   python3 scripts/ci/play-reviewer-preflight.py
   ```

   Entra con `PLAY_REVIEWER_EMAIL`/`PLAY_REVIEWER_PASSWORD` del `.env` de la raíz —las mismas
   credenciales que recorre Google— y **para el envío** si la cuenta no tiene negocio, si
   `PLAY_REVIEWER_HUB` nombra un hub que no es suyo, si ese hub no responde `status: UP` o si está
   vacío de módulos. Los cuatro son el mismo rechazo: el revisor no ve la app. **Sale en rojo si
   faltan las credenciales**, nunca en verde por no poder comprobarlo. Si falla, se arregla el hub
   o la variable y se vuelve a correr: **no se envía con esto en rojo.** Con el canal de CI
   encendido (paso 3) la corre además el job `publish-play` antes de subir cada tag (hub#1888).

1. Enviar a revisión el lote de la Alpha cerrada (botón «Submit N changes for review»).
2. Aprobada: crear release de **Producción** y enviarla también. Hasta que Producción esté LIVE,
   `play.google.com/store/apps/details?id=com.erplora.app` responde **404**.
3. Service account (invitada en Play Console con «Release to testing tracks») → secret
   `PLAY_SERVICE_ACCOUNT_JSON` + Variable `PLAY_PACKAGE_NAME` **en el repo `ERPlora/hub`**: las
   Variables de la organización NO llegan a un repo privado con el plan Free. En el MISMO repo y
   **antes** de poner `PLAY_PACKAGE_NAME`: Variables `PLAY_REVIEWER_EMAIL` y `PLAY_REVIEWER_HUB` +
   secret `PLAY_REVIEWER_PASSWORD` (los valores del `.env`). El job corre la guardia del paso 0
   antes de subir, y sin ellas cada tag sale en rojo `missing_credentials` (hub#1888).
4. Quitar **`play`** de `RELEASE_CHANNELS_PENDING` — hacerlo antes deja los tags en rojo.
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
- **Sin declaración de FGS.** ADR-0196 §2 retira el *foreground service*: la app no corre en
  segundo plano. El `AndroidManifest.xml` de `gen/android` es la fuente — consúltalo en vez de
  fiarte de una lista aquí.

## Cableado

| Pieza | Dónde |
| --- | --- |
| Build del AAB | este repo, [`tauri-release.yml`](.github/workflows/tauri-release.yml) job `build-android` |
| Subida a Object Storage | ídem, job `upload-s3` |
| Publicación en Play | ídem, job **`publish-play`** — gateado por la Variable `PLAY_PACKAGE_NAME` (vacía = saltado) |
| Redirect 302 a Play | SaaS `apps/public/downloads.py::store_url_for` + setting `GOOGLE_PLAY_APP_ID` |

## Lo que YA está hecho

No se repite en «Orden que queda» porque no queda: se deja escrito para que nadie lo rehaga.

1. **Ficha única `com.erplora.app`** creada en Play Console.
2. **Primera subida manual** del AAB al track `internal` — es la que registra la upload key
   (keystore ADR-0053) y activa Play App Signing. Con ella se quemó el `versionCode` `1000000`.
3. **Formularios**: Data safety · content rating IARC · privacy policy · target audience · ads ·
   screenshots + feature graphic 1024×500 + icono 512×512. Los 10, completos.
4. **Job `publish-play`** cableado en este repo, con el AAB firmado y verificado (`jarsigner`) y
   `changesNotSentForReview: false` (hub#984) para que el tag envíe a revisión de verdad.

⚠️ **La promoción `internal` → `production` sigue siendo MANUAL, y automatizarla no es solo
cuestión de confianza.** `publish-play` sube con `status: completed`, o sea **rollout al 100% de
golpe**: poner `PLAY_TRACK=production` creyendo que se remata la automatización mandaría cada tag,
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
