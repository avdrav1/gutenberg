//! Pane layout: how the panes are arranged, and each pane's share of the
//! window, kept so what a drag on a divider, `<` and `>` and `L` change
//! survives a resize and a relaunch. Pure, so it can be tested without a
//! window; `grid` turns an arrangement and its sizes into the `pane_grid`
//! state the view draws.

use gutenberg_core::config::{Arrangement, Layout};
use iced::widget::pane_grid::{self, Axis, Configuration};

/// The panes, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    Sources,
    List,
    Article,
}

const ALL: [Id; 3] = [Id::Sources, Id::List, Id::Article];

/// Narrowest, or shortest, a pane can be dragged or keyed to, in logical
/// pixels.
pub const MIN_WIDTH: f32 = 120.0;
/// How far `<` and `>` move a divider, as a share of the window.
pub const STEP: f32 = 0.05;
/// The store setting the columns arrangement's widths are saved under,
/// since 0.27.0.
pub const SETTING: &str = "layout.widths";
/// The store setting the chosen arrangement is saved under.
pub const ARRANGEMENT_SETTING: &str = "layout.arrangement";
/// The list's share of the height where it sits above the article, until a
/// drag changes it.
pub const DEFAULT_ROWS: f32 = 0.4;

/// What `pane_grid` has no name for: the arrangement's order, names and
/// settings.
pub trait Arrange: Sized {
    /// After `L`: columns, classic, wide, reading, and round again.
    fn next(self) -> Self;
    /// As the palette and `config.toml` name it.
    fn name(self) -> &'static str;
    fn from_name(name: &str) -> Option<Self>;
    /// Where the arrangement's sizes are kept; the article alone has none.
    fn sizes_setting(self) -> Option<&'static str>;
}

impl Arrange for Arrangement {
    fn next(self) -> Self {
        match self {
            Self::Columns => Self::Classic,
            Self::Classic => Self::Wide,
            Self::Wide => Self::Reading,
            Self::Reading => Self::Columns,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Classic => "classic",
            Self::Wide => "wide",
            Self::Reading => "reading",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        ARRANGEMENTS.into_iter().find(|a| a.name() == name.trim())
    }

    fn sizes_setting(self) -> Option<&'static str> {
        match self {
            Self::Columns => Some(SETTING),
            Self::Classic => Some("layout.classic"),
            Self::Wide => Some("layout.wide"),
            Self::Reading => None,
        }
    }
}

/// In `L`'s order.
pub const ARRANGEMENTS: [Arrangement; 4] = [
    Arrangement::Columns,
    Arrangement::Classic,
    Arrangement::Wide,
    Arrangement::Reading,
];

/// Each pane's share of the window; 0 for a hidden pane. The shares of the
/// panes that show sum to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Widths {
    pub sources: f32,
    pub list: f32,
    pub article: f32,
}

impl Widths {
    /// The shares `[layout]` in `config.toml` gives: its relative widths,
    /// 0 hiding a pane. Config guarantees at least one is wider than 0.
    pub fn from_layout(layout: Layout) -> Self {
        Self {
            sources: f32::from(layout.sources),
            list: f32::from(layout.list),
            article: f32::from(layout.article),
        }
        .normalized()
    }

    /// These shares with `layout`'s choice of which panes show: a pane the
    /// config hides goes to 0; one it shows that was hidden takes the
    /// config's share; the rest keep their proportions.
    pub fn shown(self, layout: Layout) -> Self {
        let from_layout = Self::from_layout(layout);
        let mut shown = self;
        for id in ALL {
            let share = match (layout_share(layout, id), self.get(id)) {
                (0, _) => 0.0,
                (_, hidden) if hidden <= 0.0 => from_layout.get(id),
                (_, share) => share,
            };
            shown.set(id, share);
        }
        shown.normalized()
    }

    pub fn get(&self, id: Id) -> f32 {
        match id {
            Id::Sources => self.sources,
            Id::List => self.list,
            Id::Article => self.article,
        }
    }

