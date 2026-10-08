use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::util::is_valid_slug;

const CANONICAL_FEATURE_COUNT: usize = 76;

pub struct CanonicalFeature {
    pub id: String,
    pub name: String,
    pub properties: Map<String, Value>,
    pub geometry: Value,
}

/// The immutable canonical atlas read from `public/data/gubernias.geojson`.
/// Publication state and content live in SQLite; ids, names, labels and the
/// geometry itself stay file-owned exactly like the TypeScript store.
pub struct CanonicalProvinces {
    pub features: Vec<CanonicalFeature>,
}

impl CanonicalProvinces {
    pub fn load(path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(path).map_err(|error| {
            format!(
                "Cannot read canonical gubernia GeoJSON at {}: {error}",
                path.display()
            )
        })?;
        let parsed: Value = serde_json::from_str(&source).map_err(|error| {
            format!("Canonical gubernia GeoJSON contains invalid JSON: {error}")
        })?;
        Self::from_value(parsed)
    }

    pub fn from_value(value: Value) -> Result<Self, String> {
        let Value::Object(collection) = value else {
            return Err("Canonical gubernia GeoJSON is not an object.".to_string());
        };
        if collection.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
            return Err("Canonical gubernia GeoJSON is not a FeatureCollection.".to_string());
        }
        let Some(Value::Array(features)) = collection.get("features") else {
            return Err("Canonical gubernia GeoJSON is not a FeatureCollection.".to_string());
        };
        if features.len() != CANONICAL_FEATURE_COUNT {
            return Err(format!(
                "Canonical gubernia GeoJSON must contain 76 features, found {}.",
                features.len()
            ));
        }

        let mut parsed_features = Vec::with_capacity(features.len());
        let mut ids: Vec<String> = Vec::with_capacity(features.len());
        for feature in features {
            let Value::Object(feature) = feature else {
                return Err(
                    "Canonical gubernia GeoJSON contains an invalid feature.".to_string()
                );
            };
            if feature.get("type").and_then(Value::as_str) != Some("Feature") {
                return Err(
                    "Canonical gubernia GeoJSON contains an invalid feature.".to_string()
                );
            }
            let Some(geometry) = feature.get("geometry") else {
                return Err(
                    "Canonical gubernia GeoJSON contains an invalid feature.".to_string()
                );
            };
            if !geometry.is_object() {
                return Err(
                    "Canonical gubernia GeoJSON contains an invalid feature.".to_string()
                );
            }
            let Some(Value::Object(properties)) = feature.get("properties") else {
                return Err(
                    "Canonical gubernia GeoJSON contains invalid feature metadata.".to_string()
                );
            };
            let id = properties
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            let name = properties
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            let label_ok = match properties.get("labelCoordinates") {
                Some(Value::Array(items)) => {
                    items.len() == 2
                        && items
                            .iter()
                            .all(|item| item.as_f64().is_some_and(f64::is_finite))
                }
                _ => false,
            };
            let zoom_ok = properties
                .get("labelMinZoom")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite);
            let (Some(id), Some(name)) = (id, name) else {
                return Err(
                    "Canonical gubernia GeoJSON contains invalid feature metadata.".to_string()
                );
            };
            if !label_ok || !zoom_ok {
                return Err(
                    "Canonical gubernia GeoJSON contains invalid feature metadata.".to_string()
                );
            }
            if ids.iter().any(|existing| existing == id) {
                return Err(format!(
                    "Canonical gubernia GeoJSON contains duplicate id {id}."
                ));
            }
            if properties.get("published") == Some(&Value::Bool(true)) {
                let slug_ok = properties
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(is_valid_slug);
                if !slug_ok {
                    return Err(format!(
                        "Published canonical gubernia {id} has an invalid slug."
                    ));
                }
            }
            ids.push(id.to_string());
            parsed_features.push(CanonicalFeature {
                id: id.to_string(),
                name: name.to_string(),
                properties: properties.clone(),
                geometry: geometry.clone(),
            });
        }

        Ok(Self {
            features: parsed_features,
        })
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.features.iter().position(|feature| feature.id == id)
    }

    pub fn feature(&self, id: &str) -> Option<&CanonicalFeature> {
        self.index_of(id).map(|index| &self.features[index])
    }

    /// Merges stored publication state onto the canonical properties, the same
    /// way `getGuberniasCollection` replaced `published`/`slug`.
    pub fn merged_feature(
        feature: &CanonicalFeature,
        published: bool,
        slug: Option<&str>,
    ) -> Value {
        let mut properties = feature.properties.clone();
        properties.shift_remove("published");
        properties.shift_remove("slug");
        properties.insert("published".to_string(), Value::Bool(published));
        if let Some(slug) = slug {
            properties.insert("slug".to_string(), Value::String(slug.to_string()));
        }

        let mut out = Map::new();
        out.insert("type".to_string(), Value::String("Feature".to_string()));
        out.insert("geometry".to_string(), feature.geometry.clone());
        out.insert("properties".to_string(), Value::Object(properties));
        Value::Object(out)
    }
}

