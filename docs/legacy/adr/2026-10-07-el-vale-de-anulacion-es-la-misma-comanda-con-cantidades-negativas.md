> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-10-07-el-vale-de-anulacion-es-la-misma-comanda-con-cantidades-negativas.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## Nota de mantenimiento · El vale de anulación de cocina es la misma comanda `kitchen_order`, con «ANULADA» en la etiqueta y las cantidades en negativo — 2026-10-07
- **Estado:** aceptada
- **Contexto:** cancelar una ronda ya enviada (a mano en Cocina → Comandas, KITCHEN-F22, o porque se
  eliminó la cuenta, KITCHEN-F28) solo quitaba la tarjeta de la pantalla: en una estación que
  trabaja con la comanda impresa el papel seguía en el riel y el plato se preparaba (kitchen#168).
  Toast, Square y Lightspeed sacan un «void chit» en la misma impresora. El renderizador ESC/POS vive
  en las `erplora-app` instaladas, que no se actualizan con el hub.
- **Decisión:** el vale lo imprime **el shell** (`apps/web/src/lib/print-void.ts`, HUB_SHELL-F78),
  escuchando `kitchen.order.cancelled` y por el **mismo camino que la comanda** (`buildComandaGroups`,
  `comandaRoute`, documento `kitchen_order`). **No se crea un tipo de documento ni un campo nuevo**:
  una app vieja ignoraría el campo y sacaría el vale como una comanda NUEVA (el plato se cocinaría
  dos veces). El «anulado» se dice con lo que una app vieja ya imprime: `label` pasa a
  «ANULADA · Mesa 4» (doble alto) y cada `items[].quantity` va en negativo («-2x Croquetas»); nunca
  lleva `priority`. Clave de cola propia `kitchen-void-<pedido>-<rol>` para que la cola no lo tome por
  repetido de la comanda. El módulo `kitchen` no cambia código: sus líneas canceladas siguen en
  `kitchen.orders.items` y el aviso ya salía en los dos caminos (ERPlora/hub#2629, ERPlora/kitchen#171).
- **Consecuencias:** el vale sale en toda app instalada, también en las que no se actualicen. El
  renderizador tiene que **conservar el signo** de la cantidad (fijado por
  `escpos::tests::a_void_slip_prints_the_word_and_negative_quantities`). Los límites son los de la
  comanda: con la impresora de red apagada o sin papel el vale no sale y nadie lo sabe (hub#2494), y
  una cancelación sin caja con todas las pantallas cerradas no la encola nadie. Anular un plato suelto
  (kitchen#161) y cancelar con confirmación y motivo (kitchen#166) quedan fuera.
- **Docs afectados:** `hub/crates/peripherals.md` (§ La comanda de cocina: dos productores).