    fn set(&mut self, id: Id, share: f32) {
        match id {
            Id::Sources => self.sources = share,
            Id::List => self.list = share,
            Id::Article => self.article = share,
        }
    }

    /// Makes the showing shares sum to 1. A pane at or below 0 stays hidden.
    fn normalized(mut self) -> Self {
        let total: f32 = ALL.iter().map(|id| self.get(*id).max(0.0)).sum();
        for id in ALL {
            let share = self.get(id);
            self.set(
                id,
                if share > 0.0 && total > 0.0 {
                    share / total
                } else {
                    0.0
                },
            );
        }
        self
    }

    /// The panes that show, left to right.
    pub fn visible(&self) -> Vec<Id> {
        ALL.into_iter().filter(|id| self.get(*id) > 0.0).collect()
    }

    /// The `pane_grid` split ratios, one per divider from the left: the
    /// first pane against the rest, then the second against the third.
    pub fn dividers(&self) -> Vec<f32> {
        let shares: Vec<f32> = self.visible().iter().map(|id| self.get(*id)).collect();
        (0..shares.len().saturating_sub(1))
            .map(|i| {
                let rest: f32 = shares[i..].iter().sum();
                shares[i] / rest
            })
            .collect()
    }

    /// Moves divider `divider` (0 is the leftmost) to `ratio` of the space it
    /// splits, as a drag reports it, keeping every pane at least `MIN_WIDTH`
    /// of a `window` pixels wide. Returns the ratio that applied.
    pub fn resize(&mut self, divider: usize, ratio: f32, window: f32) -> f32 {
        let visible = self.visible();
        if divider + 1 >= visible.len() {
            return ratio;
        }
        let min = MIN_WIDTH / window.max(MIN_WIDTH * visible.len() as f32);
        let rest: f32 = visible[divider..].iter().map(|id| self.get(*id)).sum();
        // The panes right of the divider scale together, so the narrowest
        // of them is what the right-hand bound protects.
        let right: Vec<f32> = visible[divider + 1..]
            .iter()
            .map(|id| self.get(*id))
            .collect();
        let right_total: f32 = right.iter().sum();
        let narrowest = right.iter().copied().fold(f32::INFINITY, f32::min);
        let lo = min / rest;
        let hi = 1.0 - (min / rest) * (right_total / narrowest);
        if lo > hi {
            return self.dividers()[divider];
        }
        let ratio = ratio.clamp(lo, hi);
        let left = ratio * rest;
        self.set(visible[divider], left);
        let scale = (rest - left) / right_total;
        for id in &visible[divider + 1..] {
            self.set(*id, self.get(*id) * scale);
        }
        ratio
    }

    /// Moves the divider between the sources pane and everything right of
    /// it, which stands as one column there (the classic arrangement), to
    /// `ratio`: the sources pane and that column each keep `MIN_WIDTH`.
    pub fn resize_sources(&mut self, ratio: f32, window: f32) -> f32 {
        if self.sources <= 0.0 || self.list + self.article <= 0.0 {
            return ratio;
        }
        let min = MIN_WIDTH / window.max(2.0 * MIN_WIDTH);
        let ratio = ratio.clamp(min, 1.0 - min);
        let scale = (1.0 - ratio) / (self.list + self.article);
        self.list *= scale;
        self.article *= scale;
        self.sources = ratio;
        ratio
    }