#[derive(Clone, Debug)]
pub enum DistrictGeometry {
    Polygon(Vec<Vec<[f64; 2]>>),
    MultiPolygon(Vec<Vec<Vec<[f64; 2]>>>),
}

#[derive(Clone)]
pub struct District {
    pub id: String,
    pub name: String,
    pub geometry: DistrictGeometry,
}

pub struct DistrictCollection {
    districts: Vec<District>,
}

impl DistrictCollection {
    pub fn empty() -> Self {
        Self {
            districts: Vec::new(),
        }
    }

    pub fn from_value(value: Value, province_id: &str) -> Result<Self, String> {
        let invalid = || format!("Historical district GeoJSON for {province_id} is invalid.");
        let Value::Object(collection) = value else {
            return Err(format!(
                "Historical district GeoJSON for {province_id} is not an object."
            ));
        };
        if collection.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
            return Err(invalid());
        }
        let Some(Value::Array(features)) = collection.get("features") else {
            return Err(invalid());
        };
        if features.is_empty() {
            return Err(invalid());
        }

        let mut iter = features.iter();
        let province = iter.next().expect("non-empty");
        let Value::Object(province) = province else {
            return Err(format!(
                "Historical district GeoJSON does not match province {province_id}."
            ));
        };
        if province.get("type").and_then(Value::as_str) != Some("Feature") {
            return Err(format!(
                "Historical district GeoJSON does not match province {province_id}."
            ));
        }
        let Some(Value::Object(province_properties)) = province.get("properties") else {
            return Err(format!(
                "Historical district GeoJSON does not match province {province_id}."
            ));
        };
        if province_properties.get("kind").and_then(Value::as_str) != Some("province")
            || province_properties.get("id").and_then(Value::as_str) != Some(province_id)
        {
            return Err(format!(
                "Historical district GeoJSON does not match province {province_id}."
            ));
        }

        let mut districts: Vec<District> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for raw_feature in iter {
            let Value::Object(feature) = raw_feature else {
                return Err(format!(
                    "Historical district GeoJSON for {province_id} contains an invalid feature."
                ));
            };
            let Some(Value::Object(properties)) = feature.get("properties") else {
                return Err(format!(
                    "Historical district GeoJSON for {province_id} contains an invalid feature."
                ));
            };
            let id = properties.get("id").and_then(Value::as_str);
            let name = properties.get("name").and_then(Value::as_str);
            let metadata_ok = feature.get("type").and_then(Value::as_str) == Some("Feature")
                && properties.get("kind").and_then(Value::as_str) == Some("district")
                && id.is_some_and(is_uyezd_id)
                && name.is_some_and(|value| !value.is_empty())
                && properties.get("provinceId").and_then(Value::as_str) == Some(province_id);
            if !metadata_ok {
                return Err(format!(
                    "Historical district GeoJSON for {province_id} has invalid metadata."
                ));
            }
            let (Some(id), Some(name)) = (id, name) else {
                return Err(format!(
                    "Historical district GeoJSON for {province_id} has invalid metadata."
                ));
            };
            if ids.iter().any(|existing| existing == id) {
                return Err(format!(
                    "Historical district GeoJSON contains duplicate id {id}."
                ));
            }
            let geometry = parse_geometry(feature.get("geometry"), id)
                .ok_or_else(|| format!("Historical district {id} has invalid geometry."))?;
            ids.push(id.to_string());
            districts.push(District {
                id: id.to_string(),
                name: name.to_string(),
                geometry,
            });
        }

