# Handoff: bring the GeoLibre STAC work to RustGIS

You are working in `D:\Data\RustGIS`. The author of this work (Ian Horn, `ianhorn`) built the STAC
features below in GeoLibre (TypeScript) in an earlier session and wants the same functionality in
RustGIS (Rust). This note is the whole context from that session. You have not seen it.

## Goal

GeoLibre issue [#3365](https://github.com/opengeos/GeoLibre/issues/3365) asks to extend native STAC
support beyond search by extent or bounding box. The same author wants it in RustGIS. The full wish
list from that issue:

1. Search by point, line, polygon, a selected feature, and a feature loaded from a file.
2. Inspect a STAC item (JSON or HTML), then add single or several assets, or band combinations.
3. Add a rendering such as true color, or an index such as NDVI.
4. Select several items and mosaic them.
5. For lidar results: filter by classification and return, and add as a VPC-like file.
6. Export a search to a script (Python, notebook, maybe an executable).

GeoLibre's PR covers 1 and 3 plus extras (below). Items 2 (inspect, multi-add), 4, 5 and 6 are not
done anywhere yet.

## RustGIS already has STAC. Read it first and extend it

- `crates/engine/src/cmd/stac.rs` (about 1,230 lines). Commands: `stac.collections`, `stac.search`,
  `stac.addFootprints`, `stac.addCube`, `stac.addAsset`, ribbon path `Home > Add > Web Data > STAC Search`.
- `stac.search` today takes `bbox` (or `"view"`), `datetime`, `limit`, `maxCloud`, `sortby` (a raw
  sort field string such as `"-datetime"`), `collections`, `url`, and more. **There is no shape
  (`intersects`) search, no buffer, no date-sort choice in the UI, and no band-combination or index
  rendering.** `maxCloud` and `sortby` exist at the command level, so check what the UI exposes.
- `crates/engine/src/cmd/cube_catalog.rs` and `cube.rs` already read STAC bands in place.
- UI lives in `crates/ui-egui/src/` (look at `cubesui.rs`, `connectionui.rs`, `chrome.rs`).
- `GEOLIBRE_PARITY.md` line ~57 tracks STAC as partial (#128). Update it in the PR that lands work.
- `ROADMAP.md` "Underway" must be checked before picking work.

## Rules you must follow (from `AGENTS.md` and `CONTRIBUTING.md`; read both in full)

- **Licence: do not copy or translate GPL code.** RustGIS is Apache-2.0 and `CONTRIBUTING.md` says a
  translation by an AI tool is still a copy. The author's **Kentucky STAC QGIS plugin
  (`D:\Data\qgis\kentucky-stac`) is GPL-2.0**. Do not port its code into Rust. Implement from the
  behaviour described in this note, which is a specification, and name the plugin in the PR as
  required ("If you followed another project's code ... name it in the pull request"). The author
  holds the copyright on their own plugin, so how to state that in the PR is the author's call;
  ask them. The GeoLibre code is MIT and also the author's. Formulas such as NDVI are public
  standards.
- No `unwrap`, `expect`, `panic!`, `todo!`, no `unsafe`. Input from files, URLs and command
  parameters is hostile: clamp sizes, bound recursion. Keep `hostile_params_never_panic` green and
  add regression tests.
- **Never silent:** anything dropped, truncated or approximated is reported through
  `Session::notify` (one notice per kind with a count). Test both ways (lossy input gives the
  notice, clean input none).
- Must build for `wasm32-unknown-unknown`: use `web_time`, file access through `Services`.
- Everything is a command (`CommandSpec` in `crates/engine/src/cmd/*.rs`); commands never open
  dialogs; UI-only commands (`ui.*`) live in `crates/ui-egui/src/lib.rs`. Follow the ribbon
  guidelines. Wrap UI text in `tl!(...)` and use `i18n::fmt`; do not edit the catalogs.
- Before each commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, layers). A new dependency
  needs a licence `deny.toml` allows, then `cargo xtask notices`.
- **Coordinate first** (open a proposal issue and wait for a maintainer) if you add a dependency or
  crate, or change an existing command's id or parameters. Prefer adding optional parameters over
  changing existing ones. Draft PRs are welcome early. Screenshots go to the PR, not the repo.

## The behaviour to build (the specification)

### Search by shape
- A shape is sent as the STAC `intersects` parameter (GeoJSON geometry) and **replaces `bbox`**
  (Item Search makes them mutually exclusive). GET fallback sends `intersects` as JSON in the query.
- Sources of a shape: drawn point, line or polygon on the map; the selected features of a layer; a
  file (GeoJSON, zipped Shapefile; RustGIS reads more formats natively, use them).
- Merging several features or shapes into one: keep **only the highest dimension present**
  (polygon, then line, then point) and count the inputs left out so the user is told (a notice).
  Dissolve (union) polygons so overlaps give a valid geometry; points and lines become the Multi
  geometry. Coordinates to 8 decimals. Refuse more than 20,000 vertices with a message. Do not run
  the polygon union at all on input already past that limit (it can freeze the UI before the size
  check); leave it as a multi-polygon and let the size check refuse it.
- A file with several layers is merged layer by layer, and each layer's "left out" count is kept and
  added to the final merge's, so the user is told about every dropped point or line.
- Static catalogs (no search endpoint): filter by the shape against each item's footprint (a quick
  extent test first, then an exact intersection; a footprint with a hole does not match a point in
  the hole). A record that has only an extent (a collection's assets) is tested with each of its
  declared boxes as a polygon against the shape itself, not against the shape's bounding box. An
  item box that crosses the antimeridian (west greater than east) is split in two before any
  overlap test.
- The shape and a typed bbox are two ways to say where to search, and the one touched last wins:
  typing a bbox or turning on "limit to the map extent" drops the shape, and choosing a shape
  clears the bbox. If extra search parameters typed by the user also name `bbox` or
  `intersects`, the one the user chose in the panel replaces it, since a server rejects both.
- A drawing that is open must end when an area is chosen another way (selection, file), a file
  still being read when another area is chosen is dropped, cancelling a drawing puts the previous
  area back on the map, and Enter or Escape typed in a text field is not the drawing's.
- Buffer: an optional distance in **meters** that widens a point or line into a polygon before
  searching; polygons are not changed. Do it properly (geodesic or in a suitable projection), not by
  degrees. (The ArcGIS Pro `kylidar-addin` buffers in feet; the user chose meters.)

### Sort and cloud cover
- Sort by date: server default, newest first, oldest first. Send as
  `sortby: [{field: "properties.datetime", direction}]`. Always also sort the loaded results
  locally, items with no date last, across pages, and re-sort them when the choice changes (keep the
  results in the server's order so "server default" can bring that order back).
- Cloud cover limit (off by default, value 10%): CQL2 filter
  `filter-lang: cql2-json`, `{op:"or", args:[{op:"<=", args:[{property:"eo:cloud_cover"}, N]},
  {op:"isNull", args:[{property:"eo:cloud_cover"}]}]}`. Also filter the results locally: drop an
  item only if it reports a number above N (items with no `eo:cloud_cover` stay: radar, elevation,
  aerial imagery). Combine with a user-typed JSON filter using `and`. A typed CQL2 **text**
  filter cannot be combined with it, so then send no cloud filter and rely on the local one.
- **Retry order** for a server that refuses a parameter: the request as asked, then without the
  sort, then without the cloud filter, then plain (the order is: each cloud form with the sort, then
  without it). Do **not** use the older STAC `query` extension as a cloud fallback: its `lte`
  drops scenes that report no cloud cover, which the limit promises to keep, and it cannot be
  recovered locally.
- Retry **only when the server refused the request** (HTTP 400, 422 or 501). A 401, 403, 429 or 5xx
  is not a refused parameter; retrying would send up to four requests at a rate-limited catalog.
- A POST "next" link usually carries only a token. Ask later pages with the body that finally got
  the first page (after any fallback), so a parameter the server refused is not sent again.
- GET fallback (a server that only does GET): `sortby=-properties.datetime` (or `+` for
  oldest), and ask again without it if refused. The user's choice in the panel wins over a
  `sortby` typed into the extra parameters.
- Keep the cloud limit and sort the same across "Load more": they belong to the search, not to the
  controls as they stand when the next page is asked for.
- Say plainly in the UI that sorting orders the results loaded so far; later pages can bring scenes
  that belong earlier or later.

### Bands, combinations and indices
- A band is one raster band of one asset, found by `common_name` in `eo:bands` / `bands` (STAC 1.1),
  else by the asset key. `raster:bands` gives `scale` and `offset` (value = DN * scale + offset);
  a STAC 1.1 unified `bands` entry names them `raster:scale` and `raster:offset`, so read both
  spellings. A band's declared `nodata` is kept when it is a number ("nan" is not one). Only assets
  that are Cloud-Optimized GeoTIFFs count.
- Name aliases (a wanted name, then stand-ins, best first): nir = nir, nir08, nir09; swir16 =
  swir16, swir1; swir22 = swir22, swir2; rededge = rededge, rededge1-3; coastal = coastal, aerosol;
  lwir = lwir, lwir11, lwir12.
- Band combinations (red, green, blue channels): True color (red, green, blue); Color infrared
  (nir, red, green); Shortwave infrared (swir22, nir, red); False color urban (swir22, swir16, red);
  Agriculture (swir16, nir, blue); Geology (swir22, swir16, blue); Bathymetric (red, green, coastal).
- Indices (formula): NDVI `(nir-red)/(nir+red)`; NDWI `(green-nir)/(green+nir)`; MNDWI
  `(green-swir16)/(green+swir16)`; NDMI `(nir-swir16)/(nir+swir16)`; NBR
  `(nir-swir22)/(nir+swir22)`; NDBI `(swir16-nir)/(swir16+nir)`; SAVI
  `1.5*(nir-red)/(nir+red+0.5)`; EVI `2.5*(nir-red)/(nir+6*red-7.5*blue+1)`.
- Offer only the combinations and indices whose bands the item has. A formula evaluator must be a
  safe parser (numbers, band names, `+ - * / ^`, brackets, and sqrt, abs, ln, log10, sin, cos, tan,
  asin, acos, atan, min, max), never eval. A "custom formula" entry is a natural extension.
  Take care when listing the names a formula uses: a number with an exponent (`1e-4 * nir`) is a
  number, not a band called `e`; and only the formula's own functions are functions, so a name
  such as `__proto__(nir)` or `toString(nir)` is an invalid formula, not a call.
- Rendering: read each band's COG window for a map tile in **the file's own CRS** (UTM, state
  plane, lon/lat; bands of one item may have different pixel sizes **and a pixel's height may differ
  from its width**), choosing the overview that matches the tile's ground resolution on both axes,
  nearest-neighbour sampling, apply scale and offset.
  - Overviews: a COG can hold **mask pages** (NewSubfileType bit 4: a transparency mask, or an
    overview of one) of the same size as the image they mask. Leave them out when choosing an
    overview, or the mask's 0/255 bytes get painted as data.
  - Rows are found with the vertical pixel size, columns with the horizontal one.
  - A file with no overview coarse enough would need an enormous window for a zoomed-out tile; read
    it thinned to a bounded size instead of leaving the tile blank.
  - **No data:** use the value the catalog declares for the band, else the file's own no-data tag,
    and assume nothing otherwise. (An earlier version treated 0 as missing for every 16-bit file;
    that hides valid zeros.) A pixel where any band is missing is transparent, **checked before the
    formula runs**, since `a ^ 0` turns a missing value into 1.
  - Colour channels stretch linearly: scaled bands 0 to 0.3 reflectance, 8-bit 0 to 255, unscaled
    16-bit 0 to 3000. An index is drawn on a 3-stop ramp over its value window (-1..1 by default;
    BrBG default: `#8c510a`, `#f5f5f5`, `#01665e`); a result that is not finite is transparent.
    Validate the window as it will be drawn, with the default end standing in for one not given:
    a minimum of 2 with the default maximum of 1 is no window at all.
- Style for a rendering: composites get brightness (gain) and gamma; indices get a ramp (BrBG,
  RdYlGn, RdBu, Spectral, Greys) and a low/high value window.
- Planetary Computer assets need a signed link (SAS token), valid for about an hour. **Do not put
  a signed link in a rendering's saved recipe**: it is a credential that would travel with the
  project, and it expires. Save the band's own address plus how to sign it (catalog URL,
  collection id, the asset address) and ask for a fresh signature each time the file is read.
  Check that the saved "how to sign" names the same file the band reads.

### Result card actions (GeoLibre's UI; adapt to egui)
- An Assets menu: add the chosen asset at once (one entry per Zarr variable). A Bands menu with the
  combinations and indices above. Download as a menu. The author asked for these instead of a
  right-click menu.

## Where to read the GeoLibre implementation

Path: `D:\Data\GeoLibre`, branch `stac/expand-capabilities` (last commit `c2bb6cef`; it is
GeoLibre PR #3381, a draft under review, so the code can still change). Start Claude
with that folder available, for example `claude --add-dir D:\Data\GeoLibre`, or `/add-dir` inside the
session. Read these files (they are the readable spec; do not translate them blindly, the rules above
apply, and GeoLibre is MIT):

- `packages/plugins/src/plugins/stac-aoi.ts` - shape merging, buffering, file reading.
- `packages/plugins/src/plugins/stac-api.ts` - `intersects`, sort, cloud filter and retry order,
  static-catalog footprint filtering, `sortStacItems`, `withinCloudCover`.
- `packages/plugins/src/plugins/stac-bands.ts` - band discovery, aliases, combinations, indices, the
  formula evaluator.
- `packages/plugins/src/plugins/stac-render.ts` - the per-band COG tile renderer and its style.
- Tests as specification: `tests/stac-aoi.test.ts`, `tests/stac-api.test.ts`,
  `tests/stac-bands.test.ts`, `tests/stac-render.test.ts`, `tests/stac-render-tiles.test.ts`
  (serves a small tiled GeoTIFF in lon/lat and UTM and renders it).
- UI: `packages/plugins/src/plugins/maplibre-stac.ts` (the panel) and
  `apps/geolibre-desktop/src/components/panels/style-panel/StacRenderingSection.tsx` (style editor).

The author's earlier tools, for behaviour only (the QGIS plugin is GPL, see the rules above):
`D:\Data\qgis\kentucky-stac`, `D:\Data\arcgispro\kyfromabove-ext`, `D:\Data\arcgispro\kylidar-addin`.

## What review found (so you do not repeat it)

GeoLibre's PR got about 46 review comments from bots. Almost all were real bugs; the ones that
matter for a second implementation are folded into the specification above. The themes:
- A fallback chain that retries on any error multiplies load on a rate-limited server; retry only
  on a refusal, and remember which form finally worked for the next page.
- A fallback that is "simpler" can silently change the meaning of a filter (the `query` extension
  dropping scenes with no cloud cover). Prefer falling back to *no* server filter and filtering
  locally.
- Anything saved with a project (a recipe, a URL) must not hold a credential, and must not expire.
- Raster details real files exercise and toy files do not: mask pages, non-square pixels, no-data
  conventions, files with no overviews, projected CRSs.
- UI state that two controls can both set (a shape and a bbox, two drawing tools, a late file read)
  needs one owner and a rule for who wins.
- Library APIs bite: geotiff.js hands out tags through `getValue`, not as properties. A test on a
  real (hand-built) file caught a fix that looked right and did nothing.

## Lessons from the GeoLibre build

- Check against real data early. Unit tests with fake data hid a UI layout bug and a CRS risk; the
  renderer was only trusted after tests that serve real tiled GeoTIFFs in two projections, with
  masks, non-square pixels and an oversized window.
- Prove a test can fail: for each fix, break the fix and watch its test fail. Two of GeoLibre's
  first tests passed against the bug they were written for.
- A viewer-side rendering of bands is a different thing from the raster layer the app already has:
  the existing raster path only combines bands inside one file, but most catalogs (Sentinel-2,
  Landsat) publish one file per band. Plan for per-band reading.
- Respect the app's own "zoom to new layers" setting when adding layers; do not zoom unconditionally.
- Keep behaviour changes small and reviewable. GeoLibre's PR was large; RustGIS asks for
  proposals first and prefers focused PRs.

## Suggested order (small PRs, each with tests, notices and screenshots)

1. Shape search (`intersects`) and static-catalog footprint filtering, with merge rules and notices;
   then buffer.
2. Date-sort choice and the cloud limit in the UI (the command parameters may already exist).
3. Item inspector (JSON/HTML), multi-asset add.
4. Band combinations and indices rendering, then their style editor.
5. Multi-item selection and mosaic; lidar filters and VPC; script export (each is its own proposal).

## What to do first

1. Read `AGENTS.md`, `CONTRIBUTING.md`, `ROADMAP.md` (Underway), `GEOLIBRE_PARITY.md`, and
   `crates/engine/src/cmd/stac.rs`.
2. Ask the author the open questions below.
3. Write a short proposal (issue or draft PR) per `CONTRIBUTING.md` before writing much code.

Open questions for the author:
- How should the PR credit the Kentucky STAC QGIS plugin (GPL-2.0) given RustGIS is Apache-2.0?
- Which of the six wish-list items should come first for RustGIS?
- Should shape search be a new set of `stac.search` parameters, or a separate command?
