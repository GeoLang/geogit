//! GeoPackage Binary geometry encoding.
//!
//! Kart stores geometries using the Standard GeoPackageBinary format
//! specified in GeoPackage v1.3.0 §2.1.3, with restrictions:
//! - Always little-endian
//! - SRS ID always 0 (CRS stored in schema, not per-geometry)
//! - Non-empty non-Point geometries must have an envelope
//! - Points and empty geometries have no envelope

use crate::value::ColumnValue;
use geozero::wkb::{GpkgWkb, Wkb};
use geozero::wkt::Wkt;
use geozero::{CoordDimensions, ToJson, ToWkb};

/// GeoPackage binary header magic bytes
const GP_MAGIC: [u8; 2] = [0x47, 0x50]; // "GP"
const GP_VERSION: u8 = 0x00;

/// Envelope types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvelopeType {
    None = 0,
    Xy = 1,
    Xyz = 2,
    Xym = 3,
    Xyzm = 4,
}

/// A GeoPackage Binary encoded geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct GpkgGeometry {
    pub data: Vec<u8>,
}

impl GpkgGeometry {
    /// Create a GeoPackage Binary from raw WKB geometry bytes.
    ///
    /// Wraps the WKB in a GeoPackage binary header with the appropriate envelope.
    pub fn from_wkb(wkb: &[u8], envelope: Option<Envelope>) -> Self {
        let mut data = Vec::new();

        // Magic
        data.extend_from_slice(&GP_MAGIC);

        // Version
        data.push(GP_VERSION);

        // Flags byte: bit layout (LE):
        // bit 0: byte order (1 = little-endian)
        // bits 1-3: envelope type
        // bit 4: empty geometry flag
        // bit 5: GeoPackageBinary type (0 = standard)
        let envelope_type = match &envelope {
            Some(env) => env.envelope_type() as u8,
            None => 0,
        };
        let flags: u8 = 0x01 | (envelope_type << 1); // LE + envelope type
        data.push(flags);

        // SRS ID (always 0, LE i32)
        data.extend_from_slice(&0i32.to_le_bytes());

        // Envelope (if present)
        if let Some(env) = &envelope {
            env.write_to(&mut data);
        }

        // WKB payload
        data.extend_from_slice(wkb);

        Self { data }
    }

    /// Extract the raw WKB payload from the GeoPackage Binary.
    pub fn to_wkb(&self) -> Result<&[u8], GeometryError> {
        Self::wkb_payload(&self.data)
    }

    /// Extract the raw WKB payload from GeoPackage Binary bytes.
    pub fn wkb_payload(data: &[u8]) -> Result<&[u8], GeometryError> {
        if data.len() < 8 {
            return Err(GeometryError::TooShort);
        }
        if data[0..2] != GP_MAGIC {
            return Err(GeometryError::InvalidMagic);
        }

        let flags = data[3];
        let envelope_type = (flags >> 1) & 0x07;

        let envelope_size = match envelope_type {
            0 => 0,
            1 => 32,     // 4 doubles (minx, maxx, miny, maxy)
            2 | 3 => 48, // 6 doubles (+ z or m range)
            4 => 64,     // 8 doubles (+ z and m range)
            _ => return Err(GeometryError::InvalidEnvelopeType(envelope_type)),
        };

        let wkb_offset = 8 + envelope_size;
        if data.len() < wkb_offset {
            return Err(GeometryError::TooShort);
        }

        Ok(&data[wkb_offset..])
    }

    /// Get the raw bytes for MessagePack storage.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

/// Bounding box envelope for GeoPackage Binary.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
    pub min_z: Option<f64>,
    pub max_z: Option<f64>,
    pub min_m: Option<f64>,
    pub max_m: Option<f64>,
}

