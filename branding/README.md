# branding — asset fuente del icono de app

Aquí vive el **único asset de marca** del que se deriva todo el set de iconos de la
app de escritorio/móvil (Tauri v2).

## Cómo regenerar los iconos de la app

1. Pon el **arte de marca definitivo** aquí como `app-icon-source.png`
   (PNG **1024×1024**, marca centrada con margen; transparente o con su propio fondo).
   - Si tienes SVG, expórtalo a PNG 1024×1024 (el pipeline acepta SVG solo si
     `cairosvg` está instalado; el PNG es el camino recomendado y sin dependencias
     nativas).
2. Corre el pipeline desde la raíz del repo `hub/`:

   ```bash
   # una vez: entorno con Pillow
   python3 -m venv .venv && . .venv/bin/activate && pip install Pillow

   python3 scripts/gen-tauri-icon.py
   # o con una fuente concreta:
   python3 scripts/gen-tauri-icon.py branding/app-icon-source.png
   ```

   Esto regenera **todo** `src-tauri/icons/` (lo que referencia
   `tauri.conf.json`).

## Qué genera el pipeline (`scripts/gen-tauri-icon.py`)

Set completo que Tauri v2 espera, con **composición de icono de app** (no el logo
plano de navbar):

| Fichero | Plataforma | Forma |
|---|---|---|
| `icon.png` (1024) | base / Linux | cuadrado a sangre |
| `32x32.png`, `128x128.png`, `128x128@2x.png` | Linux / dev | cuadrado |
| `icon.ico` (16/24/32/48/64/128/256) | Windows | cuadrado (Windows pone su máscara) |
| `icon.icns` (16…1024) | macOS | esquinas redondeadas estilo Big Sur + inset |
| `Square{30,44,71,89,107,142,150,284,310}x...Logo.png` | Windows Store / MSIX | cuadrado |
| `StoreLogo.png` (50) | Windows Store | cuadrado |

Composición aplicada (parámetros arriba del script):

- **Fondo de marca sólido** (`BACKGROUND`, por defecto azul hub `#0091CE`) bajo la
  marca cuando el fuente es transparente — los iconos de app no deben ser
  transparentes (Windows tile, iOS, macOS recortan sobre fondo opaco).
- **Safe-area / padding**: la marca ocupa `MARK_SCALE` (~72 %) del lienzo, centrada,
  para aguantar el recorte de máscara de Android/iOS y el redondeo de macOS.
- **Forma por plataforma**: solo `.icns` se redondea (squircle Big Sur,
  `MACOS_CORNER_RADIUS` + `MACOS_CONTENT_INSET`); Windows y Linux van a sangre y la
  plataforma aplica su propia máscara de tile.

## Set provisional actual (PENDIENTE: arte del humano)

El `app-icon-source.png` actual es **provisional**: lo genera
`scripts/gen-v5-source-png.py` rasterizando la marca V5 (hub central azul + 8
módulos rojo/verde/amarillo), el mismo layout que
`media/generate-v5-icons.py` del monorepo. Sirve para que `tauri.conf.json` apunte a
iconos válidos y la app **compile/arranque** mientras el diseño no esté.

> **Decisión de marca = de Ioan (fundador/marca), no de la política de quién escribe código.**
> Un icono de app dedicado NO es
> el logo de navbar: el hub central azul sobre fondo azul casi desaparece (se ve como
> un cuadrado vacío), y un icono real querría más contraste / composición propia.
> Sustituye `app-icon-source.png` por el arte definitivo y vuelve a correr el pipeline.

Para regenerar el provisional desde la marca V5 (si borras el PNG):

```bash
python3 scripts/gen-v5-source-png.py     # → app-icon-source.png
python3 scripts/gen-tauri-icon.py        # → src-tauri/icons/*
```
