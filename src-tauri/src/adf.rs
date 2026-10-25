//! Plain text to Atlassian Document Format (ADF), which Jira Cloud wants for the description, and back.
//! Supports paragraphs (blank line), line breaks and bullet lists ("- ", "* " or "• ").

use serde_json::{json, Value};

/// "- buy milk" -> Some("buy milk"). Anything else -> None.
fn bullet_text(line: &str) -> Option<&str> {
    let line = line.trim_start();
    ["- ", "* ", "• "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn paragraph(lines: &[&str]) -> Value {
    let mut nodes = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            nodes.push(json!({ "type": "hardBreak" }));
        }
        let line = line.trim_end();
        if !line.is_empty() {
            // ADF rejects text nodes with empty text, hence the check.
            nodes.push(json!({ "type": "text", "text": line }));
        }
    }
    json!({ "type": "paragraph", "content": nodes })
}

fn bullet_list(items: &[&str]) -> Value {
    let list_items: Vec<Value> = items
        .iter()
        .map(|item| {
            json!({
                "type": "listItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": item }] }]
            })
        })
        .collect();
    json!({ "type": "bulletList", "content": list_items })
}

pub fn text_to_adf(text: &str) -> Value {
    let normalized = text.replace("\r\n", "\n");
    let mut content: Vec<Value> = Vec::new();

    for block in normalized.split("\n\n").map(str::trim).filter(|b| !b.is_empty()) {
        let lines: Vec<&str> = block.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if bullet_text(lines[i]).is_some() {
                // a run of consecutive bullet lines becomes one list
                let mut items = Vec::new();
                while let Some(item) = lines.get(i).and_then(|l| bullet_text(l)) {
                    items.push(item);
                    i += 1;
                }
                content.push(bullet_list(&items));
            } else {
                let mut run = Vec::new();
                while i < lines.len() && bullet_text(lines[i]).is_none() {
                    run.push(lines[i]);
                    i += 1;
                }
                content.push(paragraph(&run));
            }
        }
    }

    json!({ "type": "doc", "version": 1, "content": content })
}

/// The reverse of `text_to_adf`: turn a Jira rich-text document (a comment, a description) back
/// into plain text a person (or the local AI) can read. Unknown node types are walked through, so
/// new kinds of content don't make text disappear; unreadable leftovers (images, media) are skipped.
pub fn adf_to_text(doc: &Value) -> String {
    let mut out = String::new();
    write_node(doc, &mut out, 0);
    // tidy: no more than one blank line in a row, no trailing space
    let mut tidy = String::new();
    let mut blank_run = 0;
    for line in out.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        tidy.push_str(line);
        tidy.push('\n');
    }
    tidy.trim().to_string()
}

fn write_children(node: &Value, out: &mut String, depth: usize) {
    if let Some(children) = node.get("content").and_then(Value::as_array) {
        for child in children {
            write_node(child, out, depth);
        }
    }
}

