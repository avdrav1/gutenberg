//! Window settings (S11.1): what a drag or a key changes and the next
//! launch reads back.

use gutenberg_store::Store;

#[test]
fn a_setting_is_none_until_saved_then_reads_back() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.setting("layout.widths").unwrap(), None);
    store.set_setting("layout.widths", "0.2,0.3,0.5").unwrap();
    assert_eq!(
        store.setting("layout.widths").unwrap().as_deref(),
        Some("0.2,0.3,0.5")
    );
}

#[test]
fn saving_again_replaces_the_value() {
    let store = Store::open_in_memory().unwrap();
    store.set_setting("layout.widths", "0.2,0.3,0.5").unwrap();
    store.set_setting("layout.widths", "0.1,0.4,0.5").unwrap();
    assert_eq!(
        store.setting("layout.widths").unwrap().as_deref(),
        Some("0.1,0.4,0.5")
    );
}

#[test]
fn unsetting_forgets_it_and_is_fine_when_there_was_nothing() {
    let store = Store::open_in_memory().unwrap();
    store.unset_setting("layout.widths").unwrap();
    store.set_setting("layout.widths", "0.2,0.3,0.5").unwrap();
    store.unset_setting("layout.widths").unwrap();
    assert_eq!(store.setting("layout.widths").unwrap(), None);
}

#[test]
fn settings_survive_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    Store::open(&path)
        .unwrap()
        .set_setting("layout.widths", "0.2,0.3,0.5")
        .unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.setting("layout.widths").unwrap().as_deref(),
        Some("0.2,0.3,0.5")
    );
}
