//! S10.2: what an update to a breaking story's pop-up replaces, and how it
//! is recorded. S10.5: the history of every notification.

use gutenberg_store::{Fate, NewArticle, NewSource, Notice, NoticeState, SourceKind, Store};

fn add(store: &mut Store, guid: &str, at: i64) -> Notice {
    let source = store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: guid.into(),
            url: Some(format!("https://{guid}.example/feed")),
            config: "{}".into(),
        })
        .unwrap()
        .id;
    store
        .insert_articles(&[NewArticle {
            source_id: source,
            guid: guid.into(),
            url: None,
            title: format!("Headline {guid}"),
            author: None,
            summary_html: None,
            published_at: Some(at),
            fetched_at: at,
        }])
        .unwrap();
    let story = store.touched_stories(at).unwrap().pop().unwrap();
    Notice {
        article_id: story.article_id,
        story_id: story.story_id,
        title: story.title,
        source: story.source,
        reason: "6 outlets in 25 min".into(),
    }
}

#[test]
fn an_update_replaces_a_pop_up_of_its_story_alone_and_keeps_its_place_in_the_hour() {
    let mut store = Store::open_in_memory().unwrap();
    let alone = add(&mut store, "a", 100);
    store
        .record_notices(&[(alone.clone(), NoticeState::Sent)], 100)
        .unwrap();
    store
        .record_popup(&[alone.article_id], 100, Some(7))
        .unwrap();
    assert_eq!(store.story_popup(alone.story_id).unwrap(), Some(7));

    // An update an hour later belongs to pop-up 7, sent at 100.
    let later = Notice {
        article_id: add(&mut store, "b", 3_700).article_id,
        reason: "now 7 outlets".into(),
        ..alone.clone()
    };
    // The desktop answered with a new id, as Omarchy's shell does: the
    // next update replaces that one.
    store.record_update(&later, 3_700, 7, Some(9)).unwrap();
    assert_eq!(store.popups_since(3_000).unwrap(), 0);
    assert_eq!(store.popups_since(0).unwrap(), 1);
    assert_eq!(store.story_popup(alone.story_id).unwrap(), Some(9));

    // Two stories folded into one summary: neither may replace it.
    let (c, d) = (add(&mut store, "c", 200), add(&mut store, "d", 200));
    store
        .record_notices(
            &[
                (c.clone(), NoticeState::Sent),
                (d.clone(), NoticeState::Sent),
            ],
            200,
        )
        .unwrap();
    store
        .record_popup(&[c.article_id, d.article_id], 200, Some(8))
        .unwrap();
    assert_eq!(store.story_popup(c.story_id).unwrap(), None);
    // Nor may a story whose pop-up the desktop gave no id.
    let e = add(&mut store, "e", 300);
    store
        .record_notices(&[(e.clone(), NoticeState::Sent)], 300)
        .unwrap();
    store.record_popup(&[e.article_id], 300, None).unwrap();
    assert_eq!(store.story_popup(e.story_id).unwrap(), None);
}

/// S10.5: every row, newest first, with its reason and what became of it.
#[test]
fn the_history_lists_every_notification_with_its_reason_and_fate() {
    let mut store = Store::open_in_memory().unwrap();
    let with = |notice: Notice, reason: &str| Notice {
        reason: reason.into(),
        ..notice
    };
    // 100: a rule's pop-up of its own.
    let shown = with(add(&mut store, "shown", 100), "rule: CVE");
    store
        .record_notices(&[(shown.clone(), NoticeState::Sent)], 100)
        .unwrap();
    store
        .record_popup(&[shown.article_id], 100, Some(1))
        .unwrap();
    // 200: that story grows, and its pop-up says so.
    let grew = Notice {
        article_id: add(&mut store, "grew", 200).article_id,
        reason: "now 7 outlets".into(),
        ..shown.clone()
    };
    store.record_update(&grew, 200, 1, Some(2)).unwrap();
    // 300 and 310: held over the budget, then sent together at 400.
    let first = with(add(&mut store, "first", 300), "rule: Rust");
    let second = with(add(&mut store, "second", 310), "6 outlets in 40 min");
    store
        .record_notices(&[(first.clone(), NoticeState::Held)], 300)
        .unwrap();
    store
        .record_notices(&[(second.clone(), NoticeState::Held)], 310)
        .unwrap();
    // Summed up, a held row keeps when it matched.
    store
        .record_popup(&[first.article_id, second.article_id], 400, Some(3))
        .unwrap();
    // 500: still waiting; 600: the window had focus.
    let held = with(add(&mut store, "held", 500), "rule: Rust");
    store
        .record_notices(&[(held.clone(), NoticeState::Held)], 500)
        .unwrap();
    let seen = with(add(&mut store, "seen", 600), "rule: CVE");
    store
        .record_notices(&[(seen.clone(), NoticeState::Seen)], 600)
        .unwrap();

    let history: Vec<_> = store
        .notification_history()
        .unwrap()
        .into_iter()
        .map(|n| (n.title, n.source, n.reason, n.at, n.fate))
        .collect();
    let row = |guid: &str, reason: &str, at, fate| {
        (
            format!("Headline {guid}"),
            guid.to_owned(),
            reason.to_owned(),
            at,
            fate,
        )
    };
    assert_eq!(
        history,
        [
            row("seen", "rule: CVE", 600, Fate::Seen),
            row("held", "rule: Rust", 500, Fate::Held),
            row("second", "6 outlets in 40 min", 310, Fate::Summary),
            row("first", "rule: Rust", 300, Fate::Summary),
            row("grew", "now 7 outlets", 200, Fate::Updated),
            row("shown", "rule: CVE", 100, Fate::Shown),
        ]
    );
    assert_eq!(
        store.notification_history().unwrap()[0].article_id,
        seen.article_id
    );
}

#[test]
fn no_notifications_is_an_empty_history() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.notification_history().unwrap().is_empty());
}
