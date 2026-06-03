//! Structural extraction: parse must surface the fields surgery reasons about,
//! without disturbing byte-for-byte round-trip (covered in roundtrip.rs).

use eigen_surgery::Session;

const SID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

fn fixture() -> String {
    [
        format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SID}"}}"#),
        format!(r#"{{"type":"file-history-snapshot","messageId":"m1","snapshot":{{}}}}"#),
        format!(r#"{{"type":"user","uuid":"00000000-0000-4000-8000-0000000000u1","parentUuid":null,"isSidechain":false,"sessionId":"{SID}","message":{{"role":"user","content":"hi"}}}}"#),
    ]
    .join("\n")
        + "\n"
}

#[test]
fn parse_extracts_the_session_id() {
    let session = Session::parse_str(&fixture()).expect("parse");
    assert_eq!(session.session_id, SID);
}
