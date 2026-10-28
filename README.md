# ERPlora App

App nativa de ERPlora para **Windows, Android, Linux y macOS**, hecha con **Tauri 2** (Rust + Kotlin).
Es un **cliente fino**: la ventana abre el SaaS (`erplora.com/shell/`), captura a qué hub apunta la
instalación y desde ahí abre ese hub. No lleva runtime ni base de datos local. Lo que aporta es lo
nativo de la caja: impresión ESC/POS (red, cola USB del SO en escritorio, Bluetooth SPP en Android),
cajón, NFC, avisos del sistema, descargas, impresión A4 y enlaces `erplora://`. Identificador
`com.erplora.app`.

## De dónde viene

Traída del monorepo antiguo `ERPlora/hub` (hoy `erplora-old/hub`), rama `origin/develop`, commit
`9dd3821573017ea8e162cf9bbb9eb597739c3aed`, con `git archive` (sin historial). La raíz de este repo es
lo que allí era `apps/tauri/`. El README anterior de la app está en ese commit, en `apps/tauri/README.md`.

| Aquí | En el monorepo |
| --- | --- |
| `src-tauri/`, `shell-dist/`, `package.json`, `*.md`, `branding/`, `workflow/` | `apps/tauri/…` |
| `crates/peripherals/`, `crates/tauri-plugin-erplora-android/` | `crates/…` (igual) |
| `Cargo.toml` (workspace de 3 miembros) | `Cargo.toml` raíz: solo `[workspace.lints]` y `[workspace.package]` |
| `Cargo.lock` | el del monorepo, podado por Cargo a lo que usan estos crates (mismas versiones) |
| `scripts/`, `.github/workflows/` | los scripts y los dos workflows de la app (`tauri-release.yml`, `test-shell.yml`) |

**Documentación histórica**: lo que se sabía de la app fuera de `apps/tauri` (contexto, diseño y
las ADR que el código cita por número, del repo `erplora-old/architecture`) está copiado en
[`docs/legacy/`](docs/legacy/README.md).

## Cómo se compila

Requisitos: Rust stable (`rust-toolchain.toml`), Node 22 con pnpm (`corepack enable`; versión fijada en
`packageManager`) y la WebView del sistema (WebView2 en Windows, WKWebView en macOS; en Linux
`libwebkit2gtk-4.1-dev` y `libgtk-3-dev`, lista completa en `tauri-release.yml`).

```bash
pnpm install --frozen-lockfile        # solo instala el CLI de Tauri (no hay frontend que construir)
pnpm exec tauri build                 # bundles de escritorio; empaqueta shell-dist/ tal cual
cargo tauri android build --apk --aab # Android: JDK 17, SDK 36, NDK 27.1.12297006 y targets de Android
```

## Cómo se prueba

```bash
cargo test --workspace --no-fail-fast      # los tres crates (erplora-tauri, plugin Android, peripherals)
scripts/kotlin-plugin-tests.sh             # tests Kotlin del plugin: JDK 17 + platforms;android-36, tras un cargo build
for t in scripts/*.test.sh scripts/tests/*.test.sh; do bash "$t"; done   # contratos de los workflows y scripts de CI (PyYAML)
actionlint -shellcheck= -pyflakes=         # los workflows (versión fijada en CI: 1.7.12)
cargo test -p erplora-peripherals --test hardware_smoke -- --ignored --nocapture   # a mano, con impresora real
```

En CI, `test-shell.yml` tiene dos jobs: `test` corre el `cargo test` en Linux, Windows y macOS y, solo
en Linux, los tests Kotlin; `scripts` corre `actionlint` y **todos** los `scripts/*.test.sh` y
`scripts/tests/*.test.sh` por glob (en el monorepo cinco de ellos solo los corría `actionlint.yml`,
que no se trajo). Se dispara en push a `main` y en PR cuando cambia algo de la app, de `scripts/` o
de un workflow, y lo **llama** `tauri-release.yml` antes de construir nada.

## CI/CD

**Una release es un push a `main` con una versión nueva en `src-tauri/tauri.conf.json`.** La versión
vive solo ahí; una versión publicada se marca con su tag `v<versión>`, que crea la propia release al
terminar. No hay que crear tags a mano.

