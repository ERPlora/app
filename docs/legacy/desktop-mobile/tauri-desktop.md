> Copia histórica de `erplora-old/architecture/desktop-mobile/tauri-desktop.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Tauri Desktop — el producto viejo murió (ADR-0154); el empaquetado volvió como cliente fino (ADR-0159)

> ⛔ **Lo que sigue retirado** ([ADR-0154](../adr/ADR-0154.md), 2026-07-25): el producto
> «Hub Local» — runtime embebido en `apps/tauri/` + SQLite + funcionamiento offline. El runtime del
> Hub es y sigue siendo **Postgres-only** (hub#210).
>
> ⭐ **Corrección — `apps/tauri/` no está borrado: resucitó como CLIENTE FINO**
> ([ADR-0159](../adr/ADR-0159.md), 2026-07-26, implementado en hub#225). Es una app de
> escritorio (Windows/macOS/Linux) cuya webview carga el onboarding del SaaS y después la **misma
> PWA de Hub Cloud** — sin runtime propio, sin BD local, sin entitlement gate: el hardware
> (impresoras, cajón) lo resuelve por `invoke` sobre `crates/peripherals`, no por un proceso
> separado. Identidad única `com.erplora.app` desde [ADR-0160](../adr/ADR-0160.md).

ERPlora es **un solo producto: Hub Cloud** — PWA sobre **Postgres**; el binario instalable de
escritorio es esta **app Tauri** de cliente fino, no un Bridge aparte (el Bridge standalone se borró
en hub#340, ver [`../hub/apps/bridge.md`](../hub/apps/bridge.md), histórico). Detalle as-built:
[`../hub/apps/tauri.md`](../hub/apps/tauri.md). Ver
[decision-log — ADR-0154](../adr/ADR-0154.md) (qué murió) ·
[ADR-0159](../adr/ADR-0159.md) (qué volvió) y [`README.md`](README.md).