    /// Widens `pane` by `delta` of the window (narrows it for a negative
    /// `delta`), taking the space from its right-hand neighbour, or the left
    /// one when it's last. Nothing changes, and `false` comes back, when the
    /// pane is alone or the step would take a pane under `MIN_WIDTH`.
    pub fn nudge(&mut self, pane: Id, delta: f32, window: f32) -> bool {
        let visible = self.visible();
        let Some(at) = visible.iter().position(|id| *id == pane) else {
            return false;
        };
        let Some(neighbour) = visible
            .get(at + 1)
            .or_else(|| at.checked_sub(1).map(|i| &visible[i]))
        else {
            return false;
        };
        let min = MIN_WIDTH / window.max(MIN_WIDTH * visible.len() as f32);
        let room = if delta > 0.0 {
            delta.min(self.get(*neighbour) - min)
        } else {
            delta.max(min - self.get(pane))
        };
        if room == 0.0 || room.signum() != delta.signum() {
            return false;
        }
        self.set(pane, self.get(pane) + room);
        self.set(*neighbour, self.get(*neighbour) - room);
        true
    }

    /// `nudge` for the sources pane against the one column right of it.
    pub fn nudge_sources(&mut self, delta: f32, window: f32) -> bool {
        if self.sources <= 0.0 || self.list + self.article <= 0.0 {
            return false;
        }
        let min = MIN_WIDTH / window.max(2.0 * MIN_WIDTH);
        let room = if delta > 0.0 {
            delta.min(1.0 - min - self.sources)
        } else {
            delta.max(min - self.sources)
        };
        if room == 0.0 || room.signum() != delta.signum() {
            return false;
        }
        self.resize_sources(self.sources + room, window);
        true
    }

    /// The shares as the store keeps them: `0.167,0.333,0.500`.
    pub fn to_setting(self) -> String {
        format!("{:.3},{:.3},{:.3}", self.sources, self.list, self.article)
    }

    /// Shares read back from the store; `None` for anything but three
    /// numbers with at least one above 0.
    pub fn from_setting(setting: &str) -> Option<Self> {
        let mut shares = setting.split(',').map(|s| s.trim().parse::<f32>().ok());
        let widths = Self {
            sources: shares.next()??,
            list: shares.next()??,
            article: shares.next()??,
        };
        if shares.next().is_some()
            || ALL
                .iter()
                .any(|id| !widths.get(*id).is_finite() || widths.get(*id) < 0.0)
            || ALL.iter().all(|id| widths.get(*id) == 0.0)
        {
            return None;
        }
        Some(widths.normalized())
    }
}

fn layout_share(layout: Layout, id: Id) -> u16 {
    match id {
        Id::Sources => layout.sources,
        Id::List => layout.list,
        Id::Article => layout.article,
    }
}

/// One arrangement's divider positions: the panes' widths, and where the
/// list sits above the article, the list's share of the height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizes {
    pub widths: Widths,
    pub rows: f32,
}

/// A divider of the grid, by what moving it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    /// Between two of the columns, from the left.
    Column(usize),
    /// Between the sources pane and the stacked list and article.
    SourcesVsRest,
    /// Between the list and the article below it.
    Rows,
}

impl Sizes {
    pub fn from_layout(layout: Layout) -> Self {
        Self {
            widths: Widths::from_layout(layout),
            rows: DEFAULT_ROWS,
        }
    }

    /// With `layout`'s choice of which panes show.
    pub fn shown(self, layout: Layout) -> Self {
        Self {
            widths: self.widths.shown(layout),
            rows: self.rows,
        }
    }

    /// Moves `divider` to `ratio` as a drag reports it, within a window
    /// `width` by `height` pixels. Returns the ratio that applied.
    pub fn resize(&mut self, divider: Divider, ratio: f32, width: f32, height: f32) -> f32 {
        match divider {
            Divider::Column(i) => self.widths.resize(i, ratio, width),
            Divider::SourcesVsRest => self.widths.resize_sources(ratio, width),
            Divider::Rows => {
                let min = MIN_WIDTH / height.max(2.0 * MIN_WIDTH);
                self.rows = ratio.clamp(min, 1.0 - min);
                self.rows
            }
        }
    }

