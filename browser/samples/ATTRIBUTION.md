# Westeros sample — attribution

`westeros.zql` (and the `westeros-*.csv` it loads) is an **unofficial fan
graph** built from *A Song of Ice and Fire* / *Game of Thrones*. It is not
affiliated with, endorsed by, or produced by HBO, George R. R. Martin, or any
rights holder in that work. The underlying world is © George R. R. Martin.

## Story facts

Characters, houses, family ties, allegiances, seats, deaths, battles and
weddings are extracted as facts (not copied prose) from **A Wiki of Ice and
Fire** (awoiaf.westeros.org), which publishes its content under the
[Creative Commons Attribution-ShareAlike 3.0 Unported licence](https://awoiaf.westeros.org/index.php/Special:Copyright)
(CC BY-SA 3.0). Every `Character`, `House`, `Location`, `Region` and `Event`
node in the dataset carries a `source` field: the AWOIAF page path the facts
were drawn from. Because the dataset is built from CC BY-SA-licensed content,
it is offered here under the same licence, **CC BY-SA 3.0**.

Pages were read through the Wayback Machine's archived copies of
awoiaf.westeros.org (the live site's Cloudflare challenge blocks a plain
fetch); see `../scripts/westeros-fetch.mjs`.

## Map positions

Every `Location` carries `x`/`y` on a shared 0–1000 grid (x west→east, y
north→south) — an original geography, never traced from any official map,
promotional art, or house sigil.

- The 67 anchor places are copied from the **westeros-map draft** (the paused
  sibling project's `places.json`, "known-world" frame). That draft is zega's
  own geography, placed by the story's relative descriptions.
- Other places are placed by hand relative to those anchors, or fitted onto
  them per region by the build script from hand-reads off fan maps, and
  cross-checked against them for consistency rather than measured off any
  single map:
  - *A Song of Ice and Fire: Speculative World Map* (v1.0, Feb 2012) by
    **theMountainGoat** (sermountaingoat.co.uk) and **Tear** of the
    Cartographer's Guild, based in part on a speculative map by Werthead —
    licensed [CC BY-NC-SA 3.0 Unported](https://creativecommons.org/licenses/by-nc-sa/3.0/),
    world © George R. R. Martin. Used here only as a reference for relative
    position; no part of the map image, its art, or its tiles is included in
    this repository.
  - quartermaester.info's interactive map, consulted for a second,
    independent read of the same relative positions. It carries no licence of
    its own (all rights reserved); nothing from it is reproduced here either —
    only compared against, the same way the fan map above was.

## Not included

This dataset does not include any HBO or George R. R. Martin artwork, video,
photography, or logos, and does not reproduce the official Westeros map. Show-
only facts (deaths that happen on screen but not on the page, for example)
are deliberately absent.
