//! Reading JSON another program wrote, or a person, as types, as leniently as reading it by
//! key did: what is not there, or is not of its field's type, reads as that field's default,
//! and nothing reads as an object but an object (serde reads a struct out of a list too, by
//! position). Each field of a type read so reads with one of these, or is of a type that
//! reads out of anything, so an object always reads.

use serde::de::{DeserializeOwned, Deserializer};
use serde::Deserialize;
use serde_json::Value;

/// `T` read from `value` where it is an object, else `T`'s default.
pub(crate) fn object_of<T: DeserializeOwned + Default>(value: Value) -> T {
    match value {
        Value::Object(_) => serde_json::from_value(value).unwrap_or_default(),
        _ => T::default(),
    }
}

/// A field as said, or its default where what is said is not of its type.
pub(crate) fn as_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
}

/// A field that is an object, read as [`object_of`] reads one.
pub(crate) fn object<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(object_of(Value::deserialize(deserializer)?))
}

/// A list's objects, each read as [`object_of`] reads one, and what is not an object left out.
/// What is not a list is none.
pub(crate) fn objects<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Array(items) => Some(
            items
                .into_iter()
                .filter(Value::is_object)
                .map(object_of)
                .collect(),
        ),
        _ => None,
    })
}

/// A field said at all, `null` included, as [`as_said`] reads it: for a key that, once it is
/// there, is read in place of another even when it says nothing of use.
pub(crate) fn present<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    as_said(deserializer).map(Some)
}
