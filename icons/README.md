# Icon

The app icon is a chrome lightning bolt with a neon halo. The shipped version is
`src-tauri/icons/icon.svg` (the "I-crt" background below); `src-tauri/icons/` holds the
generated set.

## Regenerating the set

The SVG uses nothing the macOS renderer lacks (no filters: the halo is stacked translucent
strokes). Render it with the Swift script here, then let Tauri cut the sizes:

```sh
swift icons/render.swift src-tauri/icons/icon.svg /tmp/icon-1024.png 1024
bun tauri icon /tmp/icon-1024.png -o src-tauri/icons
rm -rf src-tauri/icons/android src-tauri/icons/ios   # macOS only
```

## Ideas

`ideas/` keeps every background explored for the bolt (same bolt in each, only the
background differs), with contact sheets `sheet-A-E.png` and `sheet-F-J.png`:

| File | Background |
| --- | --- |
| `0-sunset.svg` | The first version: sliced 80s sun over a magenta perspective grid |
| `A-void.svg` | Near-black with a magenta and cyan haze |
| `B-steel.svg` | Brushed dark metal, corner rivets, neon edge light |
| `C-gradient.svg` | Hot magenta to cyan with faint speed lines |
| `D-stars.svg` | Starfield over a cyan laser grid |
| `E-rays.svg` | Pink sunburst from behind the bolt |
| `F-checker.svg` | Magenta checkerboard floor under a dark sky |
| `G-hex.svg` | Dark hex mesh with a cyan underglow |
| `H-memphis.svg` | Pink, teal and gold Memphis geometry |
| `I-crt.svg` | Teal to magenta screen glow, scanlines, vignette (**shipped**) |
| `J-storm.svg` | Purple cloud bank with distant cyan bolts |

`sheet.swift` lays rendered PNGs out as a labelled contact sheet:

```sh
for v in A-void B-steel; do swift icons/render.swift icons/ideas/$v.svg /tmp/$v.png 360; done
(cd /tmp && swift ~/devjon/projects/ternt/icons/sheet.swift sheet.png A-void B-steel)
```