        Ok(Self { districts })
    }

    pub fn get(&self, id: &str) -> Option<&District> {
        self.districts.iter().find(|district| district.id == id)
    }

    pub fn districts(&self) -> &[District] {
        &self.districts
    }
}

fn is_uyezd_id(id: &str) -> bool {
    id.strip_prefix("uyezd-1897-").is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn parse_position(value: &Value) -> Option<[f64; 2]> {
    let Value::Array(items) = value else {
        return None;
    };
    if items.len() < 2 {
        return None;
    }
    let x = items[0].as_f64().filter(|value| value.is_finite())?;
    let y = items[1].as_f64().filter(|value| value.is_finite())?;
    Some([x, y])
}

fn parse_ring(value: &Value) -> Option<Vec<[f64; 2]>> {
    let Value::Array(items) = value else {
        return None;
    };
    if items.len() < 4 {
        return None;
    }
    items.iter().map(parse_position).collect()
}

fn parse_polygon(value: &Value) -> Option<Vec<Vec<[f64; 2]>>> {
    let Value::Array(rings) = value else {
        return None;
    };
    if rings.is_empty() {
        return None;
    }
    rings.iter().map(parse_ring).collect()
}

fn parse_geometry(value: Option<&Value>, district_id: &str) -> Option<DistrictGeometry> {
    let value = value?;
    let record = value.as_object()?;
    let geometry_type = record.get("type")?.as_str()?;
    let coordinates = record.get("coordinates")?;
    match geometry_type {
        "Polygon" => parse_polygon(coordinates).map(DistrictGeometry::Polygon),
        "MultiPolygon" => {
            let Value::Array(polygons) = coordinates else {
                let _ = district_id;
                return None;
            };
            if polygons.is_empty() {
                return None;
            }
            polygons
                .iter()
                .map(parse_polygon)
                .collect::<Option<Vec<_>>>()
                .map(DistrictGeometry::MultiPolygon)
        }
        _ => None,
    }
}

/// Reads historical district files lazily and caches successful parses (a
/// missing file caches its empty collection, exactly like `loadCollection`).
pub struct GeoRuntime {
    directory: PathBuf,
    cache: Mutex<HashMap<String, Arc<DistrictCollection>>>,
}

impl GeoRuntime {
    pub fn new(public_data_dir: &Path) -> Self {
        Self {
            directory: public_data_dir.join("uyezds-1897"),
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn load(&self, province_id: &str) -> Result<Arc<DistrictCollection>, String> {
        if !is_valid_slug(province_id) {
            return Err(format!("Invalid province id {province_id}."));
        }
        if let Some(cached) = self.cached(province_id)? {
            return Ok(cached);
        }
        let path = self.district_path(province_id);
        let loaded = match fs::read_to_string(&path) {
            Ok(source) => self.parse(province_id, &source)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Arc::new(DistrictCollection::empty())
            }
            Err(error) => {
                return Err(format!(
                    "Cannot read district GeoJSON for {province_id}: {error}"
                ))
            }
        };
        self.remember(province_id, &loaded)?;
        Ok(loaded)
    }

    /// Strict variant for the importer: the canonical atlas ships every
    /// province's district file, so a missing or invalid file is a partial
    /// import and must fail instead of committing a partial registry. It reads
    /// from disk every time (import-time only), so a lenient runtime read can
    /// never mask a missing file here. Runtime map reads keep the lenient
    /// ENOENT-is-empty behaviour of `load`.
    pub fn load_required(&self, province_id: &str) -> Result<Arc<DistrictCollection>, String> {
        if !is_valid_slug(province_id) {
            return Err(format!("Invalid province id {province_id}."));
        }
        let path = self.district_path(province_id);
        let source = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Historical district GeoJSON for {province_id} is missing at {}; the canonical atlas must ship all 76 files.",
                    path.display()
                )
            } else {
                format!("Cannot read district GeoJSON for {province_id}: {error}")
            }
        })?;
        let loaded = self.parse(province_id, &source)?;
        self.remember(province_id, &loaded)?;
        Ok(loaded)
    }

    fn district_path(&self, province_id: &str) -> PathBuf {
        self.directory.join(format!("{province_id}.geojson"))
    }

    fn cached(&self, province_id: &str) -> Result<Option<Arc<DistrictCollection>>, String> {
        Ok(self
            .cache
            .lock()
            .map_err(|_| "district cache poisoned".to_string())?
            .get(province_id)
            .cloned())
    }

    fn remember(
        &self,
        province_id: &str,
        collection: &Arc<DistrictCollection>,
    ) -> Result<(), String> {
        self.cache
            .lock()
            .map_err(|_| "district cache poisoned".to_string())?
            .insert(province_id.to_string(), collection.clone());
        Ok(())
    }

    fn parse(&self, province_id: &str, source: &str) -> Result<Arc<DistrictCollection>, String> {
        let parsed: Value = serde_json::from_str(source).map_err(|error| {
            format!("Historical district GeoJSON for {province_id} contains invalid JSON: {error}")
        })?;
        Ok(Arc::new(DistrictCollection::from_value(parsed, province_id)?))
    }

    pub fn get_uyezd(&self, province_id: &str, uyezd_id: &str) -> Result<Option<String>, String> {
        Ok(self
            .load(province_id)?
            .get(uyezd_id)
            .map(|district| district.name.clone()))
    }

    pub fn is_point_in_uyezd(
        &self,
        province_id: &str,
        uyezd_id: &str,
        longitude: f64,
        latitude: f64,
    ) -> Result<bool, String> {
        let collection = self.load(province_id)?;
        let Some(district) = collection.get(uyezd_id) else {
            return Ok(false);
        };
        let point = [longitude, latitude];
        Ok(match &district.geometry {
            DistrictGeometry::Polygon(polygon) => point_in_polygon(point, polygon),
            DistrictGeometry::MultiPolygon(polygons) => polygons
                .iter()
                .any(|polygon| point_in_polygon(point, polygon)),
        })
    }
}

