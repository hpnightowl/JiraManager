//! Jira "required fields". Every project and issue type can demand different fields, so we ask
//! Jira (the createmeta API) which fields exist, turn each into a `FieldSpec` the UI can draw
//! as a form control, and later turn the user's answers back into the JSON Jira expects.
//!
//! Pure functions only (no HTTP, no database), so it is easy to test.

use crate::adf::text_to_adf;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Fields the app fills in itself (or Jira fills in), so we never ask the user for them.
const AUTO_FIELDS: &[&str] = &["project", "issuetype", "summary", "description", "reporter", "attachment", "issuelinks"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AllowedValue {
    pub id: String,
    pub label: String,
}

/// How the UI should draw the field, and how its answer is converted for Jira.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    TextArea,
    Number,
    Date,
    /// Pick one of `allowed`.
    Option,
    /// Pick several of `allowed` (components, versions...).
    MultiOption,
    /// Free words, e.g. labels.
    Labels,
    /// Search for a person.
    User,
    /// A kind we don't handle yet; pushing is blocked with a clear message.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldSpec {
    pub id: String,
    pub name: String,
    pub required: bool,
    pub has_default: bool,
    pub kind: FieldKind,
    pub allowed: Vec<AllowedValue>,
    pub note: Option<String>,
}

// --- Jira's JSON (only the parts we use) ---------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RawField {
    #[serde(default)]
    required: bool,
    #[serde(default)]
    name: String,
    key: Option<String>,
    #[serde(rename = "fieldId")]
    field_id: Option<String>,
    schema: Option<RawSchema>,
    #[serde(rename = "hasDefaultValue", default)]
    has_default_value: bool,
    #[serde(rename = "allowedValues", default)]
    allowed_values: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct RawSchema {
    #[serde(rename = "type", default)]
    kind: String,
    items: Option<String>,
    system: Option<String>,
    custom: Option<String>,
}

fn string_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn allowed_values(raw: &[Value]) -> Vec<AllowedValue> {
    raw.iter()
        .filter_map(|v| {
            let id = v.get("id").and_then(string_of)?;
            let label = ["name", "value", "title"]
                .iter()
                .find_map(|k| v.get(*k).and_then(Value::as_str))
                .unwrap_or(&id)
                .to_string();
            Some(AllowedValue { id, label })
        })
        .collect()
}

fn classify(schema: &RawSchema, has_choices: bool) -> (FieldKind, Option<String>) {
    let unsupported = |why: String| (FieldKind::Unsupported, Some(why));
    let needs_choices = |kind: FieldKind| {
        if has_choices {
            (kind, None)
        } else {
            unsupported("Jira offered no choices for this field".to_string())
        }
    };

    match (schema.kind.as_str(), schema.items.as_deref()) {
        ("string", _) => {
            let rich = matches!(schema.system.as_deref(), Some("description" | "environment"))
                || schema.custom.as_deref().is_some_and(|c| c.ends_with(":textarea"));
            (if rich { FieldKind::TextArea } else { FieldKind::Text }, None)
        }
        ("number", _) => (FieldKind::Number, None),
        ("date", _) => (FieldKind::Date, None),
        ("user", _) => (FieldKind::User, None),
        ("option" | "priority" | "securitylevel", _) => needs_choices(FieldKind::Option),
        ("array", Some("string")) => (FieldKind::Labels, None),
        ("array", Some("option" | "component" | "version")) => needs_choices(FieldKind::MultiOption),
        (other, items) => unsupported(format!(
            "field type '{}{}' is not supported yet",
            other,
            items.map(|i| format!(" of {i}")).unwrap_or_default()
        )),
    }
}

pub fn spec_from_raw(raw: RawField) -> Option<FieldSpec> {
    let id = raw.field_id.or(raw.key)?;
    let allowed = allowed_values(&raw.allowed_values);
    let (kind, note) = match &raw.schema {
        Some(schema) => classify(schema, !allowed.is_empty()),
        None => (FieldKind::Unsupported, Some("Jira gave no type information".to_string())),
    };
    Some(FieldSpec {
        id,
        name: raw.name,
        required: raw.required,
        has_default: raw.has_default_value,
        kind,
        allowed,
        note,
    })
}

/// The fields the user has to fill in: required, no default Jira can apply, not auto-filled.
pub fn required_fields(specs: &[FieldSpec]) -> Vec<&FieldSpec> {
    specs
        .iter()
        .filter(|f| f.required && !f.has_default && !AUTO_FIELDS.contains(&f.id.as_str()))
        .collect()
}

/// Commonly wanted fields that Jira does NOT require. We show them (when the project's create
/// screen has them) so the issue isn't created half-empty: no assignee, no priority, no labels...
const OPTIONAL_FIELDS: &[&str] = &["assignee", "priority", "labels", "components", "fixVersions", "duedate"];

