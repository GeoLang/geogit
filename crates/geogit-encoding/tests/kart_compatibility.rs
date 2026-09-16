use geogit_encoding::feature::StoredFeature;
use geogit_encoding::geometry::geometry_value_to_geojson;
use geogit_encoding::value::ColumnValue;

// koordinates/kart tests/data/points.tgz, repo commit 1582725, feature nz_pa_points_topo_150k/.table-dataset/feature/A/A/A/A/kQ0=
const KART_POINT_FEATURE: &[u8] = include_bytes!("fixtures/kart_point_feature.msgpack");
// koordinates/kart tests/data/polygons.tgz, repo commit 3f7166e, feature nz_waca_adjustments/.table-dataset/feature/A/F/b/4/kc4AFb4f
const KART_POLYGON_FEATURE: &[u8] = include_bytes!("fixtures/kart_polygon_feature.msgpack");

fn geometry_bytes(value: &ColumnValue) -> &[u8] {
    match value {
        ColumnValue::Geometry(data) => data,
        other => panic!("expected a geometry value, got {other:?}"),
    }
}

#[test]
fn decodes_kart_point_feature() {
    let feature = StoredFeature::from_msgpack(KART_POINT_FEATURE).unwrap();
    assert_eq!(
        feature.legend_hash,
        "5836ddaa0c9e5dd4438f730f30d4eb45d7dc615e"
    );
    assert_eq!(feature.values.len(), 5);

    let geometry = geometry_bytes(&feature.values[0]);
    assert_eq!(&geometry[0..2], b"GP");
    assert_eq!(geometry[3] >> 1 & 0x07, 0);
    assert_eq!(&geometry[4..8], &[0, 0, 0, 0]);

    let json = geometry_value_to_geojson(&feature.values[0]);
    assert_eq!(json["type"], "Point");
    assert!((json["coordinates"][0].as_f64().unwrap() - 177.617_035_916_675_23).abs() < 1e-9);
    assert!((json["coordinates"][1].as_f64().unwrap() + 37.870_441_856_462_22).abs() < 1e-9);

    assert_eq!(feature.values[1], ColumnValue::Integer(2_426_283));
    assert_eq!(feature.values[2], ColumnValue::Null);
    assert_eq!(feature.values[3], ColumnValue::Text("N".into()));
    assert_eq!(feature.values[4], ColumnValue::Null);
}

#[test]
fn decodes_kart_polygon_feature() {
    let feature = StoredFeature::from_msgpack(KART_POLYGON_FEATURE).unwrap();
    assert_eq!(
        feature.legend_hash,
        "efc9d810390e434ae25a63da0138f4f74423a87d"
    );
    assert_eq!(feature.values.len(), 4);

    let geometry = geometry_bytes(&feature.values[0]);
    assert_eq!(&geometry[0..2], b"GP");
    assert_eq!(geometry[3] >> 1 & 0x07, 1);
    assert_eq!(&geometry[4..8], &[0, 0, 0, 0]);
    let min_x = f64::from_le_bytes(geometry[8..16].try_into().unwrap());
    assert!((min_x - 175.350_319_216_7).abs() < 1e-9);

    let json = geometry_value_to_geojson(&feature.values[0]);
    assert_eq!(json["type"], "MultiPolygon");

    assert_eq!(
        feature.values[1],
        ColumnValue::Text("2011-03-25T07:30:45".into())
    );
    assert_eq!(feature.values[2], ColumnValue::Null);
    assert_eq!(feature.values[3], ColumnValue::Integer(1122));
}

#[test]
fn reencodes_kart_features_byte_for_byte() {
    for fixture in [KART_POINT_FEATURE, KART_POLYGON_FEATURE] {
        let feature = StoredFeature::from_msgpack(fixture).unwrap();
        assert_eq!(feature.to_msgpack(), fixture);
    }
}

#[test]
fn decodes_feature_written_before_the_geometry_extension() {
    let gpkg_bytes = geogit_encoding::geometry::wkt_to_gpkg_bytes("POINT(1 2)").unwrap();
    // the derived Serialize wrote a blob as an array of integers
    let old_shape = rmp_serde::to_vec(&("abc123", vec![gpkg_bytes.clone()])).unwrap();

    let feature = StoredFeature::from_msgpack(&old_shape).unwrap();
    assert_eq!(feature.legend_hash, "abc123");
    assert_eq!(feature.values[0], ColumnValue::Blob(gpkg_bytes));

    let json = geometry_value_to_geojson(&feature.values[0]);
    assert_eq!(json["type"], "Point");
    assert_eq!(json["coordinates"][0], 1.0);
    assert_eq!(json["coordinates"][1], 2.0);
}
