# Brand marks (launcher footer)

`discord.png`, `github.png`, `patreon.png` — 48×48 RGBA8, **white RGB on
transparent alpha**. Embedded with `include_bytes!` by
`src/view_native/launcher_ui/footer.rs` and tinted per-brand at runtime via
`ImageNode::color`, which multiplies. A white mask is what makes that multiply
land on the exact brand hex; a non-white mask silently produces the wrong color,
so `brand_icons_decode_to_spec` in `footer.rs` asserts it.

These are **not** game assets. Nothing under `FFXI_DAT_PATH` is involved.

## Provenance

Source: [simple-icons](https://github.com/simple-icons/simple-icons) —
`icons/{discord,github,patreon}.svg`, retrieved 2026-07-30.

The simple-icons SVG files are CC0-1.0. Regenerate with:

```sh
curl -sfLO "https://raw.githubusercontent.com/simple-icons/simple-icons/master/icons/discord.svg"
# set fill="#FFFFFF" on the path, then rasterize to 48×48 RGBA
```

## Trademarks

The marks themselves are trademarks of Discord Inc., GitHub Inc., and Patreon
Inc. respectively; CC0 covers the SVG files, not the underlying marks. Each
brand's guidelines permit the mark for linking to that service, in either a
single flat color or the brand's own color. We render each in its own brand
color (`#5865F2`, `#FFFFFF`, `#FF424D`). Do not recolor one brand's mark to
another brand's palette.
