//! `stac.addBands`: a band combination of a STAC item as one multi-band raster layer.
//!
//! Sentinel-2 and Landsat publish one file per band, so a colour image is made from three files.
//! This reads the window of each (the view or a box, in place by range, as `layer.addUrl` with a
//! `bbox` does), puts them on one grid (the finest of the three, by nearest cell) and keeps them as
//! one raster of three bands drawn as red, green and blue. Scale and offset from the catalog are
//! applied, so the values are what the bands measure (reflectance), and a cell with no value in
//! any of the three is no data in all of them. The band names and which combinations an item
//! offers are [`bands`](crate::render::rustgis_tiles::stac::bands)'s.
//!
//! It makes its raster on another thread from the app, with Cancel ([`super::workflows`]).

use serde_json::Value;

use super::workflows::Work;
use crate::{CmdResult, CommandSpec as C, Session};

pub fn commands() -> Vec<C> {
    vec![C::new("stac.addBands", "Add STAC Bands", "Home › Add › Web Data › STAC Search", add_bands).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Network)).view().params(
        r#"{"item": a STAC item (from stac.search), "combination"?: "trueColor"|"colorInfrared"|"shortwaveInfrared"|"falseColorUrban"|"agriculture"|"geology"|"bathymetric" (default trueColor), "red"?, "green"?, "blue"?: band names (red, nir, swir16… or an asset's key) used instead of a combination, "bbox"?: [west, south, east, north] in degrees | "view" (default), "cellSize"?: the coarsest overview whose cells are at most this size is read (default: about 2,048 cells across the box), "name"?: text, "ran"?: (internal: a raster the app has worked out)}"#,
    )]
}

fn add_bands(s: &mut Session, p: &Value) -> CmdResult {
    super::workflows::made_or_ran(s, p, "stac.addBands")
}

/// The browser has no thread to wait for several files on.
#[cfg(target_arch = "wasm32")]
const IN_THE_BROWSER: &str = "band combinations are made on the desktop and the command line for now";

/// What `stac.addBands` reads of the session (the item given, the view), and the work that reads
/// the three files and makes the raster.
pub(crate) fn bands_work(s: &Session, p: &Value) -> Result<Work, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err(IN_THE_BROWSER.into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    desktop::work(s, p)
}

