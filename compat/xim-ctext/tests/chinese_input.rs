use xim_ctext::{compound_text_to_utf8, utf8_to_compound_text};

#[test]
fn gb2312_commit_decodes_chinese() {
    assert_eq!(compound_text_to_utf8(b"\x1b$(AVPND").unwrap(), "中文");
    assert_eq!(compound_text_to_utf8(b"\x1b$(ADc:C").unwrap(), "你好");
}

#[test]
fn chinese_and_ascii_can_alternate_in_one_commit() {
    assert_eq!(
        compound_text_to_utf8(b"log: \x1b$(AVPND\x1b(B 123\x1b$(ADc:C").unwrap(),
        "log: 中文 123你好"
    );
    // A date phrase with ASCII digits between GB2312 characters, as produced
    // by Sogou's date completion. Charset reset sequences must not leak out.
    assert_eq!(
        compound_text_to_utf8(b"2026\x1b$(ADj\x1b(B07\x1b$(ATB\x1b(B16\x1b$(AHU").unwrap(),
        "2026年07月16日"
    );
}

#[test]
fn long_chinese_commit_is_not_truncated_or_repeated() {
    let mut bytes = b"\x1b$(A".to_vec();
    bytes.extend_from_slice(&b"VPND".repeat(4096));
    assert_eq!(compound_text_to_utf8(&bytes).unwrap(), "中文".repeat(4096));
}

#[test]
fn utf8_compositions_remain_supported() {
    for text in ["", "ASCII", "中文🎉", "東京", "가나다"] {
        assert_eq!(
            compound_text_to_utf8(&utf8_to_compound_text(text)).unwrap(),
            text
        );
    }
}

#[test]
fn legacy_japanese_commit_remains_supported() {
    assert_eq!(compound_text_to_utf8(b"\x1b$(BEl5~").unwrap(), "東京");
}
