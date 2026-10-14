#!/usr/bin/env python3
"""
ERPlora · pipeline de generación del set de iconos de APP de Tauri v2.

A partir de UN único asset de marca (PNG de alta resolución, recomendado 1024×1024
con la marca centrada y márgenes), compone un **icono de aplicación** real —no el
logo plano de navbar— y emite el set completo que Tauri v2 espera en
`src-tauri/icons/`:

  · icon.png                 (1024, base, esquinas planas)
  · icon.ico                 (Windows: 16/24/32/48/64/128/256)
  · icon.icns                (macOS: set Retina con esquinas redondeadas estilo Big Sur)
  · 32x32.png · 128x128.png · 128x128@2x.png   (Linux/dev)
  · Square{30,44,71,89,107,142,150,284,310}x...Logo.png   (Windows Store / MSIX)
  · StoreLogo.png            (50×50)

Composición de icono de APP (no logo plano):
  - Fondo sólido de marca por defecto (relleno full-bleed): los iconos de app no
    deben ser transparentes en la mayoría de plataformas (Windows tile, iOS, macOS
    Big Sur recorta su propia silueta sobre fondo opaco).
  - Safe-area / padding: la marca ocupa ~`--mark-scale` del lienzo, centrada, para
    que ni el recorte de máscara de Android/iOS ni el borde redondeado de macOS la
    coman.
  - Forma por plataforma:
      · `.icns` (macOS) → esquinas redondeadas estilo Big Sur (radio ~22.37% +
        margen ~10%, "squircle" aproximado con rounded_rectangle).
      · `icon.png` / Square*Logo (Windows) → cuadrado a sangre (Windows aplica su
        propia máscara de tile).
      · Linux → cuadrado.

Sin rasterizador SVG obligatorio: trabaja sobre PNG. Si el asset es `.svg` y
`cairosvg` está instalado, lo rasteriza; si no, pide un PNG (mensaje claro).

Uso:
    python3 scripts/gen-tauri-icon.py [SOURCE] [--out DIR] [--store-assets-only]

  SOURCE por defecto: branding/app-icon-source.png
  (ver branding/README.md — ahí va el arte de marca DEFINITIVO).
  --out DIR             destino alternativo (default: src-tauri/icons).
  --store-assets-only   emite SOLO los 3 PNG que referencia un Package.appxmanifest
                        (Square44x44Logo, Square150x150Logo, StoreLogo) — para los
                        Assets MSIX del bridge (hub#120), sin ico/icns/set completo.

Dependencia única: Pillow (`pip install Pillow`). Reproducible.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:  # pragma: no cover
    sys.exit(
        "Falta Pillow. Instálalo:\n"
        "  python3 -m venv .venv && . .venv/bin/activate && pip install Pillow\n"
        "  python3 scripts/gen-tauri-icon.py"
    )

ROOT = Path(__file__).resolve().parents[1]
ICONS_DIR = ROOT / "src-tauri/icons"
DEFAULT_SOURCE = ROOT / "branding/app-icon-source.png"

# ── Parámetros de composición del icono de APP ──────────────────────────────
# Fondo de marca (azul hub V5 = #0091CE). El arte definitivo del humano puede
# traer su propio fondo: si el PNG fuente ya es opaco a sangre, se respeta tal cual
# (BACKGROUND solo se usa para rellenar bajo la marca cuando el fuente es
# transparente).
BACKGROUND = (0, 145, 206)  # #0091CE
# Fracción del lienzo que ocupa la marca (safe-area). 0.72 deja ~14% de margen por
# lado: aguanta el recorte de máscara de Android/iOS y el redondeo de macOS.
MARK_SCALE = 0.72
# Radio de esquina (fracción del lado) para los iconos con forma redondeada (macOS).
MACOS_CORNER_RADIUS = 0.2237  # ~ squircle de Big Sur
MACOS_CONTENT_INSET = 0.10  # margen extra que macOS deja alrededor del tile

SS = 4  # supersampling para bordes/máscaras limpios

# ── Tamaños del set Tauri v2 ────────────────────────────────────────────────
PNG_SIZES = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 1024,
}
# Windows Store / MSIX (a sangre, sin redondear: Windows pone su máscara)
SQUARE_LOGOS = {
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024]


# ── Carga del asset fuente ──────────────────────────────────────────────────
def load_source(path: Path) -> Image.Image:
    if not path.exists():
        sys.exit(
            f"No existe el asset fuente: {path.relative_to(ROOT)}\n"
            "Pon el arte de marca (PNG 1024×1024, marca centrada con margen) ahí.\n"
            "Ver branding/README.md."
        )
    if path.suffix.lower() == ".svg":
        try:
            import cairosvg  # type: ignore
        except ImportError:
            sys.exit(
                "El asset es SVG y no hay rasterizador (cairosvg) instalado.\n"
                "Opción A (recomendada): exporta el SVG a PNG 1024×1024 y úsalo como fuente.\n"
                "Opción B: pip install cairosvg (arrastra cairo nativo)."
            )
        import io

        png_bytes = cairosvg.svg2png(
            url=str(path), output_width=1024, output_height=1024
        )
        return Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    return Image.open(path).convert("RGBA")


def _has_alpha_content(img: Image.Image) -> bool:
    """True si el fuente tiene transparencia real (necesita fondo de marca)."""
    if img.mode != "RGBA":
        return False
    alpha = img.getchannel("A")
    lo, _hi = alpha.getextrema()
    return lo < 250


# ── Composición ─────────────────────────────────────────────────────────────
def compose(
    src: Image.Image,
    size: int,
    *,
    rounded: bool = False,
    mark_scale: float = MARK_SCALE,
    inset: float = 0.0,
) -> Image.Image:
    """Compón el icono de app `size×size` desde la marca fuente.

    rounded → esquinas redondeadas estilo macOS (+ inset de contenido).
    Fondo de marca sólido salvo que el fuente ya sea opaco a sangre.
    """
    S = size * SS
    needs_bg = _has_alpha_content(src)

    # lienzo de fondo
    canvas = Image.new(
        "RGBA", (S, S), BACKGROUND + (255,) if needs_bg else (0, 0, 0, 0)
    )

    # tile útil tras inset (margen que la plataforma deja alrededor)
    tile = int(round(S * (1.0 - 2 * inset)))
    tile_off = (S - tile) // 2

    # si el fuente es opaco a sangre, ocupa todo el tile; si es marca con alpha,
    # se centra al mark_scale dentro del tile (safe-area).
    if needs_bg:
        # rellenar el tile con el fondo de marca
        bg_tile = Image.new("RGBA", (tile, tile), BACKGROUND + (255,))
        canvas.alpha_composite(bg_tile, (tile_off, tile_off))
        mark_span = int(round(tile * mark_scale))
        mark = src.resize((mark_span, mark_span), Image.LANCZOS)
        m_off = tile_off + (tile - mark_span) // 2
        canvas.alpha_composite(mark, (m_off, m_off))
    else:
        # fuente ya opaco a sangre: escálalo al tile completo
        full = src.resize((tile, tile), Image.LANCZOS)
        canvas.alpha_composite(full, (tile_off, tile_off))

    if rounded:
        radius = int(round(S * MACOS_CORNER_RADIUS))
        mask = Image.new("L", (S, S), 0)
        md = ImageDraw.Draw(mask)
        md.rounded_rectangle(
            [tile_off, tile_off, tile_off + tile - 1, tile_off + tile - 1],
            radius=radius,
            fill=255,
        )
        # antialias del borde
        mask = mask.filter(ImageFilter.GaussianBlur(SS * 0.4))
        out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        out.paste(canvas, (0, 0), mask)
        canvas = out

    return canvas.resize((size, size), Image.LANCZOS)


def save(img: Image.Image, name: str) -> None:
    path = ICONS_DIR / name
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)
    print(f"  {path.relative_to(ROOT)}  ({img.width}×{img.height})")


def main() -> None:
    import argparse

    global ICONS_DIR

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", nargs="?", default=str(DEFAULT_SOURCE))
    parser.add_argument("--out", default=str(ICONS_DIR))
    parser.add_argument("--store-assets-only", action="store_true")
    args = parser.parse_args()

    ICONS_DIR = Path(args.out).resolve()
    source = Path(args.source).resolve()
    src = load_source(source)
    print(f"→ fuente: {source.relative_to(ROOT) if ROOT in source.parents else source}")
    print(
        f"→ destino: {ICONS_DIR.relative_to(ROOT) if ROOT in ICONS_DIR.parents else ICONS_DIR}"
    )

    if args.store_assets_only:
        for name in ("Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png"):
            save(compose(src, SQUARE_LOGOS[name]), name)
        print("OK — Assets MSIX (manifest) generados.")
        return

    # PNGs cuadrados a sangre (Linux/dev + base) — Windows aplica su máscara
    for name, size in PNG_SIZES.items():
        save(compose(src, size), name)

    # Windows Store / MSIX (a sangre)
    for name, size in SQUARE_LOGOS.items():
        save(compose(src, size), name)

    # ICO (Windows) — multi-size en un fichero
    ico_base = compose(src, 256)
    ico_path = ICONS_DIR / "icon.ico"
    ico_base.save(ico_path, sizes=[(s, s) for s in ICO_SIZES])
    print(f"  {ico_path.relative_to(ROOT)}  ({'/'.join(map(str, ICO_SIZES))})")

    # ICNS (macOS) — esquinas redondeadas estilo Big Sur, multi-size
    icns_frames = [
        compose(src, s, rounded=True, inset=MACOS_CONTENT_INSET) for s in ICNS_SIZES
    ]
    icns_path = ICONS_DIR / "icon.icns"
    # Pillow guarda ICNS desde la imagen mayor + append_images con los tamaños.
    largest = icns_frames[-1]
    largest.save(
        icns_path,
        format="ICNS",
        append_images=icns_frames[:-1],
    )
    print(f"  {icns_path.relative_to(ROOT)}  ({'/'.join(map(str, ICNS_SIZES))})")

    print("OK — set de iconos de app Tauri generado.")


if __name__ == "__main__":
    main()
