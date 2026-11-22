//! Desktop notifications: what a `notify` rule's matches (S10.1) and
//! breaking stories (S10.2) become.
//!
//! ```toml
//! [[rules]]
//! match = "CVE"
//! action = "notify"
//!
//! [notify]
//! per_hour = 3            # the default
//! quiet = "22:00-07:00"   # local time; none by default
//! breaking_in = ["News"]  # sources or folders; none by default
//! breaking_sources = 5    # the default
//! breaking_minutes = 60   # the default
//! ```
//!
//! [`plan`] decides, as a pure function of the matches, the held ones and
//! the clock; [`deliver`] reads the store, plans, records and sends. It runs
//! on the refresh thread after each refresh, never on the UI thread.
//!
//! - A story notifies once: an article of a story that already notified,
//!   was held or was seen is dropped.
//! - Nothing pops up while the window has focus; the list shows it.
//! - Past `per_hour` pop-ups in the last hour, or in quiet hours, matches
//!   are held, and come back as one summary ("4 more stories") once the
//!   budget and the clock allow.
//! - A story is breaking when, within `breaking_minutes`, its articles come
//!   from `breaking_sources` sources, one of them in `breaking_in`. Once it
//!   has popped up, each refresh that brings it a new source replaces that
//!   pop-up ("now 7 outlets") without spending the budget.

use std::collections::HashSet;

#[cfg(target_os = "linux")]
mod desktop;
#[cfg(target_os = "linux")]
pub use desktop::Desktop;

use gutenberg_store::{Notice, NoticeState, Store, StoryArticle};

use crate::config::Notify;

/// One pop-up: a notice alone, or a summary of what waited (even one) or
/// what came past the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    pub notices: Vec<Notice>,
    pub folded: bool,
    /// The desktop's id of an earlier pop-up this one replaces.
    pub replaces: Option<u32>,
}

impl Popup {
    fn alone(notice: Notice) -> Self {
        Self {
            notices: vec![notice],
            folded: false,
            replaces: None,
        }
    }

    fn folded(notices: Vec<Notice>) -> Self {
        Self {
            notices,
            folded: true,
            replaces: None,
        }
    }

    /// The bold line.
    pub fn summary(&self) -> String {
        match (self.folded, self.notices.as_slice()) {
            (false, [one]) => one.title.clone(),
            (_, [_]) => "1 more story".to_owned(),
            (_, many) => format!("{} more stories", many.len()),
        }
    }

    /// Lines under it: the reason and the source for one, a line per story
    /// (at most five) for a summary.
    pub fn body(&self) -> String {
        const SHOWN: usize = 5;
        match (self.folded, self.notices.as_slice()) {
            (false, [one]) => format!("{} · {}", one.reason, one.source),
            (_, many) => {
                let mut lines: Vec<String> = many
                    .iter()
                    .take(SHOWN)
                    .map(|n| format!("{} · {}", n.title, n.source))
                    .collect();
                if many.len() > SHOWN {
                    lines.push(format!("and {} more", many.len() - SHOWN));
                }
                lines.join("\n")
            }
        }
    }

    /// The article Open shows: a single notice's, or a summary's newest.
    pub fn article(&self) -> Option<i64> {
        self.notices.last().map(|n| n.article_id)
    }
}

/// What a refresh's matches become.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Every notice whose state is new or changes.
    pub states: Vec<(Notice, NoticeState)>,
    pub popups: Vec<Popup>,
    /// Pop-ups that replace a story's earlier one; they don't count
    /// against the hour.
    pub updates: Vec<Popup>,
}

/// The moment a plan is made for.
#[derive(Debug, Clone, Copy)]
pub struct Moment {
    /// Pop-ups shown in the last hour.
    pub popups_last_hour: u32,
    /// Minutes past local midnight.
    pub minute: u16,
    /// Whether the window has focus.
    pub focused: bool,
}

