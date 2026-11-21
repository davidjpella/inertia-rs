//! Big-integer preservation (`preserveBigIntegers`).
//!
//! JavaScript parses integers outside `±(2^53 - 1)` lossily. When enabled,
//! such integers are wrapped as `{"$bigint": "<digits>"}` and the client turns
//! the marker back into a `BigInt`.

use serde_json::{Map, Value};

/// Marker key used for big integers.
pub const BIG_INTEGER_KEY: &str = "$bigint";

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn is_unsafe(number: &serde_json::Number) -> bool {
    if let Some(value) = number.as_u64() {
        return value > MAX_SAFE_INTEGER;
    }

    if let Some(value) = number.as_i64() {
        return value.unsigned_abs() > MAX_SAFE_INTEGER;
    }

    false
}

/// Wraps integers outside JavaScript's safe range in `$bigint` markers,
/// recursively and in place. Floats are left untouched.
pub fn encode_big_integers(value: &mut Value) {
    match value {
        Value::Number(number) if is_unsafe(number) => {
            let mut marker = Map::new();
            marker.insert(BIG_INTEGER_KEY.into(), Value::String(number.to_string()));
            *value = Value::Object(marker);
        }
        Value::Array(items) => items.iter_mut().for_each(encode_big_integers),
        Value::Object(map) => map.values_mut().for_each(encode_big_integers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wraps_only_unsafe_integers() {
        let mut value = json!({
            "safe": 9_007_199_254_740_991u64,
            "big": 9_007_199_254_740_992u64,
            "negative": -9_007_199_254_740_992i64,
            "float": 1.5e300,
            "nested": [{ "id": 900_719_925_474_099_988u64 }],
        });

        encode_big_integers(&mut value);

        assert_eq!(
            value,
            json!({
                "safe": 9_007_199_254_740_991u64,
                "big": { "$bigint": "9007199254740992" },
                "negative": { "$bigint": "-9007199254740992" },
                "float": 1.5e300,
                "nested": [{ "id": { "$bigint": "900719925474099988" } }],
            })
        );
    }
}
