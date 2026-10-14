> Copia histórica de `erplora-old/architecture/desktop-mobile/README.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Desktop & Mobile — el producto «Hub Local» sigue RETIRADO; el empaquetado Tauri VOLVIÓ (ADR-0159/0196)

> ⛔ **Lo que sigue retirado, por [ADR-0154](../adr/ADR-0154.md) (2026-07-25):** el
> producto **«Hub Local»** — runtime embebido + **SQLite** + funcionamiento offline. Eso no vuelve.
>
> ⭐ **Lo que NO sigue retirado — corrección, léase antes de asumir nada de este folder:**
> [ADR-0159](../adr/ADR-0159.md) (2026-07-26) trajo de vuelta el **empaquetado** de
> escritorio/Android/iOS con **Tauri**, y [ADR-0196](../adr/ADR-0196.md)
> (2026-08-03) lo convirtió en el **único camino al hardware** (impresoras, cajón, SPP Bluetooth de
> Android): una app fina (`com.erplora.app`, identidad única desde ADR-0160) cuya webview carga la
> **misma PWA de Hub Cloud** — sin runtime local, sin BD local, sin entitlement gate propio — y
> resuelve hardware por `invoke` sobre `crates/peripherals`. El **Bridge standalone** (el binario
> Axum en `:12321` + su app Kotlin + el pairing por device-code, que este folder documentaba como
> «el único binario instalable») se **borró después**, en hub#340 (2026-08-08): hoy no hay Bridge
> ninguno, hay la app Tauri. El repo `ERPlora-Bridge-android` sigue sin archivar (pm#52, abierta).

ERPlora es **un solo producto: Hub Cloud** — PWA + runtime Axum sobre **Postgres**. El binario
instalable es la **app Tauri** (`com.erplora.app`), no un Bridge aparte.

Los conceptos de este folder se documentan en su sitio vivo, bajo `architecture/hub/`:

| Tema | Dónde vive ahora |
|------|------------------|
| App Tauri (shell fino, hardware por `invoke`, Android/escritorio) | [`../hub/apps/tauri.md`](../hub/apps/tauri.md) |
| Shell / UI del Hub (PWA que carga la webview) | [`../hub/apps/web.md`](../hub/apps/web.md) |
| Bridge de hardware standalone — **histórico, borrado en hub#340** | [`../hub/apps/bridge.md`](../hub/apps/bridge.md) |
| Entitlement firmado + tiers de módulo (`free`/`premium`) + bloqueo de módulos de pago | [`../hub/module-system.md`](../hub/module-system.md) §2bis/§4/§5.8 · [`../hub/auth.md`](../hub/auth.md) · [`../saas/monetization.md`](../saas/monetization.md) |
| Decisión de retirar «Hub Local» (SQLite, offline) | [decision-log — ADR-0154](../adr/ADR-0154.md) |
| Decisión de traer de vuelta el empaquetado Tauri | [decision-log — ADR-0159](../adr/ADR-0159.md) · [ADR-0196](../adr/ADR-0196.md) |
