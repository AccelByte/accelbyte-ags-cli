//! Shared serde helpers bridging a behavioral gap between Go's
//! `encoding/json` and serde: Go happily unmarshals an explicit JSON `null`
//! into a bare (non-pointer) slice or string field, producing the zero
//! value. Serde's derived `Deserialize` only applies `#[serde(default)]`
//! when the key is *missing*, and errors on an explicit `null` for a
//! non-`Option` field. Fields ported from such Go fields pair
//! `#[serde(default)]` with `deserialize_with = "null_as_default"` to match
//! Go's tolerance for both "missing" and "explicitly null".

use serde::{Deserialize, Deserializer};

pub fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
