//! Naming and numbering a coordinate reference system from its well-known text,
//! following Kart's `crs_util` module.

use sha2::{Digest, Sha256};

const CUSTOM_SRS_ID_MINIMUM: i32 = 200_000;
// kart's custom range is 200000 to 209199 inclusive
const CUSTOM_SRS_ID_COUNT: u32 = 9_200;
// kart ignores these authority strings when they stand in for a real identifier
const UNUSABLE_AUTHORITY_STRINGS: [&str; 3] = ["0", "EPSG", "ESRI"];

pub fn crs_name(definition: &str) -> Option<&str> {
    let name = definition.split('"').nth(1)?.trim();
    (!name.is_empty()).then_some(name)
}

// the outermost node is the last one closed, so its authority is the last in the text
pub fn crs_authority(definition: &str) -> (Option<&str>, Option<&str>) {
    let Some(start) = definition.rfind("AUTHORITY[") else {
        return (None, None);
    };
    let mut quoted = definition[start..]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .map(|value| (!value.is_empty()).then_some(value));
    (quoted.next().flatten(), quoted.next().flatten())
}

pub fn crs_organization(definition: &str) -> String {
    crs_authority(definition).0.unwrap_or("NONE").to_string()
}

/// The identifier a dataset stores the CRS under, as in `meta/crs/EPSG:4326.wkt`.
pub fn crs_identifier(definition: &str) -> Option<String> {
    Some(identifier_string(definition)?.replace('/', "_"))
}

/// The srs id a GeoPackage working copy declares for the CRS, `None` when the
/// definition carries neither an authority nor a name.
pub fn crs_srs_id(definition: &str) -> Option<i32> {
    if let Some(code) = positive_authority_code(definition) {
        return Some(code);
    }
    Some(custom_srs_id(&identifier_string(definition)?))
}

fn positive_authority_code(definition: &str) -> Option<i32> {
    let code = crs_authority(definition).1?;
    if !code.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    code.parse::<i32>().ok().filter(|code| *code > 0)
}

fn identifier_string(definition: &str) -> Option<String> {
    let (authority, code) = crs_authority(definition);
    if let (Some(authority), Some(code)) = (authority, code) {
        return Some(format!("{authority}:{code}"));
    }
    let single = authority
        .or(code)
        .filter(|value| !UNUSABLE_AUTHORITY_STRINGS.contains(value));
    match single {
        Some(single) => Some(single.to_string()),
        None => crs_name(definition).map(str::to_string),
    }
}

fn custom_srs_id(identifier: &str) -> i32 {
    let digest = Sha256::digest(identifier.as_bytes());
    let hash = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (hash % CUSTOM_SRS_ID_COUNT) as i32 + CUSTOM_SRS_ID_MINIMUM
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTED_WITH_AUTHORITY: &str = concat!(
        "PROJCS[\"NZGD2000 / New Zealand Transverse Mercator 2000\",",
        "GEOGCS[\"NZGD2000\",AUTHORITY[\"EPSG\",\"4167\"]],AUTHORITY[\"EPSG\",\"2193\"]]"
    );
    const PROJECTED_WITHOUT_AUTHORITY: &str = concat!(
        "PROJCS[\"NZGD2000 / New Zealand Transverse Mercator 2000\",",
        "GEOGCS[\"NZGD2000\",DATUM[\"New_Zealand_Geodetic_Datum_2000\",",
        "SPHEROID[\"GRS 1980\",6378137,298.257222101]],PRIMEM[\"Greenwich\",0],",
        "UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],UNIT[\"metre\",1]]"
    );

    #[test]
    fn test_identifier_prefers_the_outermost_authority() {
        assert_eq!(
            crs_identifier(PROJECTED_WITH_AUTHORITY).unwrap(),
            "EPSG:2193"
        );
    }

    #[test]
    fn test_identifier_falls_back_to_the_name() {
        assert_eq!(
            crs_identifier(PROJECTED_WITHOUT_AUTHORITY).unwrap(),
            "NZGD2000 _ New Zealand Transverse Mercator 2000"
        );
    }

    #[test]
    fn test_srs_id_is_the_authority_code() {
        assert_eq!(crs_srs_id(PROJECTED_WITH_AUTHORITY), Some(2193));
        assert_eq!(crs_organization(PROJECTED_WITH_AUTHORITY), "EPSG");
    }

    // the number kart's uint32hash of the crs name produces, see kart/crs_util.py
    #[test]
    fn test_srs_id_of_a_custom_crs_is_in_karts_range() {
        assert_eq!(crs_srs_id(PROJECTED_WITHOUT_AUTHORITY), Some(205617));
        assert_eq!(crs_organization(PROJECTED_WITHOUT_AUTHORITY), "NONE");
        assert_eq!(
            crs_name(PROJECTED_WITHOUT_AUTHORITY),
            Some("NZGD2000 / New Zealand Transverse Mercator 2000")
        );
    }

    #[test]
    fn test_srs_id_ignores_a_zero_authority_code() {
        let definition = "GEOGCS[\"Unnamed\",AUTHORITY[\"EPSG\",\"0\"]]";
        assert_eq!(crs_srs_id(definition), Some(205587));
    }

    #[test]
    fn test_definition_without_a_name_has_no_srs_id() {
        assert_eq!(crs_srs_id("GEOGCS[]"), None);
        assert_eq!(crs_identifier("GEOGCS[]"), None);
    }
}
