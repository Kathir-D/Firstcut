# Asset attribution

Every icon, font, image or other asset committed under `assets/` must have an
entry here, added in the same commit that adds the asset
(docs/v2-plan.md §4.5: no proprietary assets ever). `cargo xtask ci` checks
this mechanically and fails on any unattributed file.

| Asset | Source | License | Added by |
| --- | --- | --- | --- |
| `crates/firstcut-decode/tests/fixtures/tiny.jpg` | Generated locally with `sips` from macOS's own `MakePDF.app/0blank.jpg` (a blank white 1×1), resampled to 8×8 | (generated test fixture; no rights asserted) | Kathir Dev |
| `fonts/Inter-Regular.ttf` | [Inter](https://rsms.me/inter/) 4.1, `extras/ttf/Inter-Regular.ttf` | SIL Open Font License 1.1 (`fonts/Inter-LICENSE.txt`) | Kathir Dev |
| `fonts/Inter-Medium.ttf` | [Inter](https://rsms.me/inter/) 4.1, `extras/ttf/Inter-Medium.ttf` | SIL Open Font License 1.1 (`fonts/Inter-LICENSE.txt`) | Kathir Dev |
| `fonts/Inter-SemiBold.ttf` | [Inter](https://rsms.me/inter/) 4.1, `extras/ttf/Inter-SemiBold.ttf` | SIL Open Font License 1.1 (`fonts/Inter-LICENSE.txt`) | Kathir Dev |
| `fonts/Inter-LICENSE.txt` | Inter 4.1 release archive, `LICENSE.txt` | SIL Open Font License 1.1 | Kathir Dev |
| `branding/firstcut-1024.png` | Original mark drawn for Firstcut (a crop bracket around an F) | GPL-3.0-only, same as the app | Kathir Dev |
| `branding/firstcut-128.png`, `firstcut-256.png`, `firstcut-512.png` | Downscaled from `firstcut-1024.png` | GPL-3.0-only | Kathir Dev |
| `branding/firstcut.icns` | macOS iconset built from `firstcut-1024.png` with `iconutil` | GPL-3.0-only | Kathir Dev |
| `branding/firstcut.ico` | Windows icon built from `firstcut-1024.png` | GPL-3.0-only | Kathir Dev |

The UI bundles Inter (D10) so macOS, Windows and Linux draw the same glyphs and
the snapshot tolerance does not have to absorb three system fonts. The Lucide
icon set is still optional; the shell uses text labels and egui's built-in
shapes, so no icon font is embedded yet.
