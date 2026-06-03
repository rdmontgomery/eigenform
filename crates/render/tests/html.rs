//! session_html: the semantic transcript as HTML — a collapsible <details> per exchange,
//! full (untruncated) content, escaped against injection.

use eigen_render::session_html;
use eigen_surgery::Session;

const SID: &str = "abcd1234-0000-4000-8000-000000000000";

fn two_exchanges() -> Session {
    let text = [
        format!(r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"first prompt"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"a1","parentUuid":"u1","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"the full first reply"}}]}}}}"#),
        format!(r#"{{"type":"system","uuid":"s1","parentUuid":"a1","isSidechain":false,"subtype":"turn_duration","durationMs":1200,"sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"user","uuid":"u2","parentUuid":"s1","isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"second prompt"}}}}"#),
        format!(r#"{{"type":"assistant","uuid":"a2","parentUuid":"u2","isSidechain":false,"sessionId":"{SID}","message":{{"role":"assistant","content":[{{"type":"text","text":"second reply"}}]}}}}"#),
        format!(r#"{{"type":"last-prompt","leafUuid":"a2","sessionId":"{SID}"}}"#),
    ]
    .join("\n")
        + "\n";
    Session::parse_str(&text).unwrap()
}

#[test]
fn one_collapsible_exchange_per_user_turn() {
    let html = session_html(&two_exchanges());
    assert!(html.contains("<details"), "collapsible:\n{html}");
    assert_eq!(html.matches("<summary").count(), 2, "one summary per exchange:\n{html}");
}

#[test]
fn shows_full_untruncated_content() {
    let html = session_html(&two_exchanges());
    assert!(html.contains("first prompt"));
    assert!(html.contains("the full first reply"), "assistant content shown in full");
    assert!(html.contains("second reply"));
}

#[test]
fn escapes_html_in_content() {
    let evil = r#"<script>alert('x')</script>"#;
    let text = format!(
        r#"{{"type":"user","uuid":"u1","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":{}}}}}"#,
        serde_json::to_string(evil).unwrap()
    ) + "\n";
    let html = session_html(&Session::parse_str(&text).unwrap());
    assert!(!html.contains("<script>alert"), "raw script must not survive:\n{html}");
    assert!(html.contains("&lt;script&gt;"), "content escaped:\n{html}");
}