/// What `new` matches (with `held` ones from before) become. `noticed`
/// says whether a story already has a row; `new` comes in arrival order.
/// `updates` replace the pop-up whose desktop id they carry.
pub fn plan(
    settings: &Notify,
    new: Vec<Notice>,
    held: Vec<Notice>,
    updates: Vec<(Notice, u32)>,
    noticed: impl Fn(i64) -> bool,
    moment: Moment,
) -> Plan {
    let mut plan = Plan::default();
    if !settings.enabled {
        return plan;
    }
    // An update waits for nothing: in quiet hours or with the window
    // focused it's only seen, and the next new source updates again.
    let quiet = settings.quiet.is_some_and(|q| q.contains(moment.minute));
    for (notice, id) in updates {
        if moment.focused || quiet {
            plan.states.push((notice, NoticeState::Seen));
        } else {
            plan.updates.push(Popup {
                replaces: Some(id),
                ..Popup::alone(notice)
            });
        }
    }
    // Once per story, against the history and within this batch.
    let mut stories: HashSet<i64> = held.iter().map(|n| n.story_id).collect();
    let new: Vec<Notice> = new
        .into_iter()
        .filter(|n| !noticed(n.story_id) && stories.insert(n.story_id))
        .collect();
    if new.is_empty() && held.is_empty() {
        return plan;
    }
    if moment.focused {
        plan.states
            .extend(held.into_iter().chain(new).map(|n| (n, NoticeState::Seen)));
        return plan;
    }
    let budget = settings.per_hour.saturating_sub(moment.popups_last_hour) as usize;
    if quiet || budget == 0 {
        plan.states
            .extend(new.into_iter().map(|n| (n, NoticeState::Held)));
        return plan;
    }
    // Held ones and anything new fold into one summary.
    if !held.is_empty() {
        let all: Vec<Notice> = held.into_iter().chain(new).collect();
        plan.states
            .extend(all.iter().map(|n| (n.clone(), NoticeState::Sent)));
        plan.popups.push(Popup::folded(all));
        return plan;
    }
    plan.states
        .extend(new.iter().map(|n| (n.clone(), NoticeState::Sent)));
    if new.len() <= budget {
        plan.popups = new.into_iter().map(Popup::alone).collect();
    } else {
        // Room for the summary: one fewer alone.
        let mut rest = new;
        let alone: Vec<Notice> = rest.drain(..budget - 1).collect();
        plan.popups = alone.into_iter().map(Popup::alone).collect();
        plan.popups.push(Popup::folded(rest));
    }
    plan
}

/// A story a refresh made breaking, or brought a new source after its
/// pop-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breaking {
    /// The newest article the refresh brought, with the outlets as reason.
    pub notice: Notice,
    /// Whether the story has a row already, so this updates its pop-up.
    pub update: bool,
}

/// The breaking stories among `articles`, every article of the stories a
/// refresh at `now` added to, by story (as
/// [`Store::touched_stories`] gives them).
pub fn breaking(settings: &Notify, articles: &[StoryArticle], now: i64) -> Vec<Breaking> {
    let places: Vec<String> = settings
        .breaking_in
        .iter()
        .map(|p| p.trim().to_lowercase())
        .collect();
    if places.is_empty() {
        return Vec::new();
    }
    // One source alone is never a story spreading.
    let enough = settings.breaking_sources.max(2) as usize;
    let since = now - i64::from(settings.breaking_minutes) * 60;
    let named = |a: &StoryArticle| {
        places.contains(&a.source.to_lowercase())
            || a.folder
                .as_ref()
                .is_some_and(|f| places.contains(&f.to_lowercase()))
    };
    articles
        .chunk_by(|a, b| a.story_id == b.story_id)
        .filter_map(|story| {
            if !story.iter().any(named) {
                return None;
            }
            let recent: Vec<&StoryArticle> = story.iter().filter(|a| a.sort_at >= since).collect();
            let outlets: HashSet<i64> = recent.iter().map(|a| a.source_id).collect();
            if outlets.len() < enough {
                return None;
            }
            let fresh = |a: &&StoryArticle| a.fetched_at == now && !a.noticed;
            let newest = story
                .iter()
                .filter(fresh)
                .max_by_key(|a| (a.sort_at, a.article_id))?;
            let update = story.iter().any(|a| a.noticed);
            let reason = if update {
                // Only a source the story didn't have counts as growth.
                let before: HashSet<i64> = story
                    .iter()
                    .filter(|a| !fresh(a))
                    .map(|a| a.source_id)
                    .collect();
                if story
                    .iter()
                    .filter(fresh)
                    .all(|a| before.contains(&a.source_id))
                {
                    return None;
                }
                format!("now {} outlets", outlets.len())
            } else {
                let first = recent.iter().map(|a| a.sort_at).min().unwrap_or(now);
                let last = recent.iter().map(|a| a.sort_at).max().unwrap_or(now);
                let minutes = ((last - first + 59) / 60).max(1);
                format!("{} outlets in {minutes} min", outlets.len())
            };
            Some(Breaking {
                notice: Notice {
                    article_id: newest.article_id,
                    story_id: newest.story_id,
                    title: newest.title.clone(),
                    source: newest.source.clone(),
                    reason,
                },
                update,
            })
        })
        .collect()
}

