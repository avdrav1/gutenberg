//! Desktop notifications (S10.1): what a `notify` rule's matches become.
//!
//! ```toml
//! [[rules]]
//! match = "CVE"
//! action = "notify"
//!
//! [notify]
//! per_hour = 3            # the default
//! quiet = "22:00-07:00"   # local time; none by default
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

use std::collections::HashSet;

#[cfg(target_os = "linux")]
mod desktop;
#[cfg(target_os = "linux")]
pub use desktop::Desktop;

use gutenberg_store::{Notice, NoticeState, Store};

use crate::config::Notify;

/// One pop-up: a notice alone, or a summary of what waited (even one) or
/// what came past the budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    pub notices: Vec<Notice>,
    pub folded: bool,
}

impl Popup {
    fn alone(notice: Notice) -> Self {
        Self {
            notices: vec![notice],
            folded: false,
        }
    }

    fn folded(notices: Vec<Notice>) -> Self {
        Self {
            notices,
            folded: true,
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

    /// Lines under it: the rule and the source for one, a line per story
    /// (at most five) for a summary.
    pub fn body(&self) -> String {
        const SHOWN: usize = 5;
        match (self.folded, self.notices.as_slice()) {
            (false, [one]) => format!("rule: {} · {}", one.reason, one.source),
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
pub fn plan(
    settings: &Notify,
    new: Vec<Notice>,
    held: Vec<Notice>,
    noticed: impl Fn(i64) -> bool,
    moment: Moment,
) -> Plan {
    let mut plan = Plan::default();
    if !settings.enabled {
        return plan;
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
        plan.states = held
            .into_iter()
            .chain(new)
            .map(|n| (n, NoticeState::Seen))
            .collect();
        return plan;
    }
    let quiet = settings.quiet.is_some_and(|q| q.contains(moment.minute));
    let budget = settings.per_hour.saturating_sub(moment.popups_last_hour) as usize;
    if quiet || budget == 0 {
        plan.states = new.into_iter().map(|n| (n, NoticeState::Held)).collect();
        return plan;
    }
    // Held ones and anything new fold into one summary.
    if !held.is_empty() {
        let all: Vec<Notice> = held.into_iter().chain(new).collect();
        plan.states = all.iter().map(|n| (n.clone(), NoticeState::Sent)).collect();
        plan.popups.push(Popup::folded(all));
        return plan;
    }
    plan.states = new.iter().map(|n| (n.clone(), NoticeState::Sent)).collect();
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

/// Where pop-ups go: the desktop over D-Bus, or a fake in tests.
pub trait Sender {
    /// Shows `popup`; the desktop's id for it, if it answered.
    fn send(&mut self, popup: &Popup) -> Option<u32>;
}

/// After a refresh at `now`: turns `matched` (`(source_id, guid, rule)`
/// from the rules) and any held notices into pop-ups, records them and
/// sends them. Returns how many pop-ups went out.
pub fn deliver(
    store: &mut Store,
    settings: &Notify,
    matched: &[(i64, String, String)],
    now: i64,
    focused: bool,
    sender: &mut dyn Sender,
) -> gutenberg_store::Result<usize> {
    let new = store.new_notices(matched, now)?;
    let held = store.held_notices()?;
    if new.is_empty() && held.is_empty() {
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
        |story| noticed.get(&story).copied().unwrap_or(false),
        moment,
    );
    store.record_notices(&plan.states, now)?;
    for popup in &plan.popups {
        let id = sender.send(popup);
        let articles: Vec<i64> = popup.notices.iter().map(|n| n.article_id).collect();
        store.record_popup(&articles, now, id)?;
    }
    Ok(plan.popups.len())
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
            reason: "CVE".into(),
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

    #[test]
    fn a_long_summary_lists_five_and_counts_the_rest() {
        let popup = Popup::folded((1..=7).map(|i| notice(i, i)).collect());
        assert_eq!(popup.summary(), "7 more stories");
        assert_eq!(popup.body().lines().count(), 6);
        assert!(popup.body().ends_with("and 2 more"));
        assert_eq!(popup.article(), Some(7));
    }
}
