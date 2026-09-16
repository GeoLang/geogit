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
use geozero::{CoordDimensions, GeomProcessor, GeozeroGeometry, ToJson, ToWkb};

/// GeoPackage binary header magic bytes
const GP_MAGIC: [u8; 2] = [0x47, 0x50]; // "GP"
const GP_VERSION: u8 = 0x00;

const HEADER_SIZE: usize = 8;
const SRS_ID_OFFSET: usize = 4;
const KART_SRS_ID: i32 = 0;
const EMPTY_FLAG: u8 = 0b0001_0000;
const LITTLE_ENDIAN_FLAG: u8 = 0b0000_0001;
const ENVELOPE_FLAG_MASK: u8 = 0b0000_1110;
const EXTENDED_TYPE_FLAG: u8 = 0b0010_0000;
const WKB_LITTLE_ENDIAN: u8 = 0x01;
// iso wkb adds this to the type code once for z, twice for zm
const ISO_WKB_Z_OFFSET: u32 = 1000;
const WKB_POINT: u32 = 1;

/// Envelope types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvelopeType {
    None = 0,
    Xy = 1,
    Xyz = 2,
    Xym = 3,
    Xyzm = 4,
}

impl EnvelopeType {
    fn from_flags(flags: u8) -> Result<Self, GeometryError> {
        match (flags & ENVELOPE_FLAG_MASK) >> 1 {
            0 => Ok(EnvelopeType::None),
            1 => Ok(EnvelopeType::Xy),
            2 => Ok(EnvelopeType::Xyz),
            3 => Ok(EnvelopeType::Xym),
            4 => Ok(EnvelopeType::Xyzm),
            other => Err(GeometryError::InvalidEnvelopeType(other)),
        }
    }

    fn byte_size(self) -> usize {
        match self {
            EnvelopeType::None => 0,
            EnvelopeType::Xy => 32,
            EnvelopeType::Xyz | EnvelopeType::Xym => 48,
            EnvelopeType::Xyzm => 64,
        }
    }
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
        let flags: u8 = LITTLE_ENDIAN_FLAG | (envelope_type << 1);
        data.push(flags);

        data.extend_from_slice(&KART_SRS_ID.to_le_bytes());

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
        let offset = wkb_offset(data)?;
        Ok(&data[offset..])
    }

    /// Get the raw bytes for MessagePack storage.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

fn header_flags(data: &[u8]) -> Result<u8, GeometryError> {
    if data.len() < HEADER_SIZE {
        return Err(GeometryError::TooShort);
    }
    if data[0..2] != GP_MAGIC {
        return Err(GeometryError::InvalidMagic);
    }
    if data[2] != GP_VERSION {
        return Err(GeometryError::UnsupportedVersion(data[2]));
    }
    if data[3] & EXTENDED_TYPE_FLAG != 0 {
        return Err(GeometryError::ExtendedBinary);
    }
    Ok(data[3])
}

fn wkb_offset(data: &[u8]) -> Result<usize, GeometryError> {
    let flags = header_flags(data)?;
    let offset = HEADER_SIZE + EnvelopeType::from_flags(flags)?.byte_size();
    if data.len() <= offset {
        return Err(GeometryError::TooShort);
    }
    Ok(offset)
}

// kart wants srs id zero, little-endian bytes, and an envelope on every non-empty non-point geometry
pub fn normalise_gpkg_geometry(data: &[u8]) -> Result<Vec<u8>, GeometryError> {
    let flags = header_flags(data)?;
    let stored_envelope = EnvelopeType::from_flags(flags)?;
    let offset = wkb_offset(data)?;
    let wkb = &data[offset..];
    let wanted_envelope = desired_envelope_type(flags, wkb)?;

    let header_is_little_endian = flags & LITTLE_ENDIAN_FLAG != 0;
    let wkb_is_little_endian = wkb[0] == WKB_LITTLE_ENDIAN;

    if header_is_little_endian && wkb_is_little_endian && stored_envelope == wanted_envelope {
        let mut out = data.to_vec();
        out[SRS_ID_OFFSET..HEADER_SIZE].copy_from_slice(&KART_SRS_ID.to_le_bytes());
        return Ok(out);
    }

    let little_endian_wkb = if wkb_is_little_endian {
        wkb.to_vec()
    } else {
        Wkb(wkb)
            .to_wkb(wkb_dimensions(wkb)?)
            .map_err(|e| GeometryError::Reencode(e.to_string()))?
    };

    let envelope = match wanted_envelope {
        EnvelopeType::None => None,
        _ => Some(measure_envelope(&little_endian_wkb, wanted_envelope)?),
    };
    Ok(GpkgGeometry::from_wkb(&little_endian_wkb, envelope).data)
}

