//! The boundary itself: schemas that deserialise, names that do not, labels that count from one.

use super::*;

#[test]
fn every_tool_definition_deserialises_as_a_call() {
    // The schema list and the `ToolCall` enum are two descriptions of the same
    // interface. If a name is added to one and not the other, the model gets a tool
    // it cannot use and the failure only shows up mid-conversation, at runtime.
    let definitions = tool_definitions(&context());
    assert_eq!(definitions.len(), 16, "the spec lists sixteen tools");

    for definition in definitions {
        let name = definition["name"].as_str().unwrap().to_string();
        let schema = &definition["input_schema"];

        // Build a minimal input from the schema's required fields.
        let mut input = serde_json::Map::new();
        for required in schema["required"].as_array().unwrap() {
            let field = required.as_str().unwrap();
            let kind = schema["properties"][field]["type"].as_str().unwrap();
            input.insert(
                field.to_string(),
                match kind {
                    "integer" | "number" => json!(1),
                    "boolean" => json!(true),
                    "array" => json!([{"pitch": 60, "start_ticks": 0, "duration_ticks": 120}]),
                    _ => json!("1/8"),
                },
            );
        }

        let value = json!({"name": name, "input": input});
        assert!(
            serde_json::from_value::<ToolCall>(value.clone()).is_ok(),
            "tool {name} has no matching ToolCall variant: {value}"
        );
    }
}

#[test]
fn an_unknown_tool_is_refused_at_the_boundary() {
    let value = json!({"name": "delete_the_project", "input": {}});
    assert!(serde_json::from_value::<ToolCall>(value).is_err());
}

#[test]
fn division_parsing() {
    let context = context();
    assert_eq!(context.division_ticks("1/4").unwrap(), 480);
    assert_eq!(context.division_ticks("1/8").unwrap(), 240);
    assert_eq!(context.division_ticks("1/16").unwrap(), 120);
    assert_eq!(context.division_ticks("1/8t").unwrap(), 160, "triplet eighth");
    assert_eq!(context.division_ticks("1/8.").unwrap(), 360, "dotted eighth");
    assert_eq!(context.division_ticks("8").unwrap(), 8, "bare ticks");
    assert!(context.division_ticks("1/5").is_err());
    assert!(context.division_ticks("nonsense").is_err());
    assert!(context.division_ticks("0").is_err());
}

#[test]
fn bar_and_beat_labels_count_from_one() {
    let context = context();
    assert_eq!(context.position_label(0), "1.1");
    assert_eq!(context.position_label(480), "1.2");
    assert_eq!(context.position_label(1920), "2.1");
}