impl Envelope {
    pub fn xy(min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
            min_z: None,
            max_z: None,
            min_m: None,
            max_m: None,
        }
    }

    fn envelope_type(&self) -> EnvelopeType {
        match (self.min_z.is_some(), self.min_m.is_some()) {
            (false, false) => EnvelopeType::Xy,
            (true, false) => EnvelopeType::Xyz,
            (false, true) => EnvelopeType::Xym,
            (true, true) => EnvelopeType::Xyzm,
        }
    }

    fn write_to(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.min_x.to_le_bytes());
        buf.extend_from_slice(&self.max_x.to_le_bytes());
        buf.extend_from_slice(&self.min_y.to_le_bytes());
        buf.extend_from_slice(&self.max_y.to_le_bytes());
        if let Some(z) = self.min_z {
            buf.extend_from_slice(&z.to_le_bytes());
            buf.extend_from_slice(&self.max_z.unwrap_or(z).to_le_bytes());
        }
        if let Some(m) = self.min_m {
            buf.extend_from_slice(&m.to_le_bytes());
            buf.extend_from_slice(&self.max_m.unwrap_or(m).to_le_bytes());
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GeometryError {
    #[error("geometry data too short")]
    TooShort,
    #[error("invalid GeoPackage binary magic bytes")]
    InvalidMagic,
    #[error("invalid envelope type: {0}")]
    InvalidEnvelopeType(u8),
}

/// Convert a stored geometry column value (WKT text or WKB/GPKG blob) to GeoJSON geometry.
pub fn geometry_value_to_geojson(value: &ColumnValue) -> serde_json::Value {
    match value {
        ColumnValue::Text(wkt) => wkt_to_geojson(wkt),
        ColumnValue::Blob(data) => bytes_to_geojson(data),
        _ => serde_json::Value::Null,
    }
}

/// Convert WKT into a stored geometry value (GeoPackage Binary blob, or WKT on failure).
pub fn geometry_value_from_wkt(wkt: &str) -> ColumnValue {
    match wkt_to_gpkg_bytes(wkt, None) {
        Some(blob) => ColumnValue::Blob(blob),
        None => ColumnValue::Text(wkt.to_string()),
    }
}

/// Convert WKT into GeoPackage Binary bytes.
pub fn wkt_to_gpkg_bytes(wkt: &str, envelope: Option<Envelope>) -> Option<Vec<u8>> {
    let wkb = Wkt(wkt).to_wkb(CoordDimensions::xy()).ok()?;
    Some(GpkgGeometry::from_wkb(&wkb, envelope).data)
}

fn wkt_to_geojson(wkt: &str) -> serde_json::Value {
    json_from_geozero(Wkt(wkt).to_json())
}

fn bytes_to_geojson(data: &[u8]) -> serde_json::Value {
    if let Ok(wkb) = GpkgGeometry::wkb_payload(data) {
        let value = json_from_geozero(Wkb(wkb).to_json());
        if !value.is_null() {
            return value;
        }
    }
    let value = json_from_geozero(GpkgWkb(data).to_json());
    if !value.is_null() {
        return value;
    }
    json_from_geozero(Wkb(data).to_json())
}

fn json_from_geozero(result: geozero::error::Result<String>) -> serde_json::Value {
    result
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpkg_geometry_roundtrip() {
        // A simple WKB point (LE): type=1 (Point), x=1.0, y=2.0
        let mut wkb = Vec::new();
        wkb.push(0x01); // LE byte order
        wkb.extend_from_slice(&1u32.to_le_bytes()); // type = Point
        wkb.extend_from_slice(&1.0f64.to_le_bytes()); // x
        wkb.extend_from_slice(&2.0f64.to_le_bytes()); // y

        // Points have no envelope
        let gpkg = GpkgGeometry::from_wkb(&wkb, None);
        assert_eq!(&gpkg.data[0..2], &GP_MAGIC);

        let extracted_wkb = gpkg.to_wkb().unwrap();
        assert_eq!(extracted_wkb, &wkb);
    }

    #[test]
    fn test_gpkg_geometry_with_envelope() {
        let wkb = vec![0x01, 0x03, 0, 0, 0]; // Minimal polygon start
        let env = Envelope::xy(-180.0, 180.0, -90.0, 90.0);
        let gpkg = GpkgGeometry::from_wkb(&wkb, Some(env));

        let flags = gpkg.data[3];
        let env_type = (flags >> 1) & 0x07;
        assert_eq!(env_type, 1); // XY envelope

        let extracted = gpkg.to_wkb().unwrap();
        assert_eq!(extracted, &wkb);
    }

    fn point_wkb(x: f64, y: f64) -> Vec<u8> {
        let mut wkb = Vec::new();
        wkb.push(0x01);
        wkb.extend_from_slice(&1u32.to_le_bytes());
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
        wkb
    }

    #[test]
    fn test_wkt_point_to_geojson_coordinates() {
        let json = geometry_value_to_geojson(&ColumnValue::Text("POINT(139.6917 35.6895)".into()));
        assert_eq!(json["type"], "Point");
        assert!((json["coordinates"][0].as_f64().unwrap() - 139.6917).abs() < 1e-9);
        assert!((json["coordinates"][1].as_f64().unwrap() - 35.6895).abs() < 1e-9);
    }

    #[test]
    fn test_gpkg_blob_point_to_geojson_coordinates() {
        let gpkg = GpkgGeometry::from_wkb(&point_wkb(10.0, -20.0), None);
        let json = geometry_value_to_geojson(&ColumnValue::Blob(gpkg.data));
        assert_eq!(json["type"], "Point");
        assert_eq!(json["coordinates"][0], 10.0);
        assert_eq!(json["coordinates"][1], -20.0);
    }

    #[test]
    fn test_raw_wkb_point_to_geojson_coordinates() {
        let json = geometry_value_to_geojson(&ColumnValue::Blob(point_wkb(1.5, 2.5)));
        assert_eq!(json["type"], "Point");
        assert_eq!(json["coordinates"][0], 1.5);
        assert_eq!(json["coordinates"][1], 2.5);
    }

    #[test]
    fn test_wkt_roundtrip_to_gpkg_blob() {
        let value = geometry_value_from_wkt("POINT(1 2)");
        match &value {
            ColumnValue::Blob(data) => {
                assert_ne!(data.as_slice(), b"GEOMETRY");
                assert_eq!(&data[0..2], &GP_MAGIC);
            }
            ColumnValue::Text(wkt) => assert_eq!(wkt, "POINT(1 2)"),
            other => panic!("unexpected geometry value: {other:?}"),
        }
        let json = geometry_value_to_geojson(&value);
        assert_eq!(json["type"], "Point");
        assert_eq!(json["coordinates"][0], 1.0);
        assert_eq!(json["coordinates"][1], 2.0);
    }
}