fn wkb_type_code(wkb: &[u8]) -> Result<u32, GeometryError> {
    if wkb.len() < 5 {
        return Err(GeometryError::TooShort);
    }
    let code = <[u8; 4]>::try_from(&wkb[1..5]).map_err(|_| GeometryError::TooShort)?;
    Ok(if wkb[0] == WKB_LITTLE_ENDIAN {
        u32::from_le_bytes(code)
    } else {
        u32::from_be_bytes(code)
    })
}

fn wkb_dimensions(wkb: &[u8]) -> Result<CoordDimensions, GeometryError> {
    let code = wkb_type_code(wkb)?;
    Ok(match code / ISO_WKB_Z_OFFSET {
        1 => CoordDimensions::xyz(),
        2 => CoordDimensions::xym(),
        3 => CoordDimensions::xyzm(),
        _ => CoordDimensions::xy(),
    })
}

// kart gives points and empty geometries no envelope, z geometries an xyz envelope, the rest xy
fn desired_envelope_type(flags: u8, wkb: &[u8]) -> Result<EnvelopeType, GeometryError> {
    if flags & EMPTY_FLAG != 0 {
        return Ok(EnvelopeType::None);
    }
    let code = wkb_type_code(wkb)?;
    if code % ISO_WKB_Z_OFFSET == WKB_POINT {
        return Ok(EnvelopeType::None);
    }
    let has_z = matches!(code / ISO_WKB_Z_OFFSET, 1 | 3);
    Ok(if has_z {
        EnvelopeType::Xyz
    } else {
        EnvelopeType::Xy
    })
}

#[derive(Default)]
struct BoundsScanner {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    min_z: f64,
    max_z: f64,
    seen: bool,
}

impl BoundsScanner {
    fn add(&mut self, x: f64, y: f64, z: Option<f64>) {
        if !self.seen {
            self.seen = true;
            self.min_x = x;
            self.max_x = x;
            self.min_y = y;
            self.max_y = y;
            let z = z.unwrap_or(0.0);
            self.min_z = z;
            self.max_z = z;
            return;
        }
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
        if let Some(z) = z {
            self.min_z = self.min_z.min(z);
            self.max_z = self.max_z.max(z);
        }
    }
}

impl GeomProcessor for BoundsScanner {
    fn dimensions(&self) -> CoordDimensions {
        CoordDimensions::xyz()
    }

    fn xy(&mut self, x: f64, y: f64, _idx: usize) -> geozero::error::Result<()> {
        self.add(x, y, None);
        Ok(())
    }

    fn coordinate(
        &mut self,
        x: f64,
        y: f64,
        z: Option<f64>,
        _m: Option<f64>,
        _t: Option<f64>,
        _tm: Option<u64>,
        _idx: usize,
    ) -> geozero::error::Result<()> {
        self.add(x, y, z);
        Ok(())
    }
}

fn measure_envelope(wkb: &[u8], envelope_type: EnvelopeType) -> Result<Envelope, GeometryError> {
    let mut scanner = BoundsScanner::default();
    Wkb(wkb)
        .process_geom(&mut scanner)
        .map_err(|e| GeometryError::Reencode(e.to_string()))?;
    if !scanner.seen {
        return Err(GeometryError::NoCoordinates);
    }
    let mut envelope = Envelope::xy(scanner.min_x, scanner.max_x, scanner.min_y, scanner.max_y);
    if envelope_type == EnvelopeType::Xyz {
        envelope.min_z = Some(scanner.min_z);
        envelope.max_z = Some(scanner.max_z);
    }
    Ok(envelope)
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
    #[error("unsupported GeoPackage binary version: {0}")]
    UnsupportedVersion(u8),
    #[error("ExtendedGeoPackageBinary geometries are not supported")]
    ExtendedBinary,
    #[error("invalid envelope type: {0}")]
    InvalidEnvelopeType(u8),
    #[error("geometry has no coordinates to measure")]
    NoCoordinates,
    #[error("failed to re-encode geometry: {0}")]
    Reencode(String),
}