    /// `<` and `>` on `pane` in `arrangement`: wider or narrower where the
    /// pane has a neighbour beside it, taller or shorter for the list above
    /// the article. `false` when nothing can move.
    pub fn nudge(
        &mut self,
        arrangement: Arrangement,
        pane: Id,
        delta: f32,
        width: f32,
        height: f32,
    ) -> bool {
        match (arrangement, pane) {
            (Arrangement::Columns, pane) => self.widths.nudge(pane, delta, width),
            (Arrangement::Classic, Id::Sources) => self.widths.nudge_sources(delta, width),
            (Arrangement::Classic | Arrangement::Wide, Id::List) => {
                if self.widths.list <= 0.0 || self.widths.article <= 0.0 {
                    return false;
                }
                let min = MIN_WIDTH / height.max(2.0 * MIN_WIDTH);
                let room = if delta > 0.0 {
                    delta.min(1.0 - min - self.rows)
                } else {
                    delta.max(min - self.rows)
                };
                if room == 0.0 || room.signum() != delta.signum() {
                    return false;
                }
                self.rows += room;
                true
            }
            _ => false,
        }
    }

    /// As the store keeps them: the widths, then the rows after `;`. The
    /// columns arrangement's row from 0.27.0 has no rows part.
    pub fn to_setting(self) -> String {
        format!("{};{:.3}", self.widths.to_setting(), self.rows)
    }

    pub fn from_setting(setting: &str) -> Option<Self> {
        let (widths, rows) = match setting.split_once(';') {
            Some((widths, rows)) => (widths, Some(rows)),
            None => (setting, None),
        };
        let widths = Widths::from_setting(widths)?;
        let rows = match rows {
            Some(rows) => rows
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|r| r.is_finite() && *r > 0.0 && *r < 1.0)?,
            None => DEFAULT_ROWS,
        };
        Some(Self { widths, rows })
    }
}

/// The panes `arrangement` shows with `sizes`, in reading order.
pub fn visible(arrangement: Arrangement, sizes: &Sizes) -> Vec<Id> {
    let w = &sizes.widths;
    match arrangement {
        Arrangement::Columns => w.visible(),
        Arrangement::Classic => w.visible(),
        Arrangement::Wide => {
            let stack: Vec<Id> = [Id::List, Id::Article]
                .into_iter()
                .filter(|id| w.get(*id) > 0.0)
                .collect();
            if stack.is_empty() {
                vec![Id::List]
            } else {
                stack
            }
        }
        Arrangement::Reading => vec![Id::Article],
    }
}

/// The grid for `arrangement` at `sizes`, and its dividers paired with
/// what each one moves, in the order `pane_grid` yields them.
pub fn grid(
    arrangement: Arrangement,
    sizes: &Sizes,
) -> (pane_grid::State<Id>, Vec<(pane_grid::Split, Divider)>) {
    let (configuration, dividers) = configuration(arrangement, sizes);
    let state = pane_grid::State::with_configuration(configuration);
    // The root split is yielded first, then the one inside its second
    // half; every tree here nests in the second half only.
    let splits = state.layout().splits().copied().zip(dividers).collect();
    (state, splits)
}

fn configuration(arrangement: Arrangement, sizes: &Sizes) -> (Configuration<Id>, Vec<Divider>) {
    let shown = visible(arrangement, sizes);
    let w = &sizes.widths;
    match arrangement {
        Arrangement::Columns => {
            let dividers = (0..shown.len().saturating_sub(1))
                .map(Divider::Column)
                .collect();
            (columns(&shown, &w.dividers()), dividers)
        }
        Arrangement::Reading => (Configuration::Pane(Id::Article), Vec::new()),
        Arrangement::Classic | Arrangement::Wide => {
            let left =
                (arrangement == Arrangement::Classic && w.sources > 0.0).then_some(Id::Sources);
            let stack: Vec<Id> = shown
                .iter()
                .copied()
                .filter(|id| *id != Id::Sources)
                .collect();
            let mut dividers = Vec::new();
            let right = match stack.as_slice() {
                [] => None,
                [only] => Some(Configuration::Pane(*only)),
                [top, bottom, ..] => {
                    dividers.push(Divider::Rows);
                    Some(Configuration::Split {
                        axis: Axis::Horizontal,
                        ratio: sizes.rows,
                        a: Box::new(Configuration::Pane(*top)),
                        b: Box::new(Configuration::Pane(*bottom)),
                    })
                }
            };
            match (left, right) {
                (Some(left), Some(right)) => {
                    dividers.insert(0, Divider::SourcesVsRest);
                    (
                        Configuration::Split {
                            axis: Axis::Vertical,
                            ratio: w.sources,
                            a: Box::new(Configuration::Pane(left)),
                            b: Box::new(right),
                        },
                        dividers,
                    )
                }
                (None, Some(right)) => (right, dividers),
                (Some(left), None) => (Configuration::Pane(left), dividers),
                (None, None) => (Configuration::Pane(Id::List), dividers),
            }
        }
    }
}

