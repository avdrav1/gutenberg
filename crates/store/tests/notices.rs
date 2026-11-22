//! S10.2: what an update to a breaking story's pop-up replaces, and how it
//! is recorded.

use gutenberg_store::{NewArticle, NewSource, Notice, NoticeState, SourceKind, Store};

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