`tauri-release.yml` corre en cada push a `main` (y a mano, `gh workflow run tauri-release.yml -R
ERPlora/app`; con `-f publish=false` solo construye). **Solo publica desde `refs/heads/main`**
(igualdad exacta): lanzado a mano en otra rama (`--ref feat/x`) construye y avisa con un
`::notice::`, pero no publica ni crea el tag. Su primer job, `version` (`scripts/release-version.sh`),
decide:

- el tag `v<versión>` **ya existe** → todo lo demás se salta y el resumen del run lo dice (un push de
  docs o de CI no publica nada);
- **no existe** y la versión es mayor que el último tag `vX.Y.Z` → tests (`test-shell.yml`, llamado)
  → builds (escritorio ×3 y Android; cada plataforma sale en rojo si le falta uno de sus bundles) →
  `collect` (están los seis instaladores y el AAB, y abre la **GitHub Release en borrador**
  `v<versión>` con los instaladores) → Microsoft Store, Google Play (hoy apagado: `PLAY_PACKAGE_NAME`
  borrada y `play` pendiente; ver `GOOGLE-PLAY.md`) y S3 → `release-gate` → publica el borrador, que
  es lo que crea el tag `v<versión>`;
- **existe un borrador `v<versión>` abierto por otra run** → **rojo**: esa publicación ya empezó
  (ver «Si una release se queda a medias»);
- **no existe** y no es mayor (o no es `X.Y.Z`, o minor/patch ≥ 1000) → **rojo** con el motivo: la
  Store y Play solo aceptan versiones crecientes y el `versionCode` es `major·10⁶ + minor·10³ + patch`.

Para publicar: subir `version` en `src-tauri/tauri.conf.json` (hoy 1.1.31, publicada el 2026-10-27; la
siguiente es 1.1.32 o mayor) y hacer push a `main`. **La versión se sube a mano**, nada la incrementa
solo, y ese push es lo único que hace falta: un push a `main` sin cambio de versión no publica nada.

Condición de la Store: en Partner Center no debe haber ningún envío en curso. El que abre la CI es
suyo y se cierra solo al certificarse; uno abierto a mano (un borrador) hace fallar `publish-store`,
como el 2026-10-27 (`MICROSOFT-STORE.md`, Fase 3).

Una release a la vez para todas las ramas: el grupo de concurrencia es `tauri-release`, sin la ref, y
un push nunca cancela una release en curso. GitHub guarda **una** run pendiente por grupo: si llega
otra, la pendiente anterior se cancela (la nueva, si es de `main`, lleva sus commits; si una run de
otra rama desplaza un push de `main` con versión nueva, se relanza con
`gh workflow run tauri-release.yml -R ERPlora/app --ref main`).

**Si una release se queda a medias** (p. ej. Play y S3 publicados y la Store en rojo): el tag no
existe todavía, pero sí el borrador `v<versión>`, que `collect` abre antes del primer canal y que
lleva el id de su run. Desde ahí:

1. Lo normal es **relanzar esa misma run**: `gh run rerun <id> --failed` (los artefactos duran 3 días).
   Repite solo lo que falló; S3 acepta sobrescribir porque `latest.json` lleva el mismo `run-id`.
2. Cualquier otra run de la misma versión (otro push a `main`, un dispatch) sale en **rojo** en
   `version`: «version X already started publishing in run N … gh run rerun N --failed … or bump».
   No recompila otro código ni lo vuelve a publicar con el mismo número.
3. Si hay que cambiar código: **subir la versión** y borrar el borrador viejo, que no tiene tag
   (`gh release delete v<versión> -R ERPlora/app`, solo si nunca llegó a publicarse).
4. Segunda capa, en S3: `upload-s3` no sobrescribe `downloads/app/v<versión>/` si su `latest.json`
   lo escribió otra run (`scripts/ci/s3-version-guard.sh`, comprueba el metadato `run-id`).

