#!/usr/bin/env python3
"""
Generates the Tauri v2 APP icon set of ERPlora.

From ONE brand asset (a high-resolution PNG, 1024×1024 recommended, with the mark centred and
margins) it composes a real **application icon** — not the flat navbar logo — and writes the full
set Tauri v2 expects in `src-tauri/icons/`:

  · icon.png                 (1024, base, square corners)
  · icon.ico                 (Windows: 16/24/32/48/64/128/256)
  · icon.icns                (macOS: Retina set with Big Sur rounded corners)
  · 32x32.png · 128x128.png · 128x128@2x.png   (Linux/dev)
  · Square{30,44,71,89,107,142,150,284,310}x...Logo.png   (Windows Store / MSIX)
  · StoreLogo.png            (50×50)

Composition of an APP icon (not a flat logo):
  - A solid brand background by default (full-bleed fill): app icons must not be transparent on
    most platforms (Windows tile, iOS, macOS Big Sur cuts its own silhouette on an opaque
    background).
  - Safe area / padding: the mark takes ~`--mark-scale` of the canvas, centred, so neither the
    Android/iOS mask nor the macOS rounded edge eats into it.
  - Shape per platform:
      · `.icns` (macOS) → Big Sur rounded corners (radius ~22.37% + ~10% margin, a squircle
        approximated with rounded_rectangle).
      · `icon.png` / Square*Logo (Windows) → full-bleed square (Windows applies its own tile
        mask).
      · Linux → square.

No SVG rasterizer is required: it works on PNG. If the asset is `.svg` and `cairosvg` is
installed, it rasterizes it; otherwise it asks for a PNG.

Usage:
    python3 scripts/gen-tauri-icon.py [SOURCE] [--out DIR] [--store-assets-only]

  SOURCE defaults to branding/app-icon-source.png
  (see branding/README.md — the FINAL brand art goes there).
  --out DIR             another destination (default: src-tauri/icons).
  --store-assets-only   writes ONLY the 3 PNG a Package.appxmanifest references
                        (Square44x44Logo, Square150x150Logo, StoreLogo), for the MSIX
                        Assets, without the ico/icns/full set.

Only dependency: Pillow (`pip install Pillow`). Reproducible.
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:  # pragma: no cover
    sys.exit(
        "Pillow is missing. Install it:\n"
        "  python3 -m venv .venv && . .venv/bin/activate && pip install Pillow\n"
        "  python3 scripts/gen-tauri-icon.py"
    )

ROOT = Path(__file__).resolve().parents[1]
ICONS_DIR = ROOT / "src-tauri/icons"
DEFAULT_SOURCE = ROOT / "branding/app-icon-source.png"

# ── Composition of the APP icon ─────────────────────────────────────────────
# Brand background (hub blue #0091CE). The final art may bring its own background: a source PNG
# that is already opaque to the edge is kept as it is, and BACKGROUND only fills under the mark
# when the source is transparent.
BACKGROUND = (0, 145, 206)
# Share of the canvas the mark takes (safe area). 0.72 leaves ~14% margin per side: it survives
# the Android/iOS mask and the macOS rounding.
MARK_SCALE = 0.72
# Corner radius (share of the side) of the rounded icons (macOS).
MACOS_CORNER_RADIUS = 0.2237  # ~ the Big Sur squircle
MACOS_CONTENT_INSET = 0.10  # the extra margin macOS leaves around the tile

SS = 4  # supersampling for clean edges and masks

# ── Sizes of the Tauri v2 set ───────────────────────────────────────────────
PNG_SIZES = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 1024,
}
# Windows Store / MSIX (full bleed, not rounded: Windows applies its mask)
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


# ── Loading the source asset ────────────────────────────────────────────────
def load_source(path: Path) -> Image.Image:
    if not path.exists():
        sys.exit(
            f"The source asset does not exist: {path.relative_to(ROOT)}\n"
            "Put the brand art there (PNG 1024×1024, the mark centred with a margin).\n"
            "See branding/README.md."
        )
    if path.suffix.lower() == ".svg":
        try:
            import cairosvg  # type: ignore
        except ImportError:
            sys.exit(
                "The asset is an SVG and no rasterizer (cairosvg) is installed.\n"
                "Option A (recommended): export the SVG to a 1024×1024 PNG and use it as the source.\n"
                "Option B: pip install cairosvg (it pulls in the native cairo)."
            )
        import io

        png_bytes = cairosvg.svg2png(
            url=str(path), output_width=1024, output_height=1024
        )
        return Image.open(io.BytesIO(png_bytes)).convert("RGBA")
    return Image.open(path).convert("RGBA")


def _has_alpha_content(img: Image.Image) -> bool:
    """True when the source has real transparency (it needs the brand background)."""
    if img.mode != "RGBA":
        return False
    alpha = img.getchannel("A")
    lo, _hi = alpha.getextrema()
    return lo < 250


# ── Composition ─────────────────────────────────────────────────────────────
def compose(
    src: Image.Image,
    size: int,
    *,
    rounded: bool = False,
    mark_scale: float = MARK_SCALE,
    inset: float = 0.0,
) -> Image.Image:
    """Composes the `size×size` app icon from the source mark.

    rounded → macOS-style rounded corners (plus the content inset).
    A solid brand background unless the source is already opaque to the edge.
    """
    S = size * SS
    needs_bg = _has_alpha_content(src)

    canvas = Image.new(
        "RGBA", (S, S), BACKGROUND + (255,) if needs_bg else (0, 0, 0, 0)
    )

    # The usable tile after the inset (the margin the platform leaves around it).
    tile = int(round(S * (1.0 - 2 * inset)))
    tile_off = (S - tile) // 2

    # A source opaque to the edge fills the whole tile; a mark with alpha is centred at
    # mark_scale inside the tile (safe area).
    if needs_bg:
        bg_tile = Image.new("RGBA", (tile, tile), BACKGROUND + (255,))
        canvas.alpha_composite(bg_tile, (tile_off, tile_off))
        mark_span = int(round(tile * mark_scale))
        mark = src.resize((mark_span, mark_span), Image.LANCZOS)
        m_off = tile_off + (tile - mark_span) // 2
        canvas.alpha_composite(mark, (m_off, m_off))
    else:
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
    print(f"→ source: {source.relative_to(ROOT) if ROOT in source.parents else source}")
    print(
        f"→ destination: {ICONS_DIR.relative_to(ROOT) if ROOT in ICONS_DIR.parents else ICONS_DIR}"
    )

    if args.store_assets_only:
        for name in ("Square44x44Logo.png", "Square150x150Logo.png", "StoreLogo.png"):
            save(compose(src, SQUARE_LOGOS[name]), name)
        print("OK — MSIX Assets (manifest) written.")
        return

    # Full-bleed square PNGs (Linux/dev + base); Windows applies its mask.
    for name, size in PNG_SIZES.items():
        save(compose(src, size), name)

    for name, size in SQUARE_LOGOS.items():
        save(compose(src, size), name)

    # ICO (Windows): every size in one file.
    ico_base = compose(src, 256)
    ico_path = ICONS_DIR / "icon.ico"
    ico_base.save(ico_path, sizes=[(s, s) for s in ICO_SIZES])
    print(f"  {ico_path.relative_to(ROOT)}  ({'/'.join(map(str, ICO_SIZES))})")

    # ICNS (macOS): Big Sur rounded corners, every size.
    icns_frames = [
        compose(src, s, rounded=True, inset=MACOS_CONTENT_INSET) for s in ICNS_SIZES
    ]
    icns_path = ICONS_DIR / "icon.icns"
    # Pillow writes ICNS from the largest image plus append_images with the other sizes.
    largest = icns_frames[-1]
    largest.save(
        icns_path,
        format="ICNS",
        append_images=icns_frames[:-1],
    )
    print(f"  {icns_path.relative_to(ROOT)}  ({'/'.join(map(str, ICNS_SIZES))})")

    print("OK — Tauri app icon set written.")


if __name__ == "__main__":
    main()
