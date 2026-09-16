use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Column values stored in MessagePack features.
/// Follows Kart's serialization conventions.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Blob(Vec<u8>),
    Geometry(Vec<u8>),
}

// kart tags geometry with the msgpack extension code ord('G')
pub const GEOMETRY_EXTENSION_CODE: i8 = 71;

impl ColumnValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ColumnValue::Integer(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            ColumnValue::Text(v) => Some(v.as_str()),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ColumnValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ColumnValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            ColumnValue::Blob(v) | ColumnValue::Geometry(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, ColumnValue::Null)
    }
}

impl std::fmt::Display for ColumnValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColumnValue::Null => write!(f, "NULL"),
            ColumnValue::Bool(v) => write!(f, "{v}"),
            ColumnValue::Integer(v) => write!(f, "{v}"),
            ColumnValue::Float(v) => write!(f, "{v}"),
            ColumnValue::Text(v) => write!(f, "{v}"),
            ColumnValue::Blob(v) | ColumnValue::Geometry(v) => write!(f, "<{} bytes>", v.len()),
        }
    }
}

struct ByteString<'a>(&'a [u8]);

impl Serialize for ByteString<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

impl Serialize for ColumnValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ColumnValue::Null => serializer.serialize_unit(),
            ColumnValue::Bool(v) => serializer.serialize_bool(*v),
            ColumnValue::Integer(v) => serializer.serialize_i64(*v),
            ColumnValue::Float(v) => serializer.serialize_f64(*v),
            ColumnValue::Text(v) => serializer.serialize_str(v),
            ColumnValue::Blob(v) => serializer.serialize_bytes(v),
            ColumnValue::Geometry(v) => serializer.serialize_newtype_struct(
                rmp_serde::MSGPACK_EXT_STRUCT_NAME,
                &(GEOMETRY_EXTENSION_CODE, ByteString(v)),
            ),
        }
    }
}

struct ExtensionPayload(Vec<u8>);

impl<'de> Deserialize<'de> for ExtensionPayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PayloadVisitor;

        impl<'de> Visitor<'de> for PayloadVisitor {
            type Value = ExtensionPayload;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("msgpack extension payload bytes")
            }

            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(ExtensionPayload(v.to_vec()))
            }

            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(ExtensionPayload(v))
            }
        }

        deserializer.deserialize_bytes(PayloadVisitor)
    }
}

impl<'de> Deserialize<'de> for ColumnValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ColumnValueVisitor;

        impl<'de> Visitor<'de> for ColumnValueVisitor {
            type Value = ColumnValue;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a feature column value")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(ColumnValue::Null)
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(ColumnValue::Null)
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(ColumnValue::Bool(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(ColumnValue::Integer(v))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                i64::try_from(v)
                    .map(ColumnValue::Integer)
                    .map_err(|_| E::custom(format!("integer {v} does not fit in i64")))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(ColumnValue::Float(v))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ColumnValue::Text(v.to_string()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ColumnValue::Text(v))
            }

            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                Ok(ColumnValue::Blob(v.to_vec()))
            }

            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Ok(ColumnValue::Blob(v))
            }

            // repositories written before geometry used the extension type store blobs as integer arrays
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(byte) = seq.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(ColumnValue::Blob(bytes))
            }

            fn visit_newtype_struct<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                let (code, payload) = <(i8, ExtensionPayload)>::deserialize(deserializer)?;
                if code != GEOMETRY_EXTENSION_CODE {
                    return Err(de::Error::custom(format!(
                        "unsupported msgpack extension code {code}"
                    )));
                }
                Ok(ColumnValue::Geometry(payload.0))
            }
        }

        deserializer.deserialize_any(ColumnValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_value_accessors() {
        assert_eq!(ColumnValue::Integer(42).as_i64(), Some(42));
        assert_eq!(ColumnValue::Text("hello".into()).as_str(), Some("hello"));
        assert_eq!(ColumnValue::Float(2.72).as_f64(), Some(2.72));
        assert_eq!(ColumnValue::Bool(true).as_bool(), Some(true));
        assert!(ColumnValue::Null.is_null());
    }

    #[test]
    fn test_blob_serializes_as_msgpack_binary() {
        let bytes = rmp_serde::to_vec(&ColumnValue::Blob(vec![1, 2, 3])).unwrap();
        assert_eq!(bytes, vec![0xc4, 0x03, 1, 2, 3]);
        let decoded: ColumnValue = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded, ColumnValue::Blob(vec![1, 2, 3]));
    }

    #[test]
    fn test_geometry_serializes_as_extension_type() {
        let bytes =
            rmp_serde::to_vec(&ColumnValue::Geometry(vec![0x47, 0x50, 0x00, 0x01])).unwrap();
        assert_eq!(bytes, vec![0xd6, 71, 0x47, 0x50, 0x00, 0x01]);
        let decoded: ColumnValue = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded, ColumnValue::Geometry(vec![0x47, 0x50, 0x00, 0x01]));
    }

    #[test]
    fn test_integer_array_decodes_as_blob() {
        let old_shape = rmp_serde::to_vec(&vec![1u8, 2, 3]).unwrap();
        let decoded: ColumnValue = rmp_serde::from_slice(&old_shape).unwrap();
        assert_eq!(decoded, ColumnValue::Blob(vec![1, 2, 3]));
    }

    #[test]
    fn test_scalar_roundtrips() {
        for value in [
            ColumnValue::Null,
            ColumnValue::Bool(true),
            ColumnValue::Integer(-7),
            ColumnValue::Float(1.5),
            ColumnValue::Text("hello".into()),
        ] {
            let bytes = rmp_serde::to_vec(&value).unwrap();
            let decoded: ColumnValue = rmp_serde::from_slice(&bytes).unwrap();
            assert_eq!(decoded, value);
        }
    }
}