fn columns(visible: &[Id], ratios: &[f32]) -> Configuration<Id> {
    match visible {
        [] => Configuration::Pane(Id::List),
        [only] => Configuration::Pane(*only),
        [first, rest @ ..] => Configuration::Split {
            axis: Axis::Vertical,
            ratio: ratios[0],
            a: Box::new(Configuration::Pane(*first)),
            b: Box::new(columns(rest, &ratios[1..])),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(sources: u16, list: u16, article: u16) -> Layout {
        Layout {
            sources,
            list,
            article,
            ..Layout::default()
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_default_layout_is_a_sixth_a_third_and_a_half() {
        let w = Widths::from_layout(Layout::default());
        assert!(close(w.sources, 1.0 / 6.0) && close(w.list, 1.0 / 3.0) && close(w.article, 0.5));
        assert_eq!(w.dividers().len(), 2);
        assert!(close(w.dividers()[0], 1.0 / 6.0) && close(w.dividers()[1], 0.4));
    }

    #[test]
    fn a_hidden_pane_has_no_divider_and_the_rest_fill_the_window() {
        let w = Widths::from_layout(layout(0, 4, 6));
        assert_eq!(w.visible(), [Id::List, Id::Article]);
        assert!(close(w.list, 0.4) && close(w.article, 0.6));
        assert_eq!(w.dividers().len(), 1);
        assert!(Widths::from_layout(layout(0, 1, 0)).dividers().is_empty());
    }

    #[test]
    fn dragging_the_left_divider_scales_the_two_panes_right_of_it_together() {
        let mut w = Widths::from_layout(Layout::default());
        let applied = w.resize(0, 0.3, 1200.0);
        assert!(close(applied, 0.3));
        assert!(close(w.sources, 0.3));
        // list : article stays 2 : 3.
        assert!(close(w.list / w.article, 2.0 / 3.0));
        assert!(close(w.sources + w.list + w.article, 1.0));
    }

    #[test]
    fn dragging_the_right_divider_leaves_the_left_pane_alone() {
        let mut w = Widths::from_layout(Layout::default());
        w.resize(1, 0.5, 1200.0);
        assert!(close(w.sources, 1.0 / 6.0));
        assert!(close(w.list, w.article));
    }

    #[test]
    fn a_drag_stops_where_a_pane_would_go_under_the_minimum() {
        let mut w = Widths::from_layout(Layout::default());
        // 120 px of 1200 is a tenth.
        let applied = w.resize(0, 0.01, 1200.0);
        assert!(close(applied, 0.1) && close(w.sources, 0.1));
        // Pushing right: the list is the narrower of the two right-hand
        // panes, so it is what stops first, at a tenth.
        let applied = w.resize(0, 0.99, 1200.0);
        assert!(close(w.list, 0.1), "{w:?}");
        assert!(close(applied, w.dividers()[0]));
        assert!(w.article > w.list);
    }

    #[test]
    fn a_window_too_narrow_for_the_minimums_ignores_the_drag() {
        let mut w = Widths::from_layout(Layout::default());
        let before = w;
        w.resize(0, 0.5, 200.0);
        assert_eq!(w, before);
    }

    #[test]
    fn a_resize_on_a_divider_that_isnt_there_changes_nothing() {
        let mut w = Widths::from_layout(layout(0, 1, 0));
        let before = w;
        assert!(close(w.resize(0, 0.3, 1200.0), 0.3));
        assert_eq!(w, before);
    }

    #[test]
    fn nudging_widens_the_pane_from_its_right_neighbour() {
        let mut w = Widths::from_layout(Layout::default());
        assert!(w.nudge(Id::List, STEP, 1200.0));
        assert!(close(w.list, 1.0 / 3.0 + STEP) && close(w.article, 0.5 - STEP));
        assert!(close(w.sources, 1.0 / 6.0));
        assert!(w.nudge(Id::Sources, -STEP, 1200.0));
        assert!(close(w.sources, 1.0 / 6.0 - STEP) && close(w.list, 1.0 / 3.0 + 2.0 * STEP));
    }

    #[test]
    fn the_last_pane_takes_from_its_left_neighbour_and_a_lone_pane_cant_move() {
        let mut w = Widths::from_layout(layout(0, 4, 6));
        assert!(w.nudge(Id::Article, STEP, 1200.0));
        assert!(close(w.article, 0.6 + STEP) && close(w.list, 0.4 - STEP));
        let mut alone = Widths::from_layout(layout(0, 1, 0));
        assert!(!alone.nudge(Id::List, STEP, 1200.0));
        assert!(
            !w.nudge(Id::Sources, STEP, 1200.0),
            "a hidden pane has nothing to widen"
        );
    }

    #[test]
    fn a_nudge_stops_at_the_minimum_and_reports_when_it_cant_move() {
        let mut w = Widths::from_layout(Layout::default());
        // From a sixth, one whole step; the floor at 1200 px is a tenth, so
        // the next step is cut to what fits, and the one after can't move.
        assert!(w.nudge(Id::Sources, -STEP, 1200.0));
        assert!(close(w.sources, 1.0 / 6.0 - STEP), "{w:?}");
        assert!(w.nudge(Id::Sources, -STEP, 1200.0));
        assert!(close(w.sources, 0.1), "{w:?}");
        assert!(!w.nudge(Id::Sources, -STEP, 1200.0));
        assert!(close(w.sources, 0.1));
    }

    #[test]
    fn the_setting_round_trips_and_rejects_junk() {
        let w = Widths::from_layout(Layout::default());
        assert_eq!(w.to_setting(), "0.167,0.333,0.500");
        // The store keeps three decimals.
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let back = Widths::from_setting(&w.to_setting()).unwrap();
        assert!(near(back.sources, w.sources) && near(back.list, w.list));
        assert!(near(back.article, w.article));
        for junk in [
            "",
            "0.2,0.3",
            "0.2,0.3,0.5,0.1",
            "a,b,c",
            "0,0,0",
            "-1,1,1",
            "nan,1,1",
        ] {
            assert_eq!(Widths::from_setting(junk), None, "{junk:?}");
        }
        // Stored shares that don't sum to one are scaled so they do.
        let w = Widths::from_setting("1,1,2").unwrap();
        assert!(close(w.sources, 0.25) && close(w.article, 0.5));
    }

    #[test]
    fn the_config_decides_what_shows_and_stored_shares_decide_how_wide() {
        let stored = Widths::from_setting("0.3,0.3,0.4").unwrap();
        // Hiding the sources pane: the other two keep 3 : 4.
        let w = stored.shown(layout(0, 4, 6));
        assert_eq!(w.visible(), [Id::List, Id::Article]);
        assert!(close(w.list / w.article, 0.75));
        // Showing it again takes the config's share for it (a sixth, against
        // the other two's one), and they keep 3 : 4.
        let w = w.shown(Layout::default());
        assert_eq!(w.visible().len(), 3);
        assert!(close(w.sources, 1.0 / 7.0), "{w:?}");
        assert!(close(w.list / w.article, 0.75));
        assert!(close(w.sources + w.list + w.article, 1.0));
    }

    #[test]
    fn the_columns_grid_has_a_pane_per_visible_pane_and_a_divider_between_each() {
        let sizes = Sizes::from_layout(Layout::default());
        let (state, dividers) = grid(Arrangement::Columns, &sizes);
        assert_eq!(state.len(), 3);
        assert_eq!(
            dividers.iter().map(|d| d.1).collect::<Vec<_>>(),
            [Divider::Column(0), Divider::Column(1)]
        );
        let (state, dividers) = grid(Arrangement::Columns, &Sizes::from_layout(layout(0, 4, 6)));
        assert_eq!((state.len(), dividers.len()), (2, 1));
        let (state, dividers) = grid(Arrangement::Columns, &Sizes::from_layout(layout(0, 1, 0)));
        assert_eq!((state.len(), dividers.len()), (1, 0));
    }

    #[test]
    fn classic_stacks_the_list_over_the_article_beside_the_sources() {
        let sizes = Sizes::from_layout(Layout::default());
        let (state, dividers) = grid(Arrangement::Classic, &sizes);
        assert_eq!(state.len(), 3);
        assert_eq!(
            dividers.iter().map(|d| d.1).collect::<Vec<_>>(),
            [Divider::SourcesVsRest, Divider::Rows]
        );
        // Hiding the sources pane leaves the stack alone; hiding the article
        // leaves sources beside the list, with no rows divider.
        let (state, dividers) = grid(Arrangement::Classic, &Sizes::from_layout(layout(0, 4, 6)));
        assert_eq!(state.len(), 2);
        assert_eq!(
            dividers.iter().map(|d| d.1).collect::<Vec<_>>(),
            [Divider::Rows]
        );
        let (state, dividers) = grid(Arrangement::Classic, &Sizes::from_layout(layout(2, 4, 0)));
        assert_eq!(state.len(), 2);
        assert_eq!(
            dividers.iter().map(|d| d.1).collect::<Vec<_>>(),
            [Divider::SourcesVsRest]
        );
    }

    #[test]
    fn wide_never_shows_the_sources_and_reading_is_the_article_alone() {
        let sizes = Sizes::from_layout(Layout::default());
        assert_eq!(visible(Arrangement::Wide, &sizes), [Id::List, Id::Article]);
        let (state, dividers) = grid(Arrangement::Wide, &sizes);
        assert_eq!(state.len(), 2);
        assert_eq!(
            dividers.iter().map(|d| d.1).collect::<Vec<_>>(),
            [Divider::Rows]
        );
        // A config that shows only the sources pane still gets a list here.
        assert_eq!(
            visible(Arrangement::Wide, &Sizes::from_layout(layout(1, 0, 0))),
            [Id::List]
        );
        assert_eq!(visible(Arrangement::Reading, &sizes), [Id::Article]);
        let (state, dividers) = grid(Arrangement::Reading, &sizes);
        assert_eq!((state.len(), dividers.len()), (1, 0));
    }

    #[test]
    fn the_rows_divider_is_the_lists_share_of_the_height_with_a_floor() {
        let mut sizes = Sizes::from_layout(Layout::default());
        assert!(close(sizes.rows, DEFAULT_ROWS));
        assert!(close(sizes.resize(Divider::Rows, 0.7, 1200.0, 800.0), 0.7));
        assert!(close(sizes.rows, 0.7));
        // 120 px of 800 is 0.15 either way.
        assert!(close(
            sizes.resize(Divider::Rows, 0.01, 1200.0, 800.0),
            0.15
        ));
        assert!(close(
            sizes.resize(Divider::Rows, 0.99, 1200.0, 800.0),
            0.85
        ));
        // The widths are untouched by a rows drag.
        assert_eq!(sizes.widths, Widths::from_layout(Layout::default()));
    }

    #[test]
    fn the_sources_divider_in_classic_treats_the_stack_as_one_column() {
        let mut sizes = Sizes::from_layout(Layout::default());
        assert!(close(
            sizes.resize(Divider::SourcesVsRest, 0.3, 1200.0, 800.0),
            0.3
        ));
        assert!(close(sizes.widths.sources, 0.3));
        assert!(close(sizes.widths.list / sizes.widths.article, 2.0 / 3.0));
        // The stack can go down to one minimum, not one per pane in it.
        assert!(close(
            sizes.resize(Divider::SourcesVsRest, 0.95, 1200.0, 800.0),
            0.9
        ));
        assert!(close(sizes.widths.sources, 0.9), "{:?}", sizes.widths);
    }

    #[test]
    fn nudging_follows_the_arrangement() {
        let mut sizes = Sizes::from_layout(Layout::default());
        // Columns: the list widens from the article.
        assert!(sizes.nudge(Arrangement::Columns, Id::List, STEP, 1200.0, 800.0));
        assert!(close(sizes.widths.list, 1.0 / 3.0 + STEP));
        // Classic: the list grows taller instead; the sources pane wider.
        let rows = sizes.rows;
        assert!(sizes.nudge(Arrangement::Classic, Id::List, STEP, 1200.0, 800.0));
        assert!(close(sizes.rows, rows + STEP));
        let sources = sizes.widths.sources;
        assert!(sizes.nudge(Arrangement::Classic, Id::Sources, STEP, 1200.0, 800.0));
        assert!(close(sizes.widths.sources, sources + STEP));
        // Wide: the list too; the sources pane isn't there.
        assert!(sizes.nudge(Arrangement::Wide, Id::List, -STEP, 1200.0, 800.0));
        assert!(close(sizes.rows, rows));
        assert!(!sizes.nudge(Arrangement::Wide, Id::Sources, STEP, 1200.0, 800.0));
        // Reading: nothing to move.
        assert!(!sizes.nudge(Arrangement::Reading, Id::List, STEP, 1200.0, 800.0));
        // The rows floor: 0.15 at 800 px.
        let mut sizes = Sizes::from_layout(Layout::default());
        sizes.rows = 0.16;
        assert!(sizes.nudge(Arrangement::Wide, Id::List, -STEP, 1200.0, 800.0));
        assert!(close(sizes.rows, 0.15));
        assert!(!sizes.nudge(Arrangement::Wide, Id::List, -STEP, 1200.0, 800.0));
    }

    #[test]
    fn sizes_round_trip_and_read_the_widths_only_row_from_0_27() {
        let sizes = Sizes {
            widths: Widths::from_layout(Layout::default()),
            rows: 0.55,
        };
        assert_eq!(sizes.to_setting(), "0.167,0.333,0.500;0.550");
        let back = Sizes::from_setting(&sizes.to_setting()).unwrap();
        assert!(close(back.rows, 0.55));
        let legacy = Sizes::from_setting("0.3,0.3,0.4").unwrap();
        assert!(close(legacy.widths.sources, 0.3) && close(legacy.rows, DEFAULT_ROWS));
        for junk in [
            "0.3,0.3,0.4;0",
            "0.3,0.3,0.4;1",
            "0.3,0.3,0.4;x",
            "junk;0.5",
        ] {
            assert_eq!(Sizes::from_setting(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn arrangements_cycle_and_have_names_and_settings() {
        let mut a = Arrangement::Columns;
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(a);
            a = a.next();
        }
        assert_eq!(seen, ARRANGEMENTS);
        assert_eq!(a, Arrangement::Columns);
        for a in ARRANGEMENTS {
            assert_eq!(Arrangement::from_name(a.name()), Some(a));
        }
        assert_eq!(Arrangement::from_name("stacked"), None);
        assert_eq!(Arrangement::Columns.sizes_setting(), Some(SETTING));
        assert_eq!(Arrangement::Reading.sizes_setting(), None);
    }
}
