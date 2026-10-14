> Copia histórica de `erplora-old/architecture/00-overview/decision-log.d/2026-10-01-ficha-de-google-play-publicada-el-interruptor-nace-encendido.md` @ `2113309c11a7201c8f61700d13008ba55f61db26`, 2026-10-14.
> Describe el producto antiguo; la referencia vigente es `erplora-next/PLAN*.md`.
> Índice, criterio de la copia y lo que no se copió: [`docs/legacy/README.md`](../README.md).

## ADR-NNNN · La ficha de Google Play está publicada: `GOOGLE_PLAY_LISTING_LIVE` nace encendido — 2026-10-01

- **Estado:** aceptada · saas#2430.
- **Contexto.** saas#1719 añadió `GOOGLE_PLAY_LISTING_LIVE` con `default=False` porque la ficha
  seguía en revisión y el default tenía que ser el que no miente. El 01/10/2026 la ficha
  `com.erplora.app` ya responde 200, pero ni `saas-web` de producción ni el de PRE definen la
  variable en Dokploy: los dos seguían enseñando «Muy pronto en Google Play».
- **Decisión.** El default pasa a `True`: `/app/download/android/` hace 302 a Play y el botón de
  `/app/` dice «Consíguela en Google Play» en todos los entornos sin tocar su env. La variable se
  queda como **interruptor de emergencia** (`False` solo si Google retira la ficha), y entonces
  vuelve la página propia `appdist/android.html`, que sigue cubierta por tests y por el gate visual.
- **Descartado.** (a) Poner la variable a mano en Dokploy: rompe la regla «todo por la API» y no
  existe endpoint para el env de `saas-web`; crear uno genérico para editar el env del propio SaaS
  abre más superficie de la que este cambio justifica. (b) Borrar el interruptor y la página: deja
  sin plan B el día que Google retire la ficha y vuelva el 404 heredado.
- **Consecuencias.** + Un despliegue basta, y el default vuelve a decir la verdad de hoy.
  − Un entorno nuevo apunta a Play aunque su app no esté publicada; hoy solo hay una ficha
  (ADR-0160), así que no aplica.
- **Docs afectados:** [`saas/public-portal.md`](../saas/public-portal.md).
