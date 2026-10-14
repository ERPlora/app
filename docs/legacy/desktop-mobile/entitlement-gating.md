> Copia histórica de `erplora-old/architecture/desktop-mobile/entitlement-gating.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

# Entitlement & gating — reubicado (ADR-0154)

> ⛔ **El marco «app Tauri GRATIS con gate de arranque propio» de este doc quedó retirado por
> [ADR-0154](../adr/ADR-0154.md) (2026-07-25)** — ERPlora es **un solo producto, Hub
> Cloud** sobre Postgres, y esa app gratuita con su propio entitlement local no vuelve. ⚠️ Sí volvió
> una app Tauri, pero de otra forma: cliente fino sin runtime ni BD propios
> ([ADR-0159](../adr/ADR-0159.md), ver [`README.md`](README.md)), así que no hay «gate
> de arranque» que reubicar — la webview carga la PWA de Hub Cloud, que ya trae su propio
> entitlement. El **mecanismo de entitlement por tiers sigue vivo** para los módulos del Hub Cloud,
> igual que antes de esta nota.

El entitlement y los tiers de módulo (`free`/`premium`) viven ahora en su sitio canónico:

| Tema | Dónde |
|------|-------|
| Tiers de módulo (`free`/`premium`, ADR-0032) + monetización | [`../saas/monetization.md`](../saas/monetization.md) · `is_module_entitled` en `saas/apps/public/modules/entitlement.py` |
| Token de entitlement firmado (RS256) + claims (`plan`/`max_devices`, sesión única) | [`../hub/auth.md`](../hub/auth.md) |
| Bloqueo de módulos de pago en el dispatcher (revalidación híbrida, ADR-0114) + capabilities | [`../hub/module-system.md`](../hub/module-system.md) §2bis/§5.8 |
| Decisión cloud-first | [decision-log — ADR-0154](../adr/ADR-0154.md) |
