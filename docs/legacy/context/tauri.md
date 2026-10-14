> Copia histórica de `erplora-old/architecture/context/tauri.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Contexto de dominio: Tauri — shell cliente fino (ADR-0159; el producto Local sigue retirado)

> ⭐ **ADR-0196 (2026-08-03), EJECUTADO (hub#339 + hub#340, 2026-08-08).** Una **sola** app
> instalable (escritorio · Android · iOS) que ES el acceso al hardware. `hub/apps/bridge/`, el
> pairing device-code y `WsBridgeTransport` **ya no existen en el árbol**; la app Kotlin tampoco.
> Solo impresora de **red**; la cola vive en el Hub. Queda la mitad servidor en el SaaS
> (saas#1274).

> ⭐ **[ADR-0159](../adr/ADR-0159.md) (2026-07-26).** Tauri **vuelve solo como
> EMPAQUETADO**: el shell de escritorio (Win/macOS/Linux) es un **cliente fino** cuya webview carga el
> onboarding del SaaS y luego la PWA del hub cloud (`hub_url`), con hardware por `invoke` sobre
> `crates/peripherals`. Lo que [ADR-0154](../adr/ADR-0154.md) (2026-07-25) retiró — el
> producto «Hub Local»: runtime embebido + SQLite + offline — **sigue retirado**. Estado 0159: diseño
> aceptado, pendiente de implementación (`hub/apps/tauri/` se resucita del historial y se adelgaza).

En Android el shell-webview es **la misma app Tauri** (ADR-0180); el Bluetooth SPP vuelve dentro
del plugin propio `tauri-plugin-erplora-android` (ADR-0204). Para una tarea, carga el contexto vivo:

- **Shell Tauri fino (diseño):** [`../hub/apps/tauri.md`](../hub/apps/tauri.md).
- **Contexto del hardware:** [`bridge.md`](bridge.md) · [`../bridge/README.md`](../bridge/README.md).
  🪦 [`../hub/apps/bridge.md`](../hub/apps/bridge.md) es **historia** (componente borrado en hub#340).
- **Contexto del Hub (cloud):** [`hub.md`](hub.md).
- **Decisiones:** [decision-log — ADR-0159 · ADR-0154](../00-overview/decision-log.md).
