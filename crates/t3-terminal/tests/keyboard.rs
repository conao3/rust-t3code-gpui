use serde_json::Value;
use t3_terminal::KeyboardEvent;

#[test]
fn matches_original_keyboard_conversion() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/original-keyboard.json")).unwrap();
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input: KeyboardEvent = serde_json::from_value(case["input"].clone()).unwrap();
        let actual = serde_json::to_value(input.encoded()).unwrap();
        assert_eq!(actual, case["expected"], "case {index}: {input:?}");
    }
}