/// The optional fields worth offering: on the create screen, of a kind we can fill in, and not
/// already asked as required.
pub fn optional_fields(specs: &[FieldSpec]) -> Vec<&FieldSpec> {
    let required: Vec<&str> = required_fields(specs).iter().map(|f| f.id.as_str()).collect();
    specs
        .iter()
        .filter(|f| OPTIONAL_FIELDS.contains(&f.id.as_str()) && f.kind != FieldKind::Unsupported)
        .filter(|f| !required.contains(&f.id.as_str()))
        .collect()
}

/// Does this issue type's create screen have a field with that id? (For example "parent".)
pub fn has_field(specs: &[FieldSpec], id: &str) -> bool {
    specs.iter().any(|f| f.id == id)
}

/// Is the description itself mandatory for this issue type?
pub fn description_required(specs: &[FieldSpec]) -> bool {
    specs.iter().any(|f| f.id == "description" && f.required && !f.has_default)
}

// --- answers -> Jira JSON --------------------------------------------------------------------

fn as_text(input: &Value) -> String {
    string_of(input).unwrap_or_default().trim().to_string()
}

fn as_list(input: &Value) -> Vec<String> {
    match input {
        Value::Array(items) => items.iter().filter_map(string_of).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        other => as_text(other)
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
    }
}

fn is_blank(input: &Value) -> bool {
    match input {
        Value::Null => true,
        Value::Array(items) => items.iter().all(is_blank),
        other => as_text(other).is_empty(),
    }
}

fn check_allowed(spec: &FieldSpec, id: &str) -> Result<(), String> {
    if spec.allowed.iter().any(|a| a.id == id) {
        Ok(())
    } else {
        Err(format!("\"{id}\" is not a valid choice for {}", spec.name))
    }
}

/// Convert one answer into the exact JSON shape Jira expects for that kind of field.
pub fn build_value(spec: &FieldSpec, input: &Value) -> Result<Value, String> {
    let text = as_text(input);
    match spec.kind {
        FieldKind::Text => Ok(json!(text)),
        FieldKind::TextArea => Ok(text_to_adf(&text)),
        FieldKind::Number => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| json!(n))
            .ok_or_else(|| format!("{} must be a number", spec.name)),
        FieldKind::Date => NaiveDate::parse_from_str(&text, "%Y-%m-%d")
            .map(|_| json!(text))
            .map_err(|_| format!("{} must be a date like 2026-10-31", spec.name)),
        FieldKind::Option => {
            check_allowed(spec, &text)?;
            Ok(json!({ "id": text }))
        }
        FieldKind::MultiOption => {
            let ids = as_list(input);
            for id in &ids {
                check_allowed(spec, id)?;
            }
            Ok(Value::Array(ids.iter().map(|id| json!({ "id": id })).collect()))
        }
        FieldKind::Labels => Ok(json!(as_list(input))),
        FieldKind::User => Ok(json!({ "accountId": text })),
        FieldKind::Unsupported => Err(format!(
            "\"{}\" is required but ProjectJira can't fill it in yet ({}). Create the issue in Jira instead.",
            spec.name,
            spec.note.as_deref().unwrap_or("unsupported field")
        )),
    }
}

