//! Which bands a STAC item offers, and which band combinations can be made from them.
//!
//! A *band* is one raster band of one GeoTIFF asset, found by its common name (`red`, `green`,
//! `nir`, `swir16`…) that catalogs give in `eo:bands` or STAC 1.1's `bands`, or, for an asset of
//! one band the catalog names none for, by the asset's key. Sentinel-2 and Landsat publish one
//! file per band, so a colour image is made from three files. Pure: no network, no files; the
//! item is hostile input (counts are bounded and nothing is indexed past its end).

use super::Item;

/// The most bands an item is taken to offer (its assets are already capped).
const MAX_BANDS: usize = 256;

/// One band of one asset's file.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    /// The asset's key.
    pub asset: String,
    /// The asset's address.
    pub href: String,
    /// Its place in the file, from 1.
    pub index: u32,
    /// Its common name, lower case; empty when the catalog names none.
    pub common: String,
    /// A stored value `v` means `v · scale + offset` (1 and 0 when the catalog gives none).
    pub scale: f64,
    pub offset: f64,
    /// The stored value the catalog says is no data, when it names one.
    pub nodata: Option<f64>,
}

/// A wanted name and the names that can stand in for it, best first.
const ALIASES: &[(&str, &[&str])] = &[
    ("nir", &["nir", "nir08", "nir09"]),
    ("swir16", &["swir16", "swir1"]),
    ("swir22", &["swir22", "swir2"]),
    ("rededge", &["rededge", "rededge1", "rededge2", "rededge3"]),
    ("coastal", &["coastal", "aerosol"]),
    ("lwir", &["lwir", "lwir11", "lwir12"]),
];

/// A band combination: three bands drawn as red, green and blue.
#[derive(Debug, PartialEq)]
pub struct Combination {
    /// A name for commands.
    pub id: &'static str,
    /// What a menu shows.
    pub name: &'static str,
    /// The band names drawn as red, green and blue.
    pub channels: [&'static str; 3],
}

/// The combinations known, in the order they are offered.
pub const COMBINATIONS: [Combination; 7] = [
    Combination { id: "trueColor", name: "True color", channels: ["red", "green", "blue"] },
    Combination { id: "colorInfrared", name: "Color infrared", channels: ["nir", "red", "green"] },
    Combination { id: "shortwaveInfrared", name: "Shortwave infrared", channels: ["swir22", "nir", "red"] },
    Combination { id: "falseColorUrban", name: "False color urban", channels: ["swir22", "swir16", "red"] },
    Combination { id: "agriculture", name: "Agriculture", channels: ["swir16", "nir", "blue"] },
    Combination { id: "geology", name: "Geology", channels: ["swir22", "swir16", "blue"] },
    Combination { id: "bathymetric", name: "Bathymetric", channels: ["red", "green", "coastal"] },
];

/// Every band of the item's GeoTIFF assets, asset by asset (as the item lists them) and in file
/// order. An asset the catalog describes no bands for is one band.
pub fn bands(item: &Item) -> Vec<Band> {
    let mut out = Vec::new();
    for asset in item.assets.iter().filter(|a| a.is_geotiff()) {
        let count = asset.bands.len().max(1);
        for i in 0..count {
            if out.len() >= MAX_BANDS {
                return out;
            }
            // An asset that names no bands has its first band's numbers on the asset itself.
            let info = asset.bands.get(i);
            let (scale, offset, nodata) = match info {
                Some(b) => (b.scale, b.offset, b.nodata),
                None if i == 0 => (asset.scale, asset.offset, asset.nodata),
                None => (None, None, None),
            };
            out.push(Band {
                asset: asset.key.clone(),
                href: asset.href.clone(),
                index: u32::try_from(i + 1).unwrap_or(u32::MAX),
                common: info.map(|b| b.common.clone()).unwrap_or_default(),
                scale: scale.unwrap_or(1.0),
                offset: offset.unwrap_or(0.0),
                nodata,
            });
        }
    }
    out
}

/// The band `wanted` names (`red`, `nir`…): the first of its stand-ins (see [`ALIASES`]) that some
/// band has as its common name, else that an asset of exactly one band has as its key. A key of an
/// asset with several bands says nothing of which one is meant, so it is not taken for one.
pub fn find<'a>(bands: &'a [Band], wanted: &str) -> Option<&'a Band> {
    let wanted = wanted.trim().to_ascii_lowercase();
    let names: Vec<&str> = match ALIASES.iter().find(|(name, _)| *name == wanted) {
        Some((_, stand_ins)) => stand_ins.to_vec(),
        None => vec![wanted.as_str()],
    };
    names.iter().find_map(|name| {
        bands.iter().find(|b| b.common == *name).or_else(|| {
            let mut of_asset = bands.iter().filter(|b| b.asset.eq_ignore_ascii_case(name));
            match (of_asset.next(), of_asset.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        })
    })
}

