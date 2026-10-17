# Super Pong data tables

These tables accompany the [game specification](../SPEC-superpong.md). Update
both together when changing the palette or score glyphs. The data tables take
precedence if a copied value in the specification disagrees.

| File | Contents |
|---|---|
| `colour_model.json` | Packaging-art palette anchors interpolated in OKLab, monochrome luma levels, and carrier texture constants. The frontend checks its constants against this file. |
| `digits.json` | Score glyphs 0–9, the tens bar, and their raster positions. |

X is the console's pixel clock column; row is the scan line. The playfield box
spans X 0..176 and rows 0..224, using half-open ranges.

Core golden fixtures are kept separately in
[`game/superpong-core/testdata/`](../../game/superpong-core/testdata/), generated
by the independent [Python mirror](../../tools/superpong-mirror/).