/// Where pop-ups go: the desktop over D-Bus, or a fake in tests.
pub trait Sender {
    /// Shows `popup`; the desktop's id for it, if it answered.
    fn send(&mut self, popup: &Popup) -> Option<u32>;
}

/// After a refresh at `now`: turns `matched` (`(source_id, guid, reason)`
/// from the rules), breaking stories and any held notices into pop-ups,
/// records them and sends them. Returns how many pop-ups went out,
/// updates included.
pub fn deliver(
    store: &mut Store,
    settings: &Notify,
    matched: &[(i64, String, String)],
    now: i64,
    focused: bool,
    sender: &mut dyn Sender,
) -> gutenberg_store::Result<usize> {
    if !settings.enabled {
        return Ok(0);
    }
    let mut new = store.new_notices(matched, now)?;
    let mut updates = Vec::new();
    if !settings.breaking_in.is_empty() {
        for found in breaking(settings, &store.touched_stories(now)?, now) {
            if !found.update {
                new.push(found.notice);
            } else if let Some(id) = store.story_popup(found.notice.story_id)? {
                updates.push((found.notice, id));
            }
        }
    }
    let held = store.held_notices()?;
    if new.is_empty() && held.is_empty() && updates.is_empty() {
        return Ok(0);
    }
    let mut noticed = std::collections::HashMap::new();
    for story in new.iter().map(|n| n.story_id) {
        noticed.insert(story, store.story_noticed(story)?);
    }
    let moment = Moment {
        popups_last_hour: store.popups_since(now - 3600)?,
        minute: store.local_minute(now)?,
        focused,
    };
    let plan = plan(
        settings,
        new,
        held,
        updates,
        |story| noticed.get(&story).copied().unwrap_or(false),
        moment,
    );
    store.record_notices(&plan.states, now)?;
    for popup in &plan.popups {
        let id = sender.send(popup);
        let articles: Vec<i64> = popup.notices.iter().map(|n| n.article_id).collect();
        store.record_popup(&articles, now, id)?;
    }
    for update in &plan.updates {
        let answered = sender.send(update);
        if let (Some(replaced), [notice]) = (update.replaces, update.notices.as_slice()) {
            store.record_update(notice, now, replaced, answered)?;
        }
    }
    Ok(plan.popups.len() + plan.updates.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Quiet;

    fn notice(id: i64, story: i64) -> Notice {
        Notice {
            article_id: id,
            story_id: story,
            title: format!("t{id}"),
            source: "LWN".into(),
            reason: "rule: CVE".into(),
        }
    }

    fn at(popups_last_hour: u32, minute: u16, focused: bool) -> Moment {
        Moment {
            popups_last_hour,
            minute,
            focused,
        }
    }

    fn states(plan: &Plan) -> Vec<(i64, NoticeState)> {
        plan.states
            .iter()
            .map(|(n, s)| (n.article_id, *s))
            .collect()
    }

    fn shown(plan: &Plan) -> Vec<Vec<i64>> {
        plan.popups
            .iter()
            .map(|p| p.notices.iter().map(|n| n.article_id).collect())
            .collect()
    }

    const NOON: u16 = 12 * 60;

    #[test]
    fn a_match_pops_up_naming_its_rule_and_source() {
        let plan = plan(
            &Notify::default(),
            vec![notice(1, 1)],
            vec![],
            vec![],
            |_| false,
            at(0, NOON, false),
        );
        assert_eq!(shown(&plan), [vec![1]]);
        assert_eq!(states(&plan), [(1, NoticeState::Sent)]);
        let popup = &plan.popups[0];
        assert_eq!(popup.summary(), "t1");
        assert_eq!(popup.body(), "rule: CVE · LWN");
        assert_eq!(popup.article(), Some(1));
    }

    #[test]
    fn a_story_notifies_once() {
        // 2 is already noticed; 3 and 4 are one story in this batch.
        let plan = plan(
            &Notify::default(),
            vec![notice(1, 2), notice(3, 7), notice(4, 7)],
            vec![],
            vec![],
            |story| story == 2,
            at(0, NOON, false),
        );
        assert_eq!(shown(&plan), [vec![3]]);
    }

    #[test]
    fn the_fourth_in_an_hour_waits_and_comes_as_a_summary() {
        let settings = Notify::default();
        let held = plan(
            &settings,
            vec![notice(4, 4)],
            vec![],
            vec![],
            |_| false,
            at(3, NOON, false),
        );
        assert!(held.popups.is_empty());
        assert_eq!(states(&held), [(4, NoticeState::Held)]);

        // Once the hour allows, held and new come together as one.
        let later = plan(
            &settings,
            vec![notice(5, 5)],
            vec![notice(4, 4)],
            vec![],
            |_| false,
            at(2, NOON, false),
        );
        assert_eq!(shown(&later), [vec![4, 5]]);
        assert_eq!(later.popups[0].summary(), "2 more stories");
        assert_eq!(later.popups[0].body(), "t4 · LWN\nt5 · LWN");
    }

    #[test]
    fn more_than_the_budget_at_once_ends_in_a_summary() {
        let new = (1..=5).map(|i| notice(i, i)).collect();
        let plan = plan(
            &Notify::default(),
            new,
            vec![],
            vec![],
            |_| false,
            at(0, NOON, false),
        );
        assert_eq!(shown(&plan), [vec![1], vec![2], vec![3, 4, 5]]);
    }

    #[test]
    fn quiet_hours_hold_and_focus_marks_seen() {
        let settings = Notify {
            quiet: Some(Quiet::try_from("22:00-07:00".to_owned()).unwrap()),
            ..Notify::default()
        };
        let night = plan(
            &settings,
            vec![notice(1, 1)],
            vec![],
            vec![],
            |_| false,
            at(0, 60, false),
        );
        assert!(night.popups.is_empty());
        assert_eq!(states(&night), [(1, NoticeState::Held)]);

        // With the window focused, nothing pops up and the held ones are
        // seen too: the list shows them.
        let focused = plan(
            &settings,
            vec![notice(2, 2)],
            vec![notice(1, 1)],
            vec![],
            |_| false,
            at(0, NOON, true),
        );
        assert!(focused.popups.is_empty());
        assert_eq!(
            states(&focused),
            [(1, NoticeState::Seen), (2, NoticeState::Seen)]
        );
    }

    #[test]
    fn turned_off_nothing_is_planned() {
        let settings = Notify {
            enabled: false,
            ..Notify::default()
        };
        assert_eq!(
            plan(
                &settings,
                vec![notice(1, 1)],
                vec![],
                vec![],
                |_| false,
                at(0, NOON, false)
            ),
            Plan::default()
        );
    }

    #[derive(Default)]
    struct Shown(Vec<String>);

    impl Sender for Shown {
        fn send(&mut self, popup: &Popup) -> Option<u32> {
            self.0.push(popup.summary());
            Some(self.0.len() as u32)
        }
    }

    #[test]
    fn over_an_hour_the_fourth_waits_then_comes_as_a_summary() {
        use gutenberg_store::{NewArticle, NewSource, SourceKind};
        let mut store = Store::open_in_memory().unwrap();
        let source = store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "LWN".into(),
                url: Some("https://lwn.example/feed".into()),
                config: "{}".into(),
            })
            .unwrap()
            .id;
        let titles = [
            "Alpha bravo charlie",
            "Delta echo foxtrot",
            "Golf hotel india",
            "Juliet kilo lima",
        ];
        let mut shown = Shown::default();
        let settings = Notify::default();
        let arrive = |store: &mut Store, i: usize, at: i64, shown: &mut Shown| {
            let guid = format!("g{i}");
            store
                .insert_articles(&[NewArticle {
                    source_id: source,
                    guid: guid.clone(),
                    url: None,
                    title: titles[i].into(),
                    author: None,
                    summary_html: None,
                    published_at: Some(at),
                    fetched_at: at,
                }])
                .unwrap();
            crate::story::group_stored(store).unwrap();
            deliver(
                store,
                &settings,
                &[(source, guid, "CVE".into())],
                at,
                false,
                shown,
            )
            .unwrap()
        };
        for (i, at) in [0, 10, 20].into_iter().enumerate() {
            assert_eq!(arrive(&mut store, i, at, &mut shown), 1);
        }
        // The fourth within the hour waits.
        assert_eq!(arrive(&mut store, 3, 30, &mut shown), 0);
        assert_eq!(store.held_notices().unwrap().len(), 1);
        // A refresh within the hour still holds it; after the hour it comes
        // as a summary.
        assert_eq!(
            deliver(&mut store, &settings, &[], 600, false, &mut shown).unwrap(),
            0
        );
        assert_eq!(
            deliver(&mut store, &settings, &[], 3_601, false, &mut shown).unwrap(),
            1
        );
        assert_eq!(shown.0.last().unwrap(), "1 more story");
        assert!(store.held_notices().unwrap().is_empty());
        assert_eq!(store.popups_since(0).unwrap(), 4);
    }

    const NOW: i64 = 1_000_000;

    /// An article of story 1 from source `source` (titled `S{source}`, in
    /// the "World" folder when `source` is even), published `ago` seconds
    /// before `NOW` and fetched at `fetched_at`.
    fn outlet(id: i64, source: i64, ago: i64, fetched_at: i64) -> StoryArticle {
        StoryArticle {
            article_id: id,
            story_id: 1,
            title: format!("t{id}"),
            source_id: source,
            source: format!("S{source}"),
            folder: (source % 2 == 0).then(|| "World".to_owned()),
            sort_at: NOW - ago,
            fetched_at,
            noticed: false,
        }
    }

    fn breaking_in(places: &[&str]) -> Notify {
        Notify {
            breaking_in: places.iter().map(|p| p.to_string()).collect(),
            ..Notify::default()
        }
    }

    #[test]
    fn five_sources_within_the_hour_is_breaking_when_one_is_named() {
        // Five sources over 40 minutes, the newest fetched now.
        let story: Vec<StoryArticle> = (1..=5)
            .map(|i| outlet(i, i, (5 - i) * 600, if i == 5 { NOW } else { NOW - 600 }))
            .collect();
        let found = breaking(&breaking_in(&["S3"]), &story, NOW);
        assert_eq!(found.len(), 1);
        assert!(!found[0].update);
        assert_eq!(found[0].notice.article_id, 5);
        assert_eq!(found[0].notice.reason, "5 outlets in 40 min");
        // A folder's name counts as a rule's `in` does, in any case.
        assert_eq!(breaking(&breaking_in(&["world"]), &story, NOW).len(), 1);
        // Named nowhere, or nothing named: quiet.
        assert!(breaking(&breaking_in(&["Elsewhere"]), &story, NOW).is_empty());
        assert!(breaking(&Notify::default(), &story, NOW).is_empty());
        // Four sources aren't enough, nor are five spread over two hours.
        assert!(breaking(&breaking_in(&["S3"]), &story[1..], NOW).is_empty());
        let mut slow = story.clone();
        slow[0].sort_at = NOW - 7200;
        assert!(breaking(&breaking_in(&["S3"]), &slow, NOW).is_empty());
        // Two articles from one source are one outlet.
        let mut twice = story[1..].to_vec();
        twice.push(outlet(6, 5, 0, NOW));
        assert!(breaking(&breaking_in(&["S3"]), &twice, NOW).is_empty());
    }

    #[test]
    fn a_breaking_story_updates_only_when_a_source_is_new() {
        let mut story: Vec<StoryArticle> = (1..=5).map(|i| outlet(i, i, 600, NOW - 600)).collect();
        story[4].noticed = true;
        // Nothing fetched now: nothing to say.
        assert!(breaking(&breaking_in(&["S1"]), &story, NOW).is_empty());
        // Another article from a source it has: still nothing.
        story.push(outlet(6, 2, 0, NOW));
        assert!(breaking(&breaking_in(&["S1"]), &story, NOW).is_empty());
        // A sixth source: an update.
        story.push(outlet(7, 6, 0, NOW));
        let found = breaking(&breaking_in(&["S1"]), &story, NOW);
        assert_eq!(found.len(), 1);
        assert!(found[0].update);
        assert_eq!(found[0].notice.reason, "now 6 outlets");
    }

    #[test]
    fn an_update_replaces_its_pop_up_without_spending_the_hour() {
        // The hour is spent, yet the update goes out, replacing pop-up 42.
        let plan = plan(
            &Notify::default(),
            vec![notice(2, 2)],
            vec![],
            vec![(notice(1, 1), 42)],
            |_| false,
            at(3, NOON, false),
        );
        assert!(plan.popups.is_empty());
        assert_eq!(plan.updates.len(), 1);
        assert_eq!(plan.updates[0].replaces, Some(42));
        assert_eq!(states(&plan), [(2, NoticeState::Held)]);
        // With the window focused it's only seen.
        let focused = plan_focused();
        assert!(focused.updates.is_empty());
        assert_eq!(states(&focused), [(1, NoticeState::Seen)]);
    }

    fn plan_focused() -> Plan {
        plan(
            &Notify::default(),
            vec![],
            vec![],
            vec![(notice(1, 1), 42)],
            |_| false,
            at(0, NOON, true),
        )
    }

    #[test]
    fn a_long_summary_lists_five_and_counts_the_rest() {
        let popup = Popup::folded((1..=7).map(|i| notice(i, i)).collect());
        assert_eq!(popup.summary(), "7 more stories");
        assert_eq!(popup.body().lines().count(), 6);
        assert!(popup.body().ends_with("and 2 more"));
        assert_eq!(popup.article(), Some(7));
    }
}