/// Convert a stored geometry column value (WKT text or WKB/GPKG blob) to GeoJSON geometry.
pub fn geometry_value_to_geojson(value: &ColumnValue) -> serde_json::Value {
    match value {
        ColumnValue::Text(wkt) => wkt_to_geojson(wkt),
        ColumnValue::Geometry(data) | ColumnValue::Blob(data) => bytes_to_geojson(data),
        _ => serde_json::Value::Null,
    }
}

/// Convert WKT into a stored geometry value (GeoPackage Binary blob, or WKT on failure).
pub fn geometry_value_from_wkt(wkt: &str) -> ColumnValue {
    match wkt_to_gpkg_bytes(wkt) {
        Some(blob) => ColumnValue::Geometry(blob),
        None => ColumnValue::Text(wkt.to_string()),
    }
}

/// Convert WKT into GeoPackage Binary bytes.
pub fn wkt_to_gpkg_bytes(wkt: &str) -> Option<Vec<u8>> {
    let wkb = Wkt(wkt).to_wkb(CoordDimensions::xy()).ok()?;
    let envelope_type = desired_envelope_type(LITTLE_ENDIAN_FLAG, &wkb).ok()?;
    let envelope = match envelope_type {
        EnvelopeType::None => None,
        _ => Some(measure_envelope(&wkb, envelope_type).ok()?),
    };
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

    fn point_wkb(x: f64, y: f64) -> Vec<u8> {
        let mut wkb = Vec::new();
        wkb.push(WKB_LITTLE_ENDIAN);
        wkb.extend_from_slice(&1u32.to_le_bytes());
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
        wkb
    }

    #[test]
    fn test_gpkg_geometry_roundtrip() {
        let wkb = point_wkb(1.0, 2.0);

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

    #[test]
    fn test_stored_geometry_has_zero_srs_id() {
        let mut source = GpkgGeometry::from_wkb(&point_wkb(1.0, 2.0), None).data;
        source[SRS_ID_OFFSET..HEADER_SIZE].copy_from_slice(&4326i32.to_le_bytes());

        let normalised = normalise_gpkg_geometry(&source).unwrap();
        assert_eq!(&normalised[SRS_ID_OFFSET..HEADER_SIZE], &[0, 0, 0, 0]);
        assert_eq!(
            GpkgGeometry::wkb_payload(&normalised).unwrap(),
            point_wkb(1.0, 2.0)
        );
    }

    #[test]
    fn test_normalise_adds_missing_envelope() {
        let wkb = Wkt("POLYGON((0 0,3 0,3 4,0 4,0 0))")
            .to_wkb(CoordDimensions::xy())
            .unwrap();
        let without_envelope = GpkgGeometry::from_wkb(&wkb, None).data;

        let normalised = normalise_gpkg_geometry(&without_envelope).unwrap();
        let flags = normalised[3];
        assert_eq!(EnvelopeType::from_flags(flags).unwrap(), EnvelopeType::Xy);
        let envelope = &normalised[HEADER_SIZE..HEADER_SIZE + 32];
        let value = |i: usize| f64::from_le_bytes(envelope[i * 8..i * 8 + 8].try_into().unwrap());
        assert_eq!(value(0), 0.0);
        assert_eq!(value(1), 3.0);
        assert_eq!(value(2), 0.0);
        assert_eq!(value(3), 4.0);
    }

    #[test]
    fn test_normalise_leaves_conforming_geometry_alone() {
        let stored = wkt_to_gpkg_bytes("POLYGON((0 0,3 0,3 4,0 4,0 0))").unwrap();
        assert_eq!(normalise_gpkg_geometry(&stored).unwrap(), stored);
    }

    #[test]
    fn test_point_from_wkt_has_no_envelope() {
        let stored = wkt_to_gpkg_bytes("POINT(1 2)").unwrap();
        assert_eq!(
            EnvelopeType::from_flags(stored[3]).unwrap(),
            EnvelopeType::None
        );
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
        let json = geometry_value_to_geojson(&ColumnValue::Geometry(gpkg.data));
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
            ColumnValue::Geometry(data) => {
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
