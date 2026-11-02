//! A JSON number that prints as an integer when it is whole.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Design pixels, fractions, scales: `8` and `0.5`, never `8.0`. This is the
/// canonical form the round-trip fixtures compare against.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Num(pub f64);

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = self.0;
        if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.0e15 {
            s.serialize_i64(v as i64)
        } else {
            s.serialize_f64(v)
        }
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = f64::deserialize(d)?;
        if v.is_finite() {
            Ok(Num(v))
        } else {
            Err(serde::de::Error::custom("number is not finite"))
        }
    }
}

impl From<f64> for Num {
    fn from(v: f64) -> Self {
        Num(v)
    }
}

impl From<i32> for Num {
    fn from(v: i32) -> Self {
        Num(f64::from(v))
    }
}