fn write_node(node: &Value, out: &mut String, depth: usize) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" => out.push_str(node.get("text").and_then(Value::as_str).unwrap_or("")),
        "hardBreak" => out.push('\n'),
        "mention" => out.push_str(node.pointer("/attrs/text").and_then(Value::as_str).unwrap_or("@someone")),
        "emoji" => out.push_str(node.pointer("/attrs/text").or_else(|| node.pointer("/attrs/shortName")).and_then(Value::as_str).unwrap_or("")),
        "inlineCard" => out.push_str(node.pointer("/attrs/url").and_then(Value::as_str).unwrap_or("")),
        "listItem" => {
            out.push_str(&format!("{}- ", "  ".repeat(depth.saturating_sub(1))));
            for child in node.get("content").and_then(Value::as_array).into_iter().flatten() {
                if child.get("type").and_then(Value::as_str) == Some("paragraph") {
                    write_children(child, out, depth); // a paragraph inside a bullet is just its text
                    out.push('\n');
                } else {
                    write_node(child, out, depth); // a nested list, etc.
                }
            }
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        "bulletList" | "orderedList" => {
            write_children(node, out, depth + 1);
            out.push('\n');
        }
        "paragraph" | "heading" | "blockquote" | "codeBlock" | "panel" => {
            write_children(node, out, depth);
            out.push_str("\n\n");
        }
        "media" | "mediaSingle" | "mediaGroup" => {} // images and attachments: nothing readable
        _ => write_children(node, out, depth), // doc, table cells, anything new: keep its text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_text_comes_back_as_readable_plain_text() {
        let doc = json!({ "type": "doc", "version": 1, "content": [
            { "type": "paragraph", "content": [
                { "type": "text", "text": "Reproduced on " },
                { "type": "text", "text": "staging", "marks": [{ "type": "strong" }] },
                { "type": "hardBreak" },
                { "type": "mention", "attrs": { "id": "u1", "text": "@Grace" } },
                { "type": "text", "text": " please check." }
            ]},
            { "type": "bulletList", "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Empty invoice crashes" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "Totals are NaN" }] }] }
            ]},
            { "type": "mediaSingle", "content": [{ "type": "media", "attrs": { "id": "x" } }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "Thanks" }] }
        ]});
        assert_eq!(
            adf_to_text(&doc),
            "Reproduced on staging\n@Grace please check.\n\n- Empty invoice crashes\n- Totals are NaN\n\nThanks"
        );
    }

    #[test]
    fn what_we_write_can_be_read_back() {
        let original = "First paragraph.\n\nAcceptance criteria:\n- one\n- two";
        // Same words in the same order; a list always starts on its own line block.
        assert_eq!(adf_to_text(&text_to_adf(original)), "First paragraph.\n\nAcceptance criteria:\n\n- one\n- two");
    }

    #[test]
    fn missing_or_odd_input_gives_empty_text_not_a_crash() {
        assert_eq!(adf_to_text(&Value::Null), "");
        assert_eq!(adf_to_text(&json!({ "type": "doc", "content": [] })), "");
        assert_eq!(adf_to_text(&json!("plain string")), "");
    }

    #[test]
    fn paragraphs_and_line_breaks() {
        let doc = text_to_adf("First line\nsecond line\n\nNew paragraph");
        assert_eq!(doc["type"], "doc");
        assert_eq!(doc["version"], 1);
        let paragraphs = doc["content"].as_array().unwrap();
        assert_eq!(paragraphs.len(), 2);
        let first = paragraphs[0]["content"].as_array().unwrap();
        assert_eq!(first[0]["text"], "First line");
        assert_eq!(first[1]["type"], "hardBreak");
        assert_eq!(first[2]["text"], "second line");
        assert_eq!(paragraphs[1]["content"][0]["text"], "New paragraph");
    }

    #[test]
    fn windows_line_endings_and_blank_input() {
        let doc = text_to_adf("a\r\n\r\nb");
        assert_eq!(doc["content"].as_array().unwrap().len(), 2);
        assert!(text_to_adf("  \n\n ")["content"].as_array().unwrap().is_empty());
    }

    #[test]
    fn never_emits_empty_text_nodes() {
        let doc = text_to_adf("x\n   \ny");
        for node in doc["content"][0]["content"].as_array().unwrap() {
            if node["type"] == "text" {
                assert!(!node["text"].as_str().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn bullet_lines_become_a_real_list() {
        let doc = text_to_adf("Acceptance criteria:\n- Login works\n* Errors are shown\n• Tests pass\nDone.");
        let nodes = doc["content"].as_array().unwrap();
        // heading paragraph, the list, a closing paragraph
        assert_eq!(nodes.iter().map(|n| n["type"].as_str().unwrap()).collect::<Vec<_>>(), ["paragraph", "bulletList", "paragraph"]);
        let items = nodes[1]["content"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["type"], "listItem");
        assert_eq!(items[0]["content"][0]["content"][0]["text"], "Login works");
        assert_eq!(items[2]["content"][0]["content"][0]["text"], "Tests pass");
    }

    #[test]
    fn a_dash_without_text_or_space_is_plain_text() {
        let doc = text_to_adf("-\n-5 degrees\n- ");
        let nodes = doc["content"].as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["type"], "paragraph");
    }
}
