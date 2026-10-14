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
bash scripts/tests/test-shell-workflow.test.sh
bash scripts/release-gate.test.sh
bash scripts/pack-msix.test.sh
bash scripts/tests/apt-lock-retry.test.sh
bash scripts/tests/setup-android-packages.test.sh
bash scripts/tests/play-reviewer-preflight.test.sh
bash scripts/tests/play-reviewer-preflight-on-publish.test.sh
cargo test -p erplora-peripherals --test hardware_smoke -- --ignored --nocapture   # a mano, con impresora real
```

En CI, `test-shell.yml` corre el `cargo test` en Linux, Windows y macOS y, solo en Linux, su propio
contrato, las reglas del `release-gate` y los tests Kotlin. Los otros cinco tests de scripts de la
lista de arriba los ejecutaba `actionlint.yml` del monorepo, que no se ha traído: hoy se corren a
mano y quedan para la adaptación de los workflows.

`tauri-release.yml` sigue tal cual (release con un tag `vX.Y.Z`), solo con las rutas corregidas; su
paso a «release al hacer push a `main`» es otro trabajo.

## Secrets y variables de la CI (solo nombres)

- **Secrets**: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `PLAY_SERVICE_ACCOUNT_JSON`,
  `PLAY_REVIEWER_PASSWORD`, `AZURE_AD_TENANT_ID`, `AZURE_AD_APPLICATION_CLIENT_ID`,
  `AZURE_AD_APPLICATION_SECRET`, `SELLER_ID`, `HETZNER_S3_ACCESS_KEY`, `HETZNER_S3_SECRET_KEY`,
  `HETZNER_S3_ENDPOINT` (y el `GITHUB_TOKEN` automático).
- **Variables**: `CI_RUNNER_LABEL`, `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER`, `MSIX_PUBLISHER_DISPLAY`,
  `MICROSOFT_STORE_PRODUCT_ID`, `PLAY_PACKAGE_NAME`, `PLAY_TRACK`, `PLAY_REVIEWER_EMAIL`,
  `PLAY_REVIEWER_HUB`, `RELEASE_CHANNELS_PENDING`.

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
