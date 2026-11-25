//! A stack's NBT as JSON with its types kept, so the client can rebuild the exact stack (its
//! enchantment glint, durability bar, tooltip) from the view: `{"b":1}` a byte, `{"s":…}` a short,
//! `{"i":…}` an int, `{"l":…}` a long, `{"f":bits}` and `{"d":bits}` floats by their bits,
//! `{"t":"…"}` a string, `{"B":[…]}`, `{"I":[…]}`, `{"L":[…]}` arrays, `{"[":[…]}` a list and
//! `{"{":{…}}` a compound. Strings are UTF-16 in NBT; JSON carries them as Rust strings (lossy
//! only for unpaired surrogates, which item data never holds).
use mod_api::{Compound, Tag};
use serde_json::{Map, Value, json};

pub fn to_json(tag: &Tag) -> Value {
    match tag {
        Tag::Byte(v) => json!({ "b": v }),
        Tag::Short(v) => json!({ "s": v }),
        Tag::Int(v) => json!({ "i": v }),
        Tag::Long(v) => json!({ "l": v }),
        Tag::Float(v) => json!({ "f": v }),
        Tag::Double(v) => json!({ "d": v }),
        Tag::ByteArray(v) => json!({ "B": v }),
        Tag::String(v) => json!({ "t": v.to_string_lossy() }),
        Tag::List(v) => json!({ "[": v.iter().map(to_json).collect::<Vec<_>>() }),
        Tag::Compound(v) => {
            let map: Map<String, Value> = v.iter().map(|(k, t)| (k.to_string_lossy(), to_json(t))).collect();
            json!({ "{": map })
        }
        Tag::IntArray(v) => json!({ "I": v }),
        Tag::LongArray(v) => json!({ "L": v }),
        #[allow(unreachable_patterns)]
        _ => Value::Null,
    }
}

pub fn from_json(value: &Value) -> Option<Tag> {
    let (kind, v) = value.as_object()?.iter().next()?;
    Some(match kind.as_str() {
        "b" => Tag::Byte(i8::try_from(v.as_i64()?).ok()?),
        "s" => Tag::Short(i16::try_from(v.as_i64()?).ok()?),
        "i" => Tag::Int(i32::try_from(v.as_i64()?).ok()?),
        "l" => Tag::Long(v.as_i64()?),
        "f" => Tag::Float(u32::try_from(v.as_u64()?).ok()?),
        "d" => Tag::Double(v.as_u64()?),
        "B" => Tag::ByteArray(v.as_array()?.iter().map(|x| x.as_u64().and_then(|n| u8::try_from(n).ok())).collect::<Option<_>>()?),
        "t" => Tag::String(v.as_str()?.into()),
        "[" => Tag::List(v.as_array()?.iter().map(from_json).collect::<Option<_>>()?),
        "{" => Tag::Compound(compound(v)?),
        "I" => Tag::IntArray(v.as_array()?.iter().map(|x| x.as_i64().and_then(|n| i32::try_from(n).ok())).collect::<Option<_>>()?),
        "L" => Tag::LongArray(v.as_array()?.iter().map(Value::as_i64).collect::<Option<_>>()?),
        _ => return None,
    })
}

fn compound(v: &Value) -> Option<Compound> {
    v.as_object()?.iter().map(|(k, t)| Some((k.as_str().into(), from_json(t)?))).collect()
}

/// A compound as one JSON string, and back.
pub fn write(c: &Compound) -> String {
    to_json(&Tag::Compound(c.clone())).to_string()
}

pub fn read(text: &str) -> Option<Compound> {
    match from_json(&serde_json::from_str(text).ok()?)? {
        Tag::Compound(c) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_tag_round_trips() {
        let inner: Compound = [("level".into(), Tag::Int(3)), ("name".into(), Tag::String("حدّة".into()))].into_iter().collect();
        let c: Compound = [
            ("b".into(), Tag::Byte(-3)),
            ("s".into(), Tag::Short(300)),
            ("l".into(), Tag::Long(-9_000_000_000)),
            ("f".into(), Tag::Float(1.5f32.to_bits())),
            ("d".into(), Tag::Double(2.25f64.to_bits())),
            ("ba".into(), Tag::ByteArray(vec![0, 255])),
            ("ia".into(), Tag::IntArray(vec![-1, 7])),
            ("la".into(), Tag::LongArray(vec![1, -1])),
            ("list".into(), Tag::List(vec![Tag::Compound(inner.clone()), Tag::Compound(inner)])),
        ]
        .into_iter()
        .collect();
        assert_eq!(read(&write(&c)), Some(c));
    }

    #[test]
    fn broken_text_reads_as_nothing() {
        assert_eq!(read("not json"), None);
        assert_eq!(read(r#"{"i":1}"#), None);
        assert_eq!(read(r#"{"{":{"x":{"b":999}}}"#), None);
    }
}