#[cfg(not(target_arch = "wasm32"))]
mod desktop {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, Ordering};

    use rustgis_data::raster::Raster;
    use rustgis_map::{RasterLayer, RasterRenderer};
    use serde_json::{Value, json};

    use super::super::layer::{WindowAsked, WindowRaster};
    use super::super::ps;
    use super::super::workflows::{Made, Then, Work};
    use crate::Session;
    use crate::render::rustgis_tiles::stac::{self, bands as sb};

    /// A window's cells across, when no cell size is given: its overview is the one nearest this.
    const ACROSS: f64 = 2048.0;

    /// A kilometre of a degree of latitude, in metres.
    const DEGREE_M: f64 = 111_320.0;

    pub(super) fn work(s: &Session, p: &Value) -> Result<Work, String> {
        let item = match p.get("item").filter(|v| v.is_object()) {
            Some(v) => stac::item_from_json(v, None).ok_or("item isn't a STAC item")?,
            None => return Err("give item: a STAC item, as stac.search answers".into()),
        };
        let all = sb::bands(&item);
        if all.is_empty() {
            return Err(format!("{}: it has no GeoTIFF asset to make a band combination from", item.id));
        }
        // The three bands: a combination, or the three named.
        let named = ["red", "green", "blue"].map(|k| ps(p, k).map(str::trim).filter(|v| !v.is_empty()));
        let (label, wanted): (String, [String; 3]) = if named.iter().any(Option::is_some) {
            let [Some(r), Some(g), Some(b)] = named else { return Err("give red, green and blue together, or a combination".into()) };
            (format!("{r}-{g}-{b}"), [r.to_string(), g.to_string(), b.to_string()])
        } else {
            let id = ps(p, "combination").map(str::trim).filter(|v| !v.is_empty()).unwrap_or("trueColor");
            let c = sb::COMBINATIONS.iter().find(|c| c.id.eq_ignore_ascii_case(id)).ok_or_else(|| {
                let ids: Vec<&str> = sb::COMBINATIONS.iter().map(|c| c.id).collect();
                format!("unknown combination '{id}' (the combinations are {})", ids.join(", "))
            })?;
            (c.name.to_string(), c.channels.map(str::to_string))
        };
        let mut picked = Vec::with_capacity(3);
        for name in &wanted {
            let band = sb::find(&all, name).ok_or_else(|| format!("{}: no band '{name}' (it has {})", item.id, names_of(&all)))?;
            picked.push(band.clone());
        }
        // Each asset's address, checked as `stac.addAsset` checks it.
        let mut hrefs = Vec::with_capacity(3);
        for band in &picked {
            hrefs.push(item.asset_href(Some(&band.asset))?.1);
        }
        let mut asked = p.clone();
        if asked.get("bbox").is_none_or(Value::is_null) {
            asked["bbox"] = json!("view");
        }
        let bbox = super::super::layer::window_box_of(s, &asked)?;
        let cell = match p.get("cellSize").filter(|v| !v.is_null()) {
            None => auto_cell(bbox),
            Some(v) => v.as_f64().filter(|c| c.is_finite() && *c > 0.0).ok_or("cellSize is a size above 0")?,
        };
        let name = ps(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| format!("{} {label}", item.id), str::to_string);
        let channels = [wanted[0].clone(), wanted[1].clone(), wanted[2].clone()];
        Ok(Box::new(move || {
            rustgis_data::progress::check().map_err(|e| e.to_string())?;
            let (made, notes) = read_and_stack(&name, &channels, &picked, &hrefs, bbox, cell)?;
            made_saying(made, notes, json!({"bands": channels, "bbox": bbox}))
        }))
    }

    /// The names a bands list offers, for an error: common names, else keys, each once.
    fn names_of(all: &[sb::Band]) -> String {
        let mut seen = BTreeSet::new();
        let names: Vec<&str> = all
            .iter()
            .map(|b| if b.common.is_empty() { b.asset.as_str() } else { b.common.as_str() })
            .filter(|n| seen.insert(*n))
            .take(24)
            .collect();
        names.join(", ")
    }

    /// The cell size that reads a box about [`ACROSS`] cells across: the overview at least that
    /// coarse, or the full image of a window small enough for it.
    pub(super) fn auto_cell(bbox: [f64; 4]) -> f64 {
        let [west, south, east, north] = bbox;
        let mid = ((south + north) / 2.0).to_radians().cos().abs().max(0.01);
        let (w, h) = ((east - west).abs() * DEGREE_M * mid, (north - south).abs() * DEGREE_M);
        (w.max(h) / ACROSS).max(0.001)
    }

    /// A window of the file at `url`, every band, read on a thread of its own while this one looks
    /// at the run's Cancel and stops the read once it is set. A thread the system refuses costs
    /// only that: the read runs here, Cancel looked at once it is in.
    fn read_window(url: &str, bbox: [f64; 4], cell: f64, bands: usize) -> Result<WindowRaster, String> {
        let asked = WindowAsked::of(url, bbox, Some(cell), bands);
        let stop = AtomicBool::new(false);
        let w = std::thread::scope(|scope| {
            let reader = std::thread::Builder::new()
                .name("STAC band window".into())
                .spawn_scoped(scope, || super::super::layer::read_window_raster(&asked, &stop));
            let Ok(reader) = reader else { return super::super::layer::read_window_raster(&asked, &stop) };
            while !reader.is_finished() {
                if rustgis_data::progress::check().is_err() {
                    stop.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            reader.join().unwrap_or_else(|_| Err(format!("{url}: the read failed unexpectedly")))
        })?;
        rustgis_data::progress::check().map_err(|e| e.to_string())?;
        Ok(w)
    }

    /// The three windows, read, and put on one grid: the raster of three bands, and what to say.
    fn read_and_stack(
        name: &str,
        channels: &[String; 3],
        picked: &[sb::Band],
        hrefs: &[String],
        bbox: [f64; 4],
        cell: f64,
    ) -> Result<(Raster, Vec<String>), String> {
        // One read for a file, whichever of its bands are wanted.
        let mut windows: Vec<(String, WindowRaster)> = Vec::new();
        let mut notes = Vec::new();
        for (i, band) in picked.iter().enumerate() {
            let href = hrefs.get(i).ok_or("a band has no address")?;
            if windows.iter().any(|(h, _)| h == href) {
                continue;
            }
            // A Planetary Computer file is signed as it is read, not when the item was found.
            let signed = stac::sign_blocking(href)?;
            let w = read_window(&signed, bbox, cell, band.index as usize)?;
            let who = channels.get(i).map_or("band", String::as_str);
            notes.extend(w.notes().into_iter().map(|n| format!("{who}: {n}")));
            windows.push((href.clone(), w));
        }
        let sources: Vec<(&Raster, &sb::Band)> = picked
            .iter()
            .enumerate()
            .map(|(i, band)| {
                let href = hrefs.get(i).ok_or("a band has no address")?;
                let w = windows.iter().find(|(h, _)| h == href).map(|(_, w)| &w.raster).ok_or("a band was not read")?;
                Ok((w, band))
            })
            .collect::<Result<_, String>>()?;
        let (raster, said) = stack(name, channels, &sources)?;
        notes.extend(said);
        Ok((raster, notes))
    }

    /// The bands as one raster of three bands on the finest grid among them, by nearest cell, with
    /// the catalog's scale and offset applied. A cell with no value in any is no data in all.
    pub(super) fn stack(name: &str, channels: &[String; 3], sources: &[(&Raster, &sb::Band)]) -> Result<(Raster, Vec<String>), String> {
        let [(a, _), (b, _), (c, _)] = sources else { return Err("a combination has three bands".into()) };
        if a.crs != b.crs || a.crs != c.crs {
            return Err("the bands are in different coordinate systems: they can't be put on one grid".into());
        }
        let finest = sources
            .iter()
            .map(|(r, _)| *r)
            .min_by(|x, y| x.transform.pixel_width.abs().total_cmp(&y.transform.pixel_width.abs()))
            .ok_or("a combination has three bands")?;
        let (w, h) = (finest.width as usize, finest.height as usize);
        let mut bands: Vec<Vec<f32>> = Vec::with_capacity(3);
        for (raster, band) in sources {
            let mut out = rustgis_analysis::rasterops::new_band(w, h, rustgis_analysis::rasterops::NODATA)?;
            let data = &raster.bands.get(band.index.saturating_sub(1) as usize).ok_or("a band is not in its file")?.data;
            let catalog_nodata = band.nodata.map(|v| v as f32);
            for row in 0..h {
                if row % 256 == 0 {
                    rustgis_data::progress::check().map_err(|e| e.to_string())?;
                }
                for col in 0..w {
                    let world = finest.transform.to_world(col as f64 + 0.5, row as f64 + 0.5);
                    let (sc, sr) = raster.transform.to_pixel(world);
                    let (sc, sr) = (sc.floor(), sr.floor());
                    if sc < 0.0 || sr < 0.0 || sc >= f64::from(raster.width) || sr >= f64::from(raster.height) {
                        continue;
                    }
                    let Some(&v) = data.get(sr as usize * raster.width as usize + sc as usize) else { continue };
                    if raster.is_valid(v) && catalog_nodata != Some(v) {
                        let value = (f64::from(v) * band.scale + band.offset) as f32;
                        if value.is_finite()
                            && let Some(o) = out.get_mut(row * w + col)
                        {
                            *o = value;
                        }
                    }
                }
            }
            bands.push(out);
        }
        // A cell with no value in one band is no data in all: a colour from two bands is no colour.
        let none = rustgis_analysis::rasterops::NODATA;
        let mut left = 0usize;
        for i in 0..w * h {
            if bands.iter().any(|b| b.get(i).is_none_or(|v| *v == none)) {
                left += 1;
                bands.iter_mut().for_each(|b| {
                    if let Some(v) = b.get_mut(i) {
                        *v = none;
                    }
                });
            }
        }
        let mut notes = Vec::new();
        let size = finest.transform.pixel_width.abs();
        let resampled: Vec<&str> = sources
            .iter()
            .zip(channels)
            .filter(|((r, _), _)| (r.transform.pixel_width.abs() - size).abs() > size * 0.01)
            .map(|(_, c)| c.as_str())
            .collect();
        if !resampled.is_empty() {
            notes.push(format!(
                "{} read on a coarser grid than the finest of the three, and put on it by nearest cell",
                resampled.join(", ")
            ));
        }
        if left > 0 {
            let share = left as f64 * 100.0 / (w * h).max(1) as f64;
            notes.push(format!("{left} cells ({share:.1}%) have no value in at least one band (no data, or outside a band's window) and are no data in all three"));
        }
        let mut raster = rustgis_analysis::rasterops::derive(finest, name, bands)?;
        for (b, c) in raster.bands.iter_mut().zip(channels) {
            b.name = c.clone();
        }
        Ok((raster, notes))
    }

    /// The made raster, kept as a GeoTIFF and drawn as red, green and blue, with what to say of it
    /// once it is added (by its name on the map) and what the command answers besides.
    fn made_saying(raster: Raster, notes: Vec<String>, extra: Value) -> Result<Made, String> {
        let bytes = rustgis_data::raster::geotiff::write_all(&raster).map_err(|e| e.to_string())?;
        let name = raster.name.clone();
        let mut layer = RasterLayer::from_bytes(raster, format!("{name}.tif"), bytes);
        layer.style.renderer = RasterRenderer::Rgb { red: 0, green: 1, blue: 2 };
        let said = name.clone();
        let then: Then = Box::new(move |s: &mut Session, v: &mut Value| {
            let id = v.get("layer").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok());
            let added = id.and_then(|id| s.map().layer(id)).map_or(said, |l| l.name.clone());
            for note in notes {
                s.notify(format!("{added}: {note}"), false);
            }
        });
        Ok(super::super::workflows::made_layer(name, layer, Some(extra), Some(then)))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::desktop::{auto_cell, stack};
    use crate::render::rustgis_tiles::stac::bands::Band;
    use rustgis_data::raster::{ColorHint, GeoTransform, Raster, crs::Crs};

    /// A raster of one band on a grid with these cells (`size` degrees), `w × h` from the corner.
    fn grid(w: u32, h: u32, size: f64, values: Vec<f32>) -> Raster {
        let t = GeoTransform { origin_x: 10.0, origin_y: 45.0, pixel_width: size, pixel_height: -size };
        Raster::new("b", w, h, vec![values], t, Crs::from_epsg(4326).unwrap(), Some(0.0), ColorHint::Gray, "UInt16").unwrap()
    }

    fn band(scale: f64, offset: f64) -> Band {
        Band { asset: "x".into(), href: "https://x.org/x.tif".into(), index: 1, common: String::new(), scale, offset, nodata: None }
    }

    fn names() -> [String; 3] {
        ["red".into(), "green".into(), "blue".into()]
    }

    #[test]
    fn three_bands_become_one_raster_on_the_finest_grid_with_their_scale() {
        // Red and green at 10 m cells (4 × 4); blue at twice the size (2 × 2).
        let red = grid(4, 4, 0.0001, vec![1000.0; 16]);
        let green = grid(4, 4, 0.0001, vec![2000.0; 16]);
        let blue = grid(2, 2, 0.0002, vec![3000.0; 4]);
        let (r, notes) =
            stack("s", &names(), &[(&red, &band(0.0001, 0.0)), (&green, &band(0.0001, 0.0)), (&blue, &band(0.0001, -0.1))]).unwrap();
        assert_eq!((r.width, r.height, r.bands.len()), (4, 4, 3));
        let values: Vec<f32> = r.bands.iter().map(|b| b.data[0]).collect();
        assert_eq!(values, [0.1, 0.2, 0.2], "reflectance: v · scale + offset");
        assert_eq!(r.bands.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["red", "green", "blue"]);
        assert!(notes.iter().any(|n| n.contains("blue") && n.contains("coarser grid")), "{notes:?}");
        assert!(!notes.iter().any(|n| n.contains("no data")), "every cell has all three: {notes:?}");
    }

    #[test]
    fn a_cell_missing_from_one_band_is_no_data_in_all_and_said() {
        let mut gone = vec![1000.0; 16];
        gone[5] = 0.0; // the file's no data
        let ok = grid(4, 4, 0.0001, vec![1000.0; 16]);
        let holed = grid(4, 4, 0.0001, gone);
        let (r, notes) = stack("s", &names(), &[(&ok, &band(1.0, 0.0)), (&holed, &band(1.0, 0.0)), (&ok, &band(1.0, 0.0))]).unwrap();
        let none = rustgis_analysis::rasterops::NODATA;
        assert!(r.bands.iter().all(|b| b.data[5] == none), "no data in all three");
        assert!(r.bands.iter().all(|b| b.data[4] == 1000.0), "the cells beside it are kept");
        assert!(notes.iter().any(|n| n.contains("1 cells") && n.contains("no value in at least one band")), "{notes:?}");
    }

    #[test]
    fn a_window_that_does_not_reach_a_cell_leaves_it_no_data() {
        let wide = grid(4, 4, 0.0001, vec![1000.0; 16]);
        // Covers only the top-left 2 × 2 of the finest grid.
        let small = grid(2, 2, 0.0001, vec![1000.0; 4]);
        let (r, notes) = stack("s", &names(), &[(&wide, &band(1.0, 0.0)), (&wide, &band(1.0, 0.0)), (&small, &band(1.0, 0.0))]).unwrap();
        let none = rustgis_analysis::rasterops::NODATA;
        assert_eq!(r.bands[0].data.iter().filter(|v| **v == none).count(), 12);
        assert!(notes.iter().any(|n| n.contains("12 cells")), "{notes:?}");
    }

    #[test]
    fn bands_in_different_coordinate_systems_are_refused() {
        let a = grid(2, 2, 0.0001, vec![1.0; 4]);
        let mut b = grid(2, 2, 0.0001, vec![1.0; 4]);
        b.crs = Crs::from_epsg(3857).unwrap();
        let err = stack("s", &names(), &[(&a, &band(1.0, 0.0)), (&b, &band(1.0, 0.0)), (&a, &band(1.0, 0.0))]).unwrap_err();
        assert!(err.contains("different coordinate systems"), "{err}");
    }

    #[test]
    fn the_cell_size_read_follows_the_box() {
        // A 10 km box is read at about 5 m cells (the full image of a 10 m scene); a 2° box coarser.
        let small = auto_cell([10.0, 45.0, 10.09, 45.09]);
        let large = auto_cell([10.0, 45.0, 12.0, 47.0]);
        assert!((4.0..6.0).contains(&small), "{small}");
        assert!(large > 100.0, "{large}");
        // A degenerate box doesn't give a zero cell.
        assert!(auto_cell([10.0, 45.0, 10.0, 45.0]) > 0.0);
    }
}