/// The combinations the item can make, each with the bands found for its red, green and blue:
/// those whose three bands it has, in the order of [`COMBINATIONS`].
pub fn available(bands: &[Band]) -> Vec<(&'static Combination, [Band; 3])> {
    COMBINATIONS
        .iter()
        .filter_map(|c| {
            let [r, g, b] = c.channels;
            Some((c, [find(bands, r)?.clone(), find(bands, g)?.clone(), find(bands, b)?.clone()]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stac::parse_items;
    use serde_json::json;

    /// An item with these assets (key, `eo:bands`/`raster:bands` JSON or null).
    fn item(assets: &[(&str, serde_json::Value)]) -> Item {
        let assets: serde_json::Map<String, serde_json::Value> = assets
            .iter()
            .map(|(key, extra)| {
                let mut a =
                    json!({"href": format!("https://x.org/{key}.tif"), "type": "image/tiff; application=geotiff; profile=cloud-optimized"});
                if let (Some(a), Some(e)) = (a.as_object_mut(), extra.as_object()) {
                    a.extend(e.clone());
                }
                (key.to_string(), a)
            })
            .collect();
        let body = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "id": "i", "bbox": [0, 0, 1, 1], "properties": {"datetime": "2024-06-01T00:00:00Z"}, "assets": assets}
        ]})
        .to_string();
        parse_items(body.as_bytes(), "https://x.org/search").unwrap().0.remove(0)
    }

    fn named(common: &str) -> serde_json::Value {
        json!({"eo:bands": [{"common_name": common}], "raster:bands": [{"scale": 0.0001, "offset": -0.1, "nodata": 0}]})
    }

    /// A Sentinel-2 style item: a file a band, the names in `eo:bands`.
    fn sentinel() -> Item {
        item(&[
            ("blue", named("blue")),
            ("green", named("green")),
            ("red", named("red")),
            ("nir", named("nir")),
            ("nir08", named("nir08")),
            ("swir16", named("swir16")),
            ("swir22", named("swir22")),
            ("thumbnail", json!({"type": "image/png", "href": "https://x.org/thumbnail.png"})),
        ])
    }

    #[test]
    fn an_items_bands_come_from_its_geotiff_assets_with_their_numbers() {
        let b = bands(&sentinel());
        assert_eq!(b.len(), 7, "the thumbnail is not a band");
        let red = find(&b, "red").unwrap();
        assert_eq!((red.asset.as_str(), red.index, red.common.as_str()), ("red", 1, "red"));
        assert_eq!((red.scale, red.offset, red.nodata), (0.0001, -0.1, Some(0.0)));
    }

    #[test]
    fn a_name_is_found_by_its_stand_ins_best_first() {
        let b = bands(&sentinel());
        // Both `nir` and `nir08` are there: the wanted name's own is the best.
        assert_eq!(find(&b, "nir").unwrap().asset, "nir");
        // Landsat has only `nir08`: it stands in for `nir`.
        let landsat = bands(&item(&[("red", named("red")), ("nir08", named("nir08"))]));
        assert_eq!(find(&landsat, "nir").unwrap().asset, "nir08");
        assert_eq!(find(&landsat, " NIR ").unwrap().asset, "nir08", "case and spaces are not part of a name");
        let old = bands(&item(&[("swir1", named("swir1")), ("swir2", named("swir2"))]));
        assert_eq!((find(&old, "swir16").unwrap().asset.as_str(), find(&old, "swir22").unwrap().asset.as_str()), ("swir1", "swir2"));
        assert!(find(&b, "lwir").is_none());
    }

    #[test]
    fn an_asset_with_no_names_is_known_by_its_key_only_when_it_is_one_band() {
        // Named by key alone: one band each.
        let by_key = bands(&item(&[("red", json!({})), ("green", json!({})), ("blue", json!({}))]));
        assert_eq!(find(&by_key, "green").unwrap().asset, "green");
        // A file of several bands, none named: its key doesn't say which one is red.
        let stack = bands(&item(&[("red", json!({"raster:bands": [{}, {}, {}]}))]));
        assert_eq!(stack.len(), 3);
        assert!(find(&stack, "red").is_none());
        // Named inside the file: each is found by its own name and number.
        let named_stack = bands(&item(&[(
            "analytic",
            json!({"eo:bands": [{"common_name": "blue"}, {"common_name": "green"}, {"common_name": "red"}, {"common_name": "nir"}]}),
        )]));
        assert_eq!((find(&named_stack, "red").unwrap().index, find(&named_stack, "nir").unwrap().index), (3, 4));
    }

    #[test]
    fn only_the_combinations_an_item_can_make_are_offered() {
        let all = available(&bands(&sentinel()));
        let ids: Vec<&str> = all.iter().map(|(c, _)| c.id).collect();
        assert_eq!(
            ids,
            ["trueColor", "colorInfrared", "shortwaveInfrared", "falseColorUrban", "agriculture", "geology"],
            "no coastal band: no bathymetric"
        );
        let (_, [r, g, b]) = &all[1];
        assert_eq!((r.asset.as_str(), g.asset.as_str(), b.asset.as_str()), ("nir", "red", "green"), "colour infrared is nir, red, green");
        // Only visible light: true color and nothing else.
        let visible = available(&bands(&item(&[("red", named("red")), ("green", named("green")), ("blue", named("blue"))])));
        assert_eq!(visible.iter().map(|(c, _)| c.id).collect::<Vec<_>>(), ["trueColor"]);
        assert!(available(&bands(&item(&[("red", named("red"))]))).is_empty());
        assert!(available(&[]).is_empty());
    }

    #[test]
    fn an_item_passed_on_as_json_keeps_the_bands_it_needs() {
        // What a search answers, handed to a command: the names and the scaling survive.
        let original = sentinel();
        let again = crate::stac::item_from_json(&crate::stac::item_json(&original), None).unwrap();
        assert_eq!(bands(&again), bands(&original));
        assert_eq!(available(&bands(&again)).len(), 6);
        // A file of three named bands keeps each one's name and place.
        let stack = item(&[("analytic", json!({"eo:bands": [{"common_name": "blue"}, {"common_name": "green"}, {"common_name": "red"}]}))]);
        let again = crate::stac::item_from_json(&crate::stac::item_json(&stack), None).unwrap();
        assert_eq!(find(&bands(&again), "red").map(|b| b.index), Some(3));
        // An asset that names no band but has a scale keeps it, and one with none writes no `bands`.
        let scaled = item(&[("red", json!({"raster:bands": [{"scale": 0.5, "offset": 1.0}]})), ("green", json!({}))]);
        let again = crate::stac::item_from_json(&crate::stac::item_json(&scaled), None).unwrap();
        let (red, green) = (find(&bands(&again), "red").unwrap().clone(), find(&bands(&again), "green").unwrap().clone());
        assert_eq!((red.scale, red.offset), (0.5, 1.0));
        assert_eq!((green.scale, green.offset), (1.0, 0.0));
        let written = crate::stac::item_json(&scaled);
        assert!(written["assets"]["green"].get("bands").is_none(), "nothing to say of a band the catalog describes none of");
    }

    #[test]
    fn the_unified_bands_of_stac_1_1_are_read_like_eo_bands() {
        let unified = json!({"bands": [{"eo:common_name": "Red", "raster:scale": 0.5, "raster:offset": 2.0, "nodata": 255}]});
        let b = bands(&item(&[("data", unified)]));
        let red = find(&b, "red").unwrap();
        assert_eq!((red.scale, red.offset, red.nodata), (0.5, 2.0, Some(255.0)));
    }

    #[test]
    fn hostile_band_lists_are_bounded_and_odd_numbers_are_not_taken() {
        // More bands than are kept, and a no-data that is not a number.
        let many: Vec<serde_json::Value> = (0..500).map(|_| json!({"common_name": "red"})).collect();
        let b = bands(&item(&[("big", json!({"eo:bands": many}))]));
        assert_eq!(b.len(), 64, "one asset's bands are cut");
        let nan = bands(&item(&[("red", json!({"eo:bands": [{"common_name": "red"}], "raster:bands": [{"nodata": "nan", "scale": 0}]}))]));
        assert_eq!(nan[0].nodata, None, "\"nan\" is not a number");
        assert_eq!(nan[0].scale, 0.0, "a stated scale is kept as stated");
        // A name cut to a sane length, and nothing panics on junk.
        let junk = bands(&item(&[("x", json!({"eo:bands": [1, "a", null, {"common_name": 7}], "raster:bands": "no"}))]));
        assert_eq!(junk.len(), 4);
        assert!(find(&junk, "red").is_none());
    }
}
