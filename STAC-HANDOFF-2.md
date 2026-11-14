# STAC handoff 2: band combinations (branch `stac-band-combinations`)

Second note, after `STAC-HANDOFF.md`. Read that one first for the repo rules, the GPL decision and
the unbuilt scope. This one covers the band-combination slice and moving the work to WSL.

## Where things stand (2026-11-14)

- Merged upstream: #1872 (STAC item ordering) and #1963 (diskwatch flaky test).
- Branch `stac-band-combinations` is cut from `main` (`2aa4b22d`). **Every change on it is
  uncommitted and unpushed.** A fresh clone in WSL will not have it. Either work from
  `/mnt/d/Data/RustGIS` (slow builds on the Windows drive) or commit a WIP on the branch and push it
  first. Ask the user before pushing.
- GPL: the Kentucky STAC QGIS plugin is a behaviour reference only. No code copied. GeoLibre's
  `stac-bands.ts` says it was ported from the plugin's `bands.py`, so do not read or port it either.
  Whether the PR credits the plugin is the user's call.

## What the slice does

A window stack: read each band's window from its asset and stack them into one multi-band raster
layer, drawn as RGB. It is not a live tile composite. Band indices stay out; `raster.index` already
works on the stacked layer.

Files (all new or modified, uncommitted):

- `crates/tiles/src/stac.rs`: `Asset.bands: Vec<BandInfo>` (`common`, `scale`, `offset`, `nodata`),
  `asset_bands()` reading `eo:bands`, STAC 1.1 `bands` and `raster:bands`; `asset_json` writes bands
  back so an item passed to a command keeps its band info.
- `crates/tiles/src/stac/bands.rs` (new): `Band`, `ALIASES` (nir/nir08/nir09, swir16/swir1,
  swir22/swir2, rededge, coastal/aerosol, lwir), `COMBINATIONS` (trueColor, colorInfrared,
  shortwaveInfrared, falseColorUrban, agriculture, geology, bathymetric), `bands()`, `find()`,
  `available()`. 7 tests.
- `crates/engine/src/cmd/stac_bands.rs` (new): command `stac.addBands`, background, Network effect.
  Params: `item` (STAC item JSON, required), `combination` (default trueColor) or `red/green/blue`,
  `bbox` (default "view"), `cellSize` (about 2048 cells across), `name`, `ran`. Stacks on the finest
  grid by nearest cell, applies scale/offset, a cell with no value in any band is no data in all
  three. Notices: coarser-grid note, and "N cells (p%) have no value in at least one band". Refuses
  different CRS. Planetary Computer links are signed fresh at read time, never saved. Local-network
  hrefs are refused. Wasm returns "made on the desktop and the command line for now". 5 tests.
- `crates/engine/src/cmd/layer.rs`: `window_box_of` wrapper. `cmd/mod.rs`: registered.
  `cmd/workflows.rs`: `IN_BACKGROUND` entry. `crates/engine/src/tests.rs`: cancel-test entry for
  `stac.addBands`.
- `crates/ui-egui/src/stacui.rs`: **Bands** menu on each result card (desktop only), lists only the
  combinations the item can make, calls `run_elsewhere(app, BANDS_SECTION, "stac.addBands", ...)`.
  `BANDS_SECTION = "rasterFunctions"`. It was "indices", which belongs to the optional Imagery
  extension and showed "Indices is part of the Imagery extension, which is off". Fixed; a test turns
  the extension off and checks it.

## Checks

- tiles `stac` 48 pass; engine `stac` 35 pass; ui-egui `stacui` 35 pass (after the section fix).
- Full engine suite 1412 passed before the section fix. Clippy `-D warnings` and fmt were clean
  before the section fix. **Not re-run since.**
- Last full `cargo xtask ci` (`ci45`, before the section fix): only failure was the known
  Windows-only `mapview::tests::explore_click_asking_only_wms_clears_without_a_notice`. Cargo stopped
  there, so later targets and `cargo xtask layers` did not run for this branch.
- No wasm32 build has been done (no clang on the Windows machine). WSL could fix that:
  `apt install clang`, `rustup target add wasm32-unknown-unknown`, then
  `cargo build --target wasm32-unknown-unknown -p rustgis-web`.

## Open items

1. **Freeze.** The user reported "it froze here for a second trying to figure out how to draw" after
   adding bands. Cause unknown. Suspect: first draw of a large (about 2048 by 2048) 3-band Float32
   layer on the UI thread. Ask when it froze (click, read, or landing). If it is the draw, measure
   in the headless renderer (`ui_shot` example) with a synthetic raster, and consider a smaller
   default `cellSize` or building a downsampled copy.
2. Re-run CI and `cargo xtask layers` separately on this branch.
3. Commit and push the branch, then open a PR (each needs the user's go-ahead). Screenshots go in the
   PR, not the repo. Update `GEOLIBRE_PARITY.md` (about line 57) and the `ROADMAP.md` Underway check.
   Add docs for `stac.addBands`.
4. After the PR merges: sync `main`, merge `main` into `handoff` (merge, no rebase, no force-push),
   push (ask first), return to `main`.
5. Unbuilt STAC scope: band indices beyond named combinations, item inspector, mosaic, lidar/VPC,
   script export.
6. The user pasted a GeoJSON of one COPC point (`N093E313_LAS_Phase3.copc pointcloud (3)`, at
   -84.289, 38.017, classification 2, intensity 54960, z 1009.96) with no question. It may relate to
   the lidar/VPC slice. Not acted on.

## Why WSL

The user wants to work the repo in WSL because WMS and the webview do not work in testing on
Windows, and the known Windows-only failure above is a WMS test. Things to expect:

- Use a clone on the WSL filesystem (`~/RustGIS`), not `/mnt/d`, for build speed and file watching.
- Working copy on Windows is CRLF (`core.autocrlf=true`); the repo is LF. A WSL clone is LF already.
  Do not copy the Windows tree over it.
- Needs `build-essential`, `pkg-config`, and the GUI libraries for the desktop app (X11/Wayland, GTK
  or the webview's WebKitGTK, depending on the feature). Headless tests and `ui_shot` do not need a
  display. WSLg shows the app window on Windows 11.
- Cargo jobs: keep `CARGO_BUILD_JOBS=4` (memory; never 8, fall back to 2 on memory errors). WSL2
  caps memory at half the host by default; set `.wslconfig` if builds get killed.
- Remotes: `origin` = `https://github.com/ianhorn/RustGIS.git` (the user's fork), `upstream` =
  `https://github.com/opengeos/RustGIS.git`. The `gh` CLI needs its own login inside WSL.
- `STAC-HANDOFF.md` and this file live only on branch `handoff`, not on `main`. Never add them to a PR
  branch or `.gitignore`.

## Lessons carried over

- Never trust a background "exit code 0" notice; grep the log for `exit=`, `layers: ok`, `FAILED`,
  `^error`.
- Run builds as tracked background tasks. Do not start cargo while the user is building the app.
- `git fetch` and check `git rev-list --left-right --count` before pushing; merges or `--ff-only`,
  never force-push.
- Outward-facing actions (push, PR, comments, resolving threads) need explicit approval each time.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and the session URL
  line; PR bodies end with the Claude Code line and the session URL.
