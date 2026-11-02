//! The JSON Schema and the Rust types describe the same vocabulary.

mod schema;

use notchd_proto::version::{CANVAS_COMMANDS, PRIMITIVES};
use notchd_proto::{Slot, TextSize, Token, Tone};
use serde::Serialize;
use serde_json::{Value, json};

fn names<T: Serialize>(all: &[T]) -> Vec<Value> {
    all.iter()
        .map(|v| serde_json::to_value(v).unwrap())
        .collect()
}

fn one_of_consts(schema: &Value, def: &str) -> Vec<String> {
    schema["$defs"][def]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let name = r["$ref"]
                .as_str()
                .unwrap()
                .strip_prefix("#/$defs/")
                .unwrap();
            let props = &schema["$defs"][name]["properties"];
            props["t"]["const"]
                .as_str()
                .or_else(|| props["c"]["const"].as_str())
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn the_schema_lists_every_primitive_and_command() {
    let schema = schema::load();
    assert_eq!(one_of_consts(&schema, "Node"), PRIMITIVES);
    assert_eq!(one_of_consts(&schema, "Cmd"), CANVAS_COMMANDS);
}

#[test]
fn the_schema_enums_match_the_types() {
    let schema = schema::load();
    assert_eq!(schema["$defs"]["Tone"]["enum"], json!(names(&Tone::ALL)));
    assert_eq!(schema["$defs"]["Token"]["enum"], json!(names(&Token::ALL)));
    assert_eq!(
        schema["$defs"]["TextSize"]["enum"],
        json!(names(&TextSize::ALL))
    );
}

#[test]
fn slots_are_the_five_of_notch_module() {
    assert_eq!(
        names(&Slot::ALL),
        [
            "compact",
            "verticalCompact",
            "peek",
            "verticalPeek",
            "expanded"
        ]
    );
}