fn point_on_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> bool {
    let [x, y] = point;
    let [x1, y1] = start;
    let [x2, y2] = end;
    let cross = (x - x1) * (y2 - y1) - (y - y1) * (x2 - x1);
    let tolerance = f64::EPSILON
        * 32.0
        * 1.0f64
            .max(x.abs())
            .max(y.abs())
            .max(x1.abs())
            .max(y1.abs())
            .max(x2.abs())
            .max(y2.abs());
    if cross.abs() > tolerance {
        return false;
    }
    x >= x1.min(x2) - tolerance
        && x <= x1.max(x2) + tolerance
        && y >= y1.min(y2) - tolerance
        && y <= y1.max(y2) + tolerance
}

enum RingLocation {
    Outside,
    Inside,
    Boundary,
}

fn locate_in_ring(point: [f64; 2], ring: &[[f64; 2]]) -> RingLocation {
    let mut inside = false;
    let mut previous = ring.len() - 1;
    for index in 0..ring.len() {
        let start = ring[previous];
        let end = ring[index];
        if point_on_segment(point, start, end) {
            return RingLocation::Boundary;
        }
        let crosses = (start[1] > point[1]) != (end[1] > point[1])
            && point[0]
                < ((end[0] - start[0]) * (point[1] - start[1])) / (end[1] - start[1]) + start[0];
        if crosses {
            inside = !inside;
        }
        previous = index;
    }
    if inside {
        RingLocation::Inside
    } else {
        RingLocation::Outside
    }
}

