# Asset attribution

Every icon, font, image or other asset committed under `assets/` must have an
entry here, added in the same commit that adds the asset
(docs/v2-plan.md §4.5: no proprietary assets ever). `cargo xtask ci` checks
this mechanically and fails on any unattributed file.

| Asset | Source | License | Added by |
| --- | --- | --- | --- |

Nothing yet — the egui shell at M0 uses no bundled assets. The first assets
(D10: the Inter font family and the Lucide icon set, both permissively
licensed) land with the themed UI work.