/// Build the `fields` entries for every required field, or explain everything that is wrong
/// in one message (so the user fixes it all at once).
pub fn build_fields_payload(specs: &[FieldSpec], answers: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let (mut missing, mut problems) = (Vec::new(), Vec::new());

    for spec in required_fields(specs) {
        if spec.kind == FieldKind::Unsupported {
            problems.push(build_value(spec, &Value::Null).unwrap_err());
            continue;
        }
        match answers.get(&spec.id) {
            Some(input) if !is_blank(input) => match build_value(spec, input) {
                Ok(value) => {
                    out.insert(spec.id.clone(), value);
                }
                Err(e) => problems.push(e),
            },
            _ => missing.push(spec.name.clone()),
        }
    }

    // Optional fields: only sent when the user actually filled them in.
    for spec in optional_fields(specs) {
        if let Some(input) = answers.get(&spec.id).filter(|v| !is_blank(v)) {
            match build_value(spec, input) {
                Ok(value) => {
                    out.insert(spec.id.clone(), value);
                }
                Err(e) => problems.push(e),
            }
        }
    }

    if missing.is_empty() && problems.is_empty() {
        return Ok(out);
    }
    if !missing.is_empty() {
        problems.insert(0, format!("Please fill in: {}", missing.join(", ")));
    }
    Err(problems.join(". "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like Jira's createmeta response for one issue type.
    fn fixture() -> Vec<FieldSpec> {
        let fields: Value = json!([
            { "required": true,  "name": "Summary", "fieldId": "summary", "schema": {"type":"string","system":"summary"}, "hasDefaultValue": false },
            { "required": true,  "name": "Issue Type", "fieldId": "issuetype", "schema": {"type":"issuetype","system":"issuetype"}, "allowedValues": [{"id":"10001","name":"Story"}] },
            { "required": false, "name": "Description", "fieldId": "description", "schema": {"type":"string","system":"description"} },
            { "required": true,  "name": "Priority", "fieldId": "priority", "schema": {"type":"priority","system":"priority"}, "hasDefaultValue": true,
              "allowedValues": [{"id":"3","name":"Medium"}] },
            { "required": true,  "name": "Team", "fieldId": "customfield_10050", "schema": {"type":"option","custom":"com.atlassian.jira.plugin.system.customfieldtypes:select","customId":10050},
              "allowedValues": [{"id":"201","value":"Platform"},{"id":"202","value":"Mobile"}] },
            { "required": true,  "name": "Components", "fieldId": "components", "schema": {"type":"array","items":"component","system":"components"},
              "allowedValues": [{"id":"11","name":"API"},{"id":"12","name":"UI"}] },
            { "required": true,  "name": "Story Points", "fieldId": "customfield_10016", "schema": {"type":"number","custom":"com.atlassian.jira.plugin.system.customfieldtypes:float"} },
            { "required": true,  "name": "Acceptance Criteria", "fieldId": "customfield_10060", "schema": {"type":"string","custom":"com.atlassian.jira.plugin.system.customfieldtypes:textarea"} },
            { "required": true,  "name": "Due date", "fieldId": "duedate", "schema": {"type":"date","system":"duedate"} },
            { "required": true,  "name": "Labels", "fieldId": "labels", "schema": {"type":"array","items":"string","system":"labels"} },
            { "required": true,  "name": "Approver", "fieldId": "customfield_10070", "schema": {"type":"user","custom":"x:userpicker"} },
            { "required": true,  "name": "Weird", "fieldId": "customfield_10099", "schema": {"type":"option-with-child","custom":"x:cascadingselect"}, "allowedValues": [{"id":"1","value":"a"}] },
            { "required": false, "name": "Optional text", "fieldId": "customfield_10080", "schema": {"type":"string"} }
        ]);
        serde_json::from_value::<Vec<RawField>>(fields)
            .unwrap()
            .into_iter()
            .filter_map(spec_from_raw)
            .collect()
    }

    fn by_id<'a>(specs: &'a [FieldSpec], id: &str) -> &'a FieldSpec {
        specs.iter().find(|f| f.id == id).unwrap()
    }

    #[test]
    fn kinds_are_derived_from_jira_schemas() {
        let specs = fixture();
        let kind = |id: &str| by_id(&specs, id).kind;
        assert_eq!(kind("customfield_10050"), FieldKind::Option);
        assert_eq!(kind("components"), FieldKind::MultiOption);
        assert_eq!(kind("customfield_10016"), FieldKind::Number);
        assert_eq!(kind("customfield_10060"), FieldKind::TextArea);
        assert_eq!(kind("duedate"), FieldKind::Date);
        assert_eq!(kind("labels"), FieldKind::Labels);
        assert_eq!(kind("customfield_10070"), FieldKind::User);
        assert_eq!(kind("customfield_10099"), FieldKind::Unsupported);
        assert_eq!(by_id(&specs, "customfield_10050").allowed[0].label, "Platform"); // "value" label
        assert_eq!(by_id(&specs, "components").allowed[1].label, "UI"); // "name" label
    }

    #[test]
    fn only_required_fields_without_defaults_are_asked() {
        let specs = fixture();
        let asked: Vec<&str> = required_fields(&specs).iter().map(|f| f.id.as_str()).collect();
        // Not asked: summary/issuetype/description (auto), priority (has a default), optional text.
        assert!(!asked.contains(&"summary") && !asked.contains(&"priority") && !asked.contains(&"customfield_10080"));
        assert!(asked.contains(&"customfield_10050") && asked.contains(&"duedate"));
        assert!(!description_required(&specs));
    }

    #[test]
    fn answers_become_the_json_jira_expects() {
        let specs = fixture();
        let v = |id: &str, input: Value| build_value(by_id(&specs, id), &input).unwrap();
        assert_eq!(v("customfield_10050", json!("201")), json!({"id":"201"}));
        assert_eq!(v("components", json!(["11", "12"])), json!([{"id":"11"},{"id":"12"}]));
        assert_eq!(v("customfield_10016", json!("3.5")), json!(3.5));
        assert_eq!(v("duedate", json!("2026-10-31")), json!("2026-10-31"));
        assert_eq!(v("labels", json!("backend, urgent api")), json!(["backend", "urgent", "api"]));
        assert_eq!(v("customfield_10070", json!("abc123")), json!({"accountId":"abc123"}));
        assert_eq!(v("customfield_10060", json!("Line one"))["type"], "doc"); // rich text -> ADF
    }

    #[test]
    fn bad_answers_are_rejected_with_a_reason() {
        let specs = fixture();
        let err = |id: &str, input: Value| build_value(by_id(&specs, id), &input).unwrap_err();
        assert!(err("customfield_10050", json!("999")).contains("not a valid choice"));
        assert!(err("customfield_10016", json!("abc")).contains("must be a number"));
        assert!(err("customfield_10016", json!("NaN")).contains("must be a number"));
        assert!(err("duedate", json!("31/10/2026")).contains("date like"));
        assert!(err("customfield_10099", json!("x")).contains("can't fill it in yet"));
    }

    #[test]
    fn payload_reports_everything_missing_in_one_message() {
        let specs = fixture();
        let mut answers = Map::new();
        answers.insert("customfield_10050".into(), json!("202"));
        let message = build_fields_payload(&specs, &answers).unwrap_err();
        assert!(message.contains("Please fill in:"));
        assert!(message.contains("Components") && message.contains("Due date"));
        assert!(message.contains("Weird")); // unsupported required field also explained
    }

    fn with_optional() -> Vec<FieldSpec> {
        let mut specs = fixture();
        specs.retain(|f| f.id != "customfield_10099");
        let more: Value = json!([
            { "required": false, "name": "Assignee", "fieldId": "assignee", "schema": {"type":"user","system":"assignee"} },
            { "required": false, "name": "Labels", "fieldId": "labels", "schema": {"type":"array","items":"string","system":"labels"} },
            { "required": false, "name": "Parent", "fieldId": "parent", "schema": {"type":"issuelink","system":"parent"} },
            { "required": false, "name": "Fix versions", "fieldId": "fixVersions", "schema": {"type":"array","items":"version","system":"fixVersions"},
              "allowedValues": [{"id":"1","name":"1.0"}] }
        ]);
        specs.extend(serde_json::from_value::<Vec<RawField>>(more).unwrap().into_iter().filter_map(spec_from_raw));
        specs
    }

    #[test]
    fn useful_optional_fields_are_offered_but_required_ones_are_not_repeated() {
        let specs = with_optional();
        let offered: Vec<&str> = optional_fields(&specs).iter().map(|f| f.id.as_str()).collect();
        assert!(offered.contains(&"assignee") && offered.contains(&"fixVersions"));
        assert!(offered.contains(&"priority")); // required-with-default counts as optional to the user
        assert!(!offered.contains(&"parent")); // handled separately
        // "labels" appears twice in the fixture (required in the first, optional in the second);
        // required wins, so it is not offered again.
        assert!(!offered.contains(&"labels"));
        assert!(has_field(&specs, "parent") && !has_field(&specs, "sprint"));
    }

    #[test]
    fn optional_answers_are_sent_only_when_filled() {
        let specs = with_optional();
        let required = |extra: Value| -> Map<String, Value> {
            let mut base: Map<String, Value> = serde_json::from_value(json!({
                "customfield_10050": "201", "components": ["11"], "customfield_10016": "5",
                "customfield_10060": "x", "duedate": "2026-11-01", "labels": "a",
                "customfield_10070": "acc-1"
            }))
            .unwrap();
            base.extend(serde_json::from_value::<Map<String, Value>>(extra).unwrap());
            base
        };

        let without = build_fields_payload(&specs, &required(json!({}))).unwrap();
        assert!(!without.contains_key("assignee") && !without.contains_key("priority"));

        let with = build_fields_payload(&specs, &required(json!({ "assignee": "me-1", "priority": "3", "fixVersions": ["1"] }))).unwrap();
        assert_eq!(with["assignee"], json!({"accountId":"me-1"}));
        assert_eq!(with["priority"], json!({"id":"3"}));
        assert_eq!(with["fixVersions"], json!([{"id":"1"}]));

        // An optional answer that is wrong is reported rather than silently dropped.
        let err = build_fields_payload(&specs, &required(json!({ "priority": "999" }))).unwrap_err();
        assert!(err.contains("not a valid choice"), "got: {err}");
    }

    #[test]
    fn complete_answers_produce_a_payload() {
        let mut specs = fixture();
        specs.retain(|f| f.id != "customfield_10099"); // drop the unsupported one
        let answers: Map<String, Value> = serde_json::from_value(json!({
            "customfield_10050": "201", "components": ["11"], "customfield_10016": "5",
            "customfield_10060": "Given...", "duedate": "2026-11-01", "labels": "a b",
            "customfield_10070": "acc-1"
        }))
        .unwrap();
        let payload = build_fields_payload(&specs, &answers).unwrap();
        assert_eq!(payload.len(), 7);
        assert_eq!(payload["customfield_10050"], json!({"id":"201"}));
    }
}
