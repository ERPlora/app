> Copia histórica de `erplora-old/architecture/desktop-mobile/android.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Android — el objetivo VOLVIÓ, dentro de la app Tauri (no como Bridge Kotlin)

> ⛔ **Lo que sigue retirado** ([ADR-0154](../adr/ADR-0154.md), 2026-07-25): el producto
> «Hub Local» con runtime embebido + SQLite. Ese Android no vuelve.
>
> ⭐ **Corrección — Android SÍ es un objetivo hoy, pero de la app Tauri, no de un Bridge Kotlin
> aparte.** [ADR-0180](../adr/ADR-0180.md) (2026-08-01) devolvió Android a
> `tauri-release.yml` como **la misma app** `com.erplora.app` (identidad única desde
> [ADR-0160](../adr/ADR-0160.md)), y [ADR-0204](../adr/ADR-0204.md)
> (2026-08-06) le añadió el **SPP Bluetooth** dentro de un plugin Tauri propio
> (`tauri-plugin-erplora-android`). El **Bridge Android en Kotlin** (el artefacto separado que este
> doc señalaba) es el que quedó retirado de verdad, en hub#340 (2026-08-08); su repo
> `ERPlora-Bridge-android` sigue sin archivar (pm#52, abierta) — es historia, no la vía actual.

ERPlora es **un solo producto: Hub Cloud** (PWA). El artefacto instalable en Android es la **app
Tauri** (`com.erplora.app`) — ver [`../hub/apps/tauri.md`](../hub/apps/tauri.md). El Bridge Kotlin
histórico está en [`../bridge/android-app.md`](../bridge/android-app.md) y
[`../hub/apps/bridge.md`](../hub/apps/bridge.md) (ambos marcados como retirados).
Decisiones: [ADR-0154](../adr/ADR-0154.md) (retira Hub Local) ·
[ADR-0180](../adr/ADR-0180.md)/[ADR-0204](../adr/ADR-0204.md)
(traen Android de vuelta en la app Tauri).
