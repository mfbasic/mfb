use tinyjson::JsonValue;

const USAGE: &str = "Usage: mfb <command>\n\nCommands:\n  help    Show this message";

fn main() {
    let _json_lib_marker: JsonValue = JsonValue::Object(Default::default());
    println!("{USAGE}");
}
