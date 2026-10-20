//! Text unescaped from a page (`html::unescape`): an entity name is ASCII, so a
//! character of any other script after an `&` ends the name and never splits.

use bagholder_sources::html::unescape;

#[test]
fn a_character_of_another_script_after_an_ampersand_is_kept_whole() {
    assert_eq!(unescape("R&D 中文公告"), "R&D 中文公告");
    assert_eq!(unescape("&中文"), "&中文");
    assert_eq!(unescape("Société &amp; Générale"), "Société & Générale");
    assert_eq!(unescape("&ampé"), "&é");
    // a character of two, three and four bytes at every place a name can reach, its 32-byte limit included
    for c in ["é", "中", "😀"] {
        for pad in 0..40 {
            let t = format!("&{}{}{}", "a".repeat(pad), c, c);
            assert_eq!(unescape(&t), t, "{t}");
        }
    }
}
