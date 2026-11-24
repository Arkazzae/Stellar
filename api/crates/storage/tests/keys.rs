//! What an object key may look like.

use stellarr_storage::key::{INCOMING_FOLDER, MAX_KEY_BYTES, is_safe, is_valid};

#[test]
fn only_keys_that_stay_inside_the_folder_they_are_joined_to_are_safe() {
    for (key, safe) in [
        ("attachments/a/b.png", true),
        ("embeds/2026/11/0f1e.jpg", true),
        ("x", true),
        (".hidden", true),
        ("...", true),
        ("a/.../b", true),
        ("a/..b", true),
        ("a/b..", true),
        ("..a", true),
        ("żółć/ü", true),
        ("a b/c", true),
        ("", false),
        ("../escape", false),
        ("a/../../b", false),
        ("a/..", false),
        ("..", false),
        (".", false),
        ("a/./b", false),
        ("./a", false),
        ("a/.", false),
        ("/absolute", false),
        ("/", false),
        ("a//b", false),
        ("a/", false),
        ("//", false),
        ("a\\b", false),
        ("\\", false),
        ("a\0b", false),
        ("\0", false),
        ("a/\0", false),
    ] {
        assert_eq!(is_safe(key), safe, "{key:?}");
    }
}

#[test]
fn the_stores_also_refuse_the_staging_folder_and_keys_past_the_longest() {
    assert_eq!(INCOMING_FOLDER, ".incoming");
    assert!(!is_valid(".incoming"));
    assert!(!is_valid(".incoming/x"));
    assert!(!is_valid(".incoming/a/b"));
    assert!(is_valid(".incomingx"));
    assert!(is_valid(".incoming.bak/x"));
    assert!(is_valid("a/.incoming"));
    assert!(is_valid("a/.incoming/b"));

    assert_eq!(MAX_KEY_BYTES, 1024);
    assert!(is_valid(&"k".repeat(1024)));
    assert!(!is_valid(&"k".repeat(1025)));
    // The limit is in bytes, not characters.
    assert!(is_valid(&"é".repeat(512)));
    assert!(!is_valid(&"é".repeat(513)));
}
