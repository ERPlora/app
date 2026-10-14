> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-10-08-el-vale-de-un-plato-anulado.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## ADR-NNNN · El vale de anulación de UN plato: por línea de venta y con etiqueta propia — 2026-10-08
- **Estado:** aceptada
- **Contexto:** desde kitchen#161 Cocina tacha en su pantalla el plato que el TPV anula
  (`kitchen.item.voided`, una por línea de cocina), pero en papel solo salía vale si la ronda se
  quedaba vacía (el de la ronda entera, decisión «Cocina tacha el plato que el TPV anula»): una
  cocina que trabaja con la comanda impresa no se enteraba (ERPlora/hub#2640, P1). Toast imprime un
  «VOID» del artículo en la impresora de su estación; Square y Lightspeed, igual.
- **Decisión:** el shell (`apps/web/src/lib/print-void.ts`) escucha también `kitchen.item.voided` y
  saca un vale por **línea de venta** (`sales_order_item_id`: un menú anulado entero es un solo
  vale por rol con sus componentes, sin los ya servidos), con el mismo documento `kitchen_order` de
  siempre (cantidades negativas, sin campo nuevo: las apps instaladas viejas lo pintan igual) y la
  etiqueta **«PLATO ANULADO · Mesa 4»**, distinta de «ANULADA», que en papel se lee como la mesa
  entera. Clave de cola `kitchen-void-<pedido>-<línea de venta>-<rol>`. El vale de la ronda
  cancelada deja fuera lo ya `voided`: si el plato anulado era el último vivo, sale su vale y no otro
  de ronda encima. Misma ruta que el de la ronda (directo, por la cola, o nadie), mismos avisos con
  el nombre del plato.
- **Consecuencias:** se cierra la consecuencia «el vale en papel de UN plato no existe todavía» de
  la decisión de kitchen#161. Sigue el hueco común a la comanda: con la impresora de red apagada el
  vale no sale y nadie lo sabe (hub#2494).
- **Docs afectados:** `hub/crates/peripherals.md`, `modules/kitchen.md`, `modules/sales.md`,
  `workflows/restaurante-dia.md`
