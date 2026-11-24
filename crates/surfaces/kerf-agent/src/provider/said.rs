//! Reading what a provider said, field by field, as reading it by key did: a field said
//! otherwise than it is read here reads as its default, and the rest still reads, so one odd
//! field does not cost the event it is in. Each type here also ignores fields it does not
//! name, so a provider adding one changes nothing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// A field as said, or its default when what was said is not of its type.
pub(super) fn as_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(deserializer)?).unwrap_or_default())
}

/// What is said where an object belongs, read as `T` when it is one and as `T`'s default
/// otherwise, as a key looked up in anything else finds nothing. ([`as_said`] would read a
/// list as a `T` too, field by field.)
pub(super) fn object<T: DeserializeOwned + Default>(said: &Value) -> T {
    match said {
        Value::Object(_) => T::deserialize(said).unwrap_or_default(),
        _ => T::default(),
    }
}

/// A field where an object belongs, read as [`object`] reads it.
pub(super) fn object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(object(&Value::deserialize(deserializer)?))
}

/// An object whose being said at all matters: `Some` when it is, read as [`object`] reads it.
pub(super) fn some_object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    object_said(deserializer).map(Some)
}

/// A list of objects, each read on its own as [`object`] reads it: one said otherwise is still
/// an item, of its default. What is not a list has none.
pub(super) fn each_object<T: DeserializeOwned + Default>(said: &Value) -> Vec<T> {
    match said {
        Value::Array(items) => items.iter().map(object).collect(),
        _ => Vec::new(),
    }
}

/// A field that holds a list of objects, read as [`each_object`] reads it.
pub(super) fn each_object_said<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Ok(each_object(&Value::deserialize(deserializer)?))
}