fn point_in_polygon(point: [f64; 2], polygon: &[Vec<[f64; 2]>]) -> bool {
    match locate_in_ring(point, &polygon[0]) {
        RingLocation::Outside => false,
        RingLocation::Boundary => true,
        RingLocation::Inside => {
            for hole in &polygon[1..] {
                match locate_in_ring(point, hole) {
                    RingLocation::Inside => return false,
                    RingLocation::Boundary => return true,
                    RingLocation::Outside => {}
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn square(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<[f64; 2]> {
        vec![
            [min_x, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [min_x, max_y],
            [min_x, min_y],
        ]
    }

    #[test]
    fn point_in_polygon_includes_boundary_and_excludes_holes() {
        let polygon = vec![square(0.0, 0.0, 10.0, 10.0), square(4.0, 4.0, 6.0, 6.0)];
        assert!(point_in_polygon([5.0, 5.0], &polygon) == false);
        assert!(point_in_polygon([5.0, 2.0], &polygon));
        assert!(point_in_polygon([0.0, 5.0], &polygon));
        assert!(point_in_polygon([4.0, 5.0], &polygon));
        assert!(!point_in_polygon([-0.5, 5.0], &polygon));
        assert!(!point_in_polygon([11.0, 5.0], &polygon));
    }

    #[test]
    fn multipolygon_accepts_any_landmass() {
        let geometry = DistrictGeometry::MultiPolygon(vec![
            vec![square(0.0, 0.0, 1.0, 1.0)],
            vec![square(5.0, 5.0, 6.0, 6.0)],
        ]);
        let point = [5.5, 5.5];
        let inside = match &geometry {
            DistrictGeometry::Polygon(polygon) => point_in_polygon(point, polygon),
            DistrictGeometry::MultiPolygon(polygons) => {
                polygons.iter().any(|polygon| point_in_polygon(point, polygon))
            }
        };
        assert!(inside);
    }

    #[test]
    fn canonical_validation_rejects_wrong_feature_count() {
        let value = json!({ "type": "FeatureCollection", "features": [] });
        let error = match CanonicalProvinces::from_value(value) {
            Ok(_) => panic!("expected validation failure"),
            Err(error) => error,
        };
        assert!(error.contains("must contain 76 features"), "{error}");
    }

    #[test]
    fn district_validation_checks_province_and_metadata() {
        let value = json!({
            "type": "FeatureCollection",
            "features": [
                { "type": "Feature", "properties": { "kind": "province", "id": "ryazan" }, "geometry": { "type": "Polygon", "coordinates": [square(0.0, 0.0, 1.0, 1.0)] } },
                { "type": "Feature", "properties": { "kind": "district", "id": "uyezd-1897-353", "name": "Скопинский уезд", "provinceId": "ryazan" }, "geometry": { "type": "MultiPolygon", "coordinates": [[square(0.0, 0.0, 0.5, 0.5)]] } }
            ]
        });
        let collection = DistrictCollection::from_value(value, "ryazan").unwrap();
        assert!(collection.get("uyezd-1897-353").is_some());
        assert!(collection.get("uyezd-1897-999").is_none());

        let wrong_province = json!({
            "type": "FeatureCollection",
            "features": [
                { "type": "Feature", "properties": { "kind": "province", "id": "tula" }, "geometry": { "type": "Polygon", "coordinates": [square(0.0, 0.0, 1.0, 1.0)] } }
            ]
        });
        assert!(DistrictCollection::from_value(wrong_province, "ryazan").is_err());
    }
}
