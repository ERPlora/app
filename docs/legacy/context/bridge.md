> Copia histórica de `erplora-old/architecture/context/bridge.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Contexto de dominio: Bridge (hardware POS)

> 🪦 **SUBSISTEMA RETIRADO — ADR-0196 (ejecutado hub#339/#340, 2026-08-08).** El Bridge standalone
> no existe: hay **una sola** app instalable (`erplora-app`) que ES el acceso al hardware, con
> `hub/crates/peripherals` in-process y la cola de impresión en el Hub. El WS `:12321` fue borrado.
> **No cargues este fichero como contexto de una tarea** — usa
> [`tauri.md`](tauri.md). Esto se conserva solo como historia.

> **Fuente canónica:** [`../bridge/README.md`](../bridge/README.md) +
> [`../bridge/hardware-protocol.md`](../bridge/hardware-protocol.md) +
> [`../bridge/websocket-interface.md`](../bridge/websocket-interface.md). Si discrepan, manda la fuente.

> **STUB (ratchet).** Distilar: puente local de hardware (impresoras de red ESC/POS, cajón),
> **red-only**; crate Rust `hub/crates/peripherals`, consumido **in-process** por la app instalable
> (`hub/apps/tauri`, `invoke`). 🪦 El standalone `hub/apps/bridge` (Axum WS:12321) y la app Kotlin
> se **borraron** con ADR-0196 (hub#340, 2026-08-08): no hay WebSocket local ni puerto que abrir.
> Rellenar leyendo la fuente canónica antes de usar en un pack.