`release-gate` (`scripts/release-gate.sh`) pone la release en rojo si un canal no publicó, salvo que
esté declarado en la Variable `RELEASE_CHANNELS_PENDING` (hoy solo `play`: S3, enchufado desde el
2026-10-14, y la Store ya son obligatorios); entonces pasa en verde con un aviso. Un
canal declarado ni siquiera corre: su job se salta aunque sus Variables estén puestas (app#7), así que
un canal pendiente no puede poner la release en rojo con un fallo. Al enchufar un canal se ponen
primero sus Variables y secrets y después se quita de la lista; quitarlo es lo que lo enciende.

**S3 (AWS, OIDC, sin claves estáticas).** `upload-s3` asume el rol `AWS_DOWNLOADS_ROLE_ARN` y escribe
en `DOWNLOADS_BUCKET` las claves de siempre: `downloads/app/v<versión>/…` y `downloads/app/latest/…`,
con `downloads/app/latest/latest.json` al final (lo que lee el SaaS en `GET /api/v1/app/release/`).
El rol de aws-next debe confiar **solo** en los runs de `main` de este repo: sujeto OIDC
`repo:ERPlora/app:ref:refs/heads/main`. Ojo: la organización emite sujetos con id inmutable, así que
el que llega de verdad es
`repo:ERPlora@183305526/app@1418874207:ref:refs/heads/main` (org `ERPlora` = 183305526, repo
`ERPlora/app` = 1418874207); la condición de confianza tiene que usar ese.
aws-next creó el bucket y el rol el 2026-10-14 (PLAN-AWS, ronda 18; `stacks/prod/s3_downloads.tf`):

- Bucket `erplora-prod-downloads` (`eu-west-1`): privado, sin nada público. El SaaS lee `latest.json`
  con el rol de su tarea y sirve cada descarga con una URL prefirmada de 300 s.
- Rol `arn:aws:iam::363427305647:role/erplora-prod-app-release`. Confía solo en el sujeto inmutable de
  `main` de este repo.
- Permisos del rol: `s3:ListBucket` en el bucket, y `GetObject`, `GetObjectTagging`, `PutObject` y
  `AbortMultipartUpload` en `downloads/app/*`. Sin borrar.
- Variables del repo, puestas ese mismo día. Si hubiera que rehacerlas:

```bash
gh variable set -R ERPlora/app AWS_DOWNLOADS_ROLE_ARN --body arn:aws:iam::363427305647:role/erplora-prod-app-release
gh variable set -R ERPlora/app DOWNLOADS_BUCKET --body erplora-prod-downloads
gh variable set -R ERPlora/app RELEASE_CHANNELS_PENDING --body play   # `s3` y `store` ya no están pendientes
```

Sin las dos Variables, el job avisa («S3 upload skipped: AWS_DOWNLOADS_ROLE_ARN/DOWNLOADS_BUCKET not set»)
y los instaladores solo van a la GitHub Release. Con `s3` fuera de `RELEASE_CHANNELS_PENDING`,
`release-gate` lo pone en rojo.

**Secrets que hay que cargar** (no se pueden leer del repo antiguo; ninguno existe aún en
`ERPlora/app`). Cada comando pide el valor por la entrada estándar:

```bash
# Android (obligatorios para CUALQUIER release: sin ellos build-android sale en rojo nombrándolos)
gh secret set -R ERPlora/app ANDROID_KEYSTORE_BASE64      # base64 de la keystore (alias erplora, ADR-0053)
gh secret set -R ERPlora/app ANDROID_KEYSTORE_PASSWORD
# Microsoft Store (MICROSOFT-STORE.md, fase 3)
gh secret set -R ERPlora/app AZURE_AD_TENANT_ID
gh secret set -R ERPlora/app AZURE_AD_APPLICATION_CLIENT_ID
gh secret set -R ERPlora/app AZURE_AD_APPLICATION_SECRET
gh secret set -R ERPlora/app SELLER_ID
# Google Play (GOOGLE-PLAY.md, paso 3)
gh secret set -R ERPlora/app PLAY_SERVICE_ACCOUNT_JSON < service-account.json
gh secret set -R ERPlora/app PLAY_REVIEWER_PASSWORD
```

Y las Variables que encienden cada tienda (sin ellas su job se salta y cuenta como pendiente):

```bash
gh variable set -R ERPlora/app MICROSOFT_STORE_PRODUCT_ID --body <Store ID>   # tras la 1.ª publicación manual
gh variable set -R ERPlora/app PLAY_PACKAGE_NAME --body com.erplora.app
gh variable set -R ERPlora/app PLAY_REVIEWER_EMAIL --body <email del revisor>
gh variable set -R ERPlora/app PLAY_REVIEWER_HUB --body <slug del hub del revisor>
gh variable set -R ERPlora/app PLAY_TRACK --body internal                      # opcional; internal por defecto
```

Un job de tienda que corre sin uno de sus secrets sale en **rojo nombrándolo**
(`scripts/ci/require-config.sh`), nunca se salta en silencio.

**Runner.** `CI_RUNNER_LABEL` es una Variable de la organización `ERPlora` (`self-hosted`) con
visibilidad solo para repos privados: `ERPlora/app` es público, no la ve, y todos los jobs Linux caen
en `ubuntu-latest` (lo correcto en un repo público: un runner propio no debe ejecutar código de PRs
ajenas). No se ha copiado al repo.

Las acciones de terceros (todo lo que no es `actions/*`) van fijadas por SHA de commit, con la
versión en un comentario; el contrato lo exige en todos los workflows. Para subir una: resolver el SHA
del tag nuevo (`git ls-remote https://github.com/<owner>/<repo> 'refs/tags/<tag>^{}'`) y cambiar
SHA y comentario a la vez.

Contratos: `scripts/tests/tauri-release-workflow.test.sh`, `scripts/tests/test-shell-workflow.test.sh`,
`scripts/release-version.test.sh`, `scripts/release-gate.test.sh`, `scripts/tests/require-config.test.sh`,
`scripts/tests/s3-version-guard.test.sh` y los tests Rust `release_gate.rs`,
`release_runner_assumptions.rs`, `app_update_channel.rs`.

## Secrets y variables de la CI (solo nombres)

- **Secrets**: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `PLAY_SERVICE_ACCOUNT_JSON`,
  `PLAY_REVIEWER_PASSWORD`, `AZURE_AD_TENANT_ID`, `AZURE_AD_APPLICATION_CLIENT_ID`,
  `AZURE_AD_APPLICATION_SECRET`, `SELLER_ID` (y el `GITHUB_TOKEN` automático). Los de Hetzner
  (`HETZNER_S3_*`) ya no se usan: S3 va por OIDC.
- **Variables**: `AWS_REGION` (`eu-west-1`), `AWS_DOWNLOADS_ROLE_ARN`, `DOWNLOADS_BUCKET`,
  `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER`, `MSIX_PUBLISHER_DISPLAY`, `MICROSOFT_STORE_PRODUCT_ID`,
  `PLAY_PACKAGE_NAME`, `PLAY_TRACK`, `PLAY_REVIEWER_EMAIL`, `PLAY_REVIEWER_HUB`,
  `RELEASE_CHANNELS_PENDING`, `CI_RUNNER_LABEL` (de la organización, no visible aquí).

Ningún secreto vive en el repo: la keystore y `keystore.properties` los escribe el job de Android
desde los secrets, y `.gitignore` excluye `*.jks`, `*.keystore`, `keystore.properties` y
`google-services.json`.

## Cambios hechos al traerla

Solo rutas y layout; el comportamiento de la app no cambia (comandos nativos, `/shell/`, captura
`?shell=1`, capabilities, `SAAS_HOSTS`, `PLAY_SAAS_PAGES`, esquema `erplora://`, identificador y versión
siguen igual).

- `src-tauri/Cargo.toml`: dependencias de ruta a `../crates/peripherals` y
  `../crates/tauri-plugin-erplora-android`.
- `package.json`: añade `"packageManager": "pnpm@10.0.0"` (antes lo fijaba el `package.json` raíz del
  monorepo) y `pnpm-lock.yaml` propio (solo `@tauri-apps/cli` 2.11.4). `.npmrc` con el registry público.
- `rust-toolchain.toml`: `stable` sin el target `wasm32-unknown-unknown`, que era de los guests WASM del hub.
- `tauri-release.yml`: `projectPath: .`, `src-tauri/tauri.conf.json`, `src-tauri/gen/android`; el build
  de Android corre desde la raíz.
- `test-shell.yml`: `paths` del repo nuevo (`src-tauri/**`, `shell-dist/**`, `crates/peripherals/**`,
  `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `package.json`, `pnpm-lock.yaml`,
  `scripts/kotlin-plugin-tests.sh`), sin el disparador de `apps/web/src/theme/variables.css`, y el
  `cargo test` incluye `-p erplora-peripherals`: en el monorepo lo probaba `test-hub.yml`, que no se ha
  traído, y sin esto sus tests no correrían en ningún sitio. Su contrato
  (`scripts/tests/test-shell-workflow.test.sh`) exige ahora esas rutas y ese `-p`.
- `scripts/`: rutas de `pack-msix.ps1`, `pack-msix.test.sh`, `kotlin-plugin-tests.sh` y
  `gen-tauri-icon.py` sin el prefijo `apps/tauri/`.
- Tests Rust que leían `../../..` (raíz del monorepo) ahora leen `..` (raíz de este repo).

Después (2026-10-14), la CI/CD de este repo (sección «CI/CD»): release al hacer push a `main` con
compuerta de versión en vez de tags `v*`, S3 de AWS por OIDC en vez de Hetzner, tag y GitHub Release
al final, secrets de las tiendas comprobados con nombre, y `test-shell.yml` llamado por la release
con un job `scripts` (actionlint + todos los tests de scripts). `tauri.conf.json` pasa de `1.0.1` (el
valor que el workflow antiguo sobrescribía con el tag) a `1.1.30`, la última versión publicada desde
el monorepo (`erplora-old/hub`, tag `v1.1.30` → `ce584178`). El tag `v1.1.30` de este repo la marca
como publicada; va anotado sobre el commit que trae el workflow nuevo y esa versión, no sobre
`f252e71`: el `tauri-release.yml` de `f252e71` aún se disparaba con tags `v*` y habría lanzado una
release con el flujo antiguo.

## Tests retirados

Leían el frontend web del monorepo (`apps/web`), que no existe aquí: la SPA nueva vive en `hub-next`
(`packages/ui`) y el puente JS aún no está portado. Se quitó **solo** la aserción sobre el fichero web;
lo que se puede comprobar en este repo sigue. El port del puente JS en `hub-next` debe recrearlas.

- `src-tauri/tests/app_update_channel.rs`, test `the_page_and_the_shell_name_the_same_command`
  (retirado entero): `apps/web/src/lib/app-update.ts` invoca `plugin:app|version` y
  `erplora_bridge_status`.
- `src-tauri/tests/notice_tap.rs`, test `the_page_and_the_shell_name_the_same_event_and_argument`:
  `apps/web/src/main.ts` contiene `listenTauriPlugin('notification', 'actionPerformed', cb)` y
  `apps/web/src/lib/bridge-transport.ts` contiene
  `invokeTauri('erplora_notify', { title, body, id, path })`. Siguen las tres aserciones sobre `lib.rs`.
- `src-tauri/tests/notice_tap.rs`, test `the_page_claims_the_tap_the_shell_kept`: `apps/web/src/main.ts`
  contiene `take: () => invokeTauri('erplora_take_notice_tap')` y
  `onPoke: (cb) => listenTauriEvent('erplora://notice-tapped', cb)`. Sigue la de `notice_tap.rs`.
- `src-tauri/tests/android_status_bar_inset.rs`, tests de color de la franja bajo el reloj (hub#1903):
  el `assert_eq!` entre `android:windowBackground` (`values` y `values-night`) y
  `--ion-toolbar-background` de `apps/web/src/theme/variables.css` (`:root` y
  `:root.ion-palette-dark`). Sigue la mitad Android (el tema declara un `@color/` opaco); los tests se
  llaman ahora `the_strip_under_the_clock_is_an_opaque_theme_colour_in_{light,dark}_mode`.
- `src-tauri/tests/android_status_bar_inset.rs`, test
  `the_colour_tests_run_when_the_header_colour_changes` (retirado entero): `test-shell.yml` se disparaba
  en los PR que tocaban `apps/web/src/theme/variables.css`.

## Tests adaptados

- `crates/peripherals/tests/arquitectura_names_the_live_transports_hub1562.rs` leía `ARQUITECTURA.md`
  del monorepo (§2.7). Ahora guarda `crates/peripherals/README.md`, que tiene la misma tabla de
  transportes (`network:{ip}:{port}`, `bluetooth:{mac}`, `usb:{queue}`): ningún transporte vivo puede
  darse por descartado en el documento que se lee antes de tocar la impresión.
- `scripts/tests/apt-lock-retry.test.sh`: su lista de pasos con apt conocidos ya no incluye
  `test-web.yml` ni `visual-baselines.yml`, que se quedaron en el monorepo.
- `crates/peripherals/src/queue.rs` (tests): en el monorepo este crate solo se probaba en Linux; aquí
  `test-shell.yml` lo corre también en Windows, donde dos tests fallaron en el primer run
  (`38564104951`). `a_single_attempt_policy_does_not_retry` exigía < 10 ms y en Windows un `connect`
  rechazado en loopback tarda ~1 s: ahora se mide contra el backoff (10 s), que es lo que delataría un
  reintento. `send_once_gives_up_on_a_printer_that_accepts_but_never_reads` se ignora **solo en
  Windows**: Winsock acepta los 16 MiB del envío no bloqueante en sus búferes aunque el otro extremo no
  lea, así que no hay atasco que cortar; el plazo es el mismo código en todos los sistemas y Linux y
  macOS lo siguen ejercitando.
- `scripts/tests/test-shell-workflow.test.sh`: las dos aserciones «el paso `Release gate rules` corre
  en Linux» y «un paso corre este contrato en Linux» pasan a «el job `scripts` corre todos los
  `scripts/*.test.sh` y `scripts/tests/*.test.sh` por glob, y el job `test` ya no los repite». Nuevas:
  `workflow_call`, `scripts/**` en los `paths`, `github.workflow` en el grupo de concurrencia y
  actionlint fijado.
- `src-tauri/tests/release_gate.rs`: el gate ya no se acota a `startsWith(github.ref, 'refs/tags/v')`
  sino a `needs.version.outputs.release == 'true'`; «el gate necesita todos los demás jobs» excluye
  ahora el único job que va detrás de él (`github-release`, el del tag), y un test nuevo exige que el
  tag solo se cree ahí y con el gate en verde.
- `src-tauri/tests/app_update_channel.rs`: el paso que escribe `latest.json` se llama
  `Upload installers and latest.json to S3` (antes `Upload to Hetzner Object Storage`), y se exige que
  vaya al bucket de AWS (`vars.DOWNLOADS_BUCKET`) y no a Hetzner.
- Revisión del 2026-10-14 (publicar solo desde `main`, release a medias, instaladores ausentes):
  en `release_gate.rs` el job del tag ya no hace `gh release create`, publica el borrador
  (`gh release edit … --draft=false`, que es lo que crea el tag), y fuera de él un `gh release create`
  solo vale con `--draft`. En `tauri-release-workflow.test.sh`, los jobs que publican necesitan
  `collect` (que necesita los dos builds) en vez de los builds directamente; «`github-release` es el
  único con `contents: write`» pasa a «solo `version` (para ver borradores), `collect` y
  `github-release`»; y la acción de credenciales de AWS se reconoce por nombre, fijada por SHA de la
  línea v4, en vez de por `@v4`.
- `crates/peripherals/src/queue.rs`, test `a_printer_that_comes_back_still_gets_its_job`: la ventana
  para que la impresora vuelva era de 8 intentos × 20 ms (~160 ms) y en macOS, con ~200 tests en
  paralelo en 3 núcleos, la impresora volvía más tarde y el trabajo fallaba (run `38573290656`); se
  subió a 50 intentos (~1 s). Volvió a caer en macOS (app#3, run `39903050741`) por otra causa: allí
  la impresora apagada descarta el SYN en vez de rechazarlo, así que el primer intento se quedaba
  pendiente y entraba con la retransmisión (el test pasaba con una cola que no reintenta), o su plazo
  de 1 s chocaba con esa retransmisión y una conexión vacía gastaba el único `accept` de la
  impresora del test, que cerraba el puerto. Ahora la impresora está apagada más que el plazo de un
  intento (300 ms frente a 100 ms), vuelve antes de agotar los intentos por el orden de los
  temporizadores (todo en el mismo hilo del test, no por velocidad) y atiende todas las conexiones
  hasta recibir el trabajo, como una real; el test llama antes con una conexión vacía para
  encontrarse siempre ese caso.
