use pixiv_manager::app::filter::{FilterState, OptionFilterKey, OptionFilters, Tristate};
use pixiv_manager::app::sort::{SortDirection, SortKey, SortState};
use pixiv_manager::test_util::mock_illust;
use pixivj_rs::model::*;
use proptest::prelude::*;

fn arb_tristate() -> impl Strategy<Value = Tristate> {
    prop_oneof![
        Just(Tristate::Indeterminate),
        Just(Tristate::Selected),
        Just(Tristate::Deselected),
    ]
}

fn arb_option_filter_key() -> impl Strategy<Value = OptionFilterKey> {
    prop_oneof![
        Just(OptionFilterKey::R18),
        Just(OptionFilterKey::Ai),
        Just(OptionFilterKey::Bookmarked),
        Just(OptionFilterKey::Landscape),
        Just(OptionFilterKey::Visible),
        Just(OptionFilterKey::Restricted),
        Just(OptionFilterKey::Missing),
    ]
}

fn arb_illust_id() -> impl Strategy<Value = u64> {
    1u64..10000
}

fn arb_sort_key() -> impl Strategy<Value = SortKey> {
    prop_oneof![
        Just(SortKey::Id),
        Just(SortKey::Views),
        Just(SortKey::Bookmarks),
        Just(SortKey::BookmarksPerView),
        Just(SortKey::Date),
        Just(SortKey::Sanity),
        Just(SortKey::Pixel),
        Just(SortKey::Pages),
    ]
}

fn arb_sort_direction() -> impl Strategy<Value = SortDirection> {
    prop_oneof![
        Just(SortDirection::Undefined),
        Just(SortDirection::Down),
        Just(SortDirection::Up),
    ]
}

/// The option-filter tristates, bundled to keep the builder readable.
struct OptionTristates {
    r18: Tristate,
    ai: Tristate,
    bm: Tristate,
    land: Tristate,
    vis: Tristate,
    rest: Tristate,
    miss: Tristate,
}

fn make_filter_state(text: String, opts: OptionTristates, tags: Vec<String>) -> FilterState {
    let OptionTristates {
        r18,
        ai,
        bm,
        land,
        vis,
        rest,
        miss,
    } = opts;
    FilterState {
        search_text: text,
        search_tags: true,
        search_author: true,
        search_title: true,
        search_caption: true,
        search_id: true,
        search_ignore_case: true,
        search_tools: false,
        options: OptionFilters {
            r18,
            ai,
            bookmarked: bm,
            landscape: land,
            visible: vis,
            restricted: rest,
            missing: miss,
        },
        selected_tags: tags,
        sources: Default::default(),
        tool_states: Default::default(),
        tag_input: String::new(),
    }
}

// ── Extension traits for property tests ──────────────────────────────────

trait OptionFiltersMut {
    fn set(&mut self, key: OptionFilterKey, value: Tristate);
}

impl OptionFiltersMut for OptionFilters {
    fn set(&mut self, key: OptionFilterKey, value: Tristate) {
        match key {
            OptionFilterKey::R18 => self.r18 = value,
            OptionFilterKey::Ai => self.ai = value,
            OptionFilterKey::Bookmarked => self.bookmarked = value,
            OptionFilterKey::Landscape => self.landscape = value,
            OptionFilterKey::Visible => self.visible = value,
            OptionFilterKey::Restricted => self.restricted = value,
            OptionFilterKey::Missing => self.missing = value,
        }
    }
}

trait SortKeyValue {
    fn extract_value(&self, illust: &Illustration) -> f64;
}

impl SortKeyValue for SortKey {
    fn extract_value(&self, illust: &Illustration) -> f64 {
        match self {
            SortKey::Id => illust.id as f64,
            SortKey::Views => illust.total_view.unwrap_or(0) as f64,
            SortKey::Bookmarks => illust.total_bookmarks.unwrap_or(0) as f64,
            SortKey::BookmarksPerView => {
                let views = illust.total_view.unwrap_or(0) as f64;
                let bm = illust.total_bookmarks.unwrap_or(0) as f64;
                if views > 0.0 {
                    bm / views
                } else {
                    0.0
                }
            }
            SortKey::Date => illust
                .create_date
                .map(|d| d.timestamp() as f64)
                .unwrap_or(0.0),
            SortKey::Sanity => illust.sanity_level.unwrap_or(0) as f64,
            SortKey::Pixel => {
                let w = illust.width.unwrap_or(0) as f64;
                let h = illust.height.unwrap_or(0) as f64;
                w * h
            }
            SortKey::Pages => illust.page_count.unwrap_or(1) as f64,
            SortKey::Srgb | SortKey::Hue => 0.0,
        }
    }
}

// ── Property Tests ──────────────────────────────────────────────────────

proptest! {
    #[test]
    fn proptest_filter_idempotent(
        ids in prop::collection::vec(arb_illust_id(), 0..100),
        r18 in arb_tristate(),
        ai in arb_tristate(),
        bm in arb_tristate(),
        land in arb_tristate(),
        vis in arb_tristate(),
        rest in arb_tristate(),
        miss in arb_tristate(),
        tags in prop::collection::vec("[a-z_]{1,10}", 0..5),
    ) {
        let illusts: Vec<Illustration> = ids.into_iter().map(mock_illust).collect();
        let filter = make_filter_state(
            String::new(),
            OptionTristates {
                r18,
                ai,
                bm,
                land,
                vis,
                rest,
                miss,
            },
            tags,
        );
        let once = filter.apply(&illusts, &Default::default());
        let twice = filter.apply(&once, &Default::default());
        prop_assert_eq!(once.len(), twice.len());
        for (a, b) in once.iter().zip(twice.iter()) {
            prop_assert_eq!(a.id, b.id);
        }
    }

    #[test]
    fn proptest_filter_monotone(
        ids in prop::collection::vec(arb_illust_id(), 1..100),
        extra_key in arb_option_filter_key(),
        extra_value in arb_tristate(),
    ) {
        let illusts: Vec<Illustration> = ids.into_iter().map(mock_illust).collect();
        // Base: all options Indeterminate (no filtering), no tags
        let base_filter = make_filter_state(
            String::new(),
            OptionTristates {
                r18: Tristate::Indeterminate,
                ai: Tristate::Indeterminate,
                bm: Tristate::Indeterminate,
                land: Tristate::Indeterminate,
                vis: Tristate::Indeterminate,
                rest: Tristate::Indeterminate,
                miss: Tristate::Indeterminate,
            },
            vec![],
        );
        let base_result = base_filter.apply(&illusts, &Default::default());

        // Add one filter on top
        let mut more_filter = base_filter;
        more_filter.options.set(extra_key, extra_value);
        let more_result = more_filter.apply(&illusts, &Default::default());

        // more_result must be a subset of base_result (base has no filters = all pass)
        let base_ids: std::collections::HashSet<u64> = base_result.iter().map(|i| i.id).collect();
        for item in &more_result {
            prop_assert!(base_ids.contains(&item.id),
                "item {} not in base result", item.id);
        }
    }

    #[test]
    fn proptest_sort_stable(
        ids in prop::collection::vec(arb_illust_id(), 1..200),
        active in prop::collection::vec((arb_sort_key(), arb_sort_direction()), 0..4),
    ) {
        let illusts: Vec<Illustration> = ids.into_iter().map(mock_illust).collect();
        let sort_state = SortState {
            multi_sort: active.len() > 1,
            active,
            color: None,
            color_level: 0,
        };

        let mut sorted = illusts.clone();
        sort_state.sort(&mut sorted, &Default::default());

        prop_assert_eq!(sorted.len(), illusts.len());

        if let Some((key, dir)) = sort_state.active.first() {
            if *dir != SortDirection::Undefined {
                for w in sorted.windows(2) {
                    let va = key.extract_value(&w[0]);
                    let vb = key.extract_value(&w[1]);
                    match dir {
                        SortDirection::Down => prop_assert!(va >= vb,
                            "expected {:?} >= {:?}", va, vb),
                        SortDirection::Up => prop_assert!(va <= vb,
                            "expected {:?} <= {:?}", va, vb),
                        SortDirection::Undefined => {}
                    }
                }
            }
        }
    }

    #[test]
    fn proptest_sort_idempotent(
        ids in prop::collection::vec(arb_illust_id(), 0..100),
        active in prop::collection::vec((arb_sort_key(), arb_sort_direction()), 0..4),
    ) {
        let illusts: Vec<Illustration> = ids.into_iter().map(mock_illust).collect();
        let sort_state = SortState {
            multi_sort: active.len() > 1,
            active,
            color: None,
            color_level: 0,
        };

        let mut once = illusts.clone();
        sort_state.sort(&mut once, &Default::default());

        let mut twice = once.clone();
        sort_state.sort(&mut twice, &Default::default());

        prop_assert_eq!(once.len(), twice.len());
        for (a, b) in once.iter().zip(twice.iter()) {
            prop_assert_eq!(a.id, b.id);
        }
    }

    #[test]
    fn proptest_tristate_cycle_3(state in arb_tristate()) {
        let cycled = state.cycle().cycle().cycle();
        prop_assert_eq!(state, cycled);
    }

    #[test]
    fn proptest_sort_direction_cycle_nullable(dir in arb_sort_direction()) {
        let cycled = dir.cycle(true).cycle(true).cycle(true);
        prop_assert_eq!(dir, cycled);
    }

    #[test]
    fn proptest_sort_direction_cycle_non_nullable(dir in arb_sort_direction()) {
        // Non-nullable: 2 cycles from Down/Up returns to same (2-cycle).
        // Undefined enters the cycle at Down.
        if dir == SortDirection::Down || dir == SortDirection::Up {
            let cycled = dir.cycle(false).cycle(false);
            prop_assert_eq!(dir, cycled);
        } else {
            // Undefined → Down in one step
            prop_assert_eq!(SortDirection::Down, dir.cycle(false));
        }
    }

    #[test]
    fn proptest_multi_sort_prefix_dominant(
        ids in prop::collection::vec(arb_illust_id(), 2..50),
        key1 in arb_sort_key(),
        key2 in arb_sort_key(),
        dir1 in arb_sort_direction(),
        dir2 in arb_sort_direction(),
    ) {
        let illusts: Vec<Illustration> = ids.into_iter().map(mock_illust).collect();
        let sort_state = SortState {
            multi_sort: true,
            active: vec![(key1, dir1), (key2, dir2)],
            color: None,
            color_level: 0,
        };

        let mut sorted = illusts;
        sort_state.sort(&mut sorted, &Default::default());

        if dir1 != SortDirection::Undefined && sorted.len() >= 2 {
            for w in sorted.windows(2) {
                let va = key1.extract_value(&w[0]);
                let vb = key1.extract_value(&w[1]);
                match dir1 {
                    SortDirection::Down => prop_assert!(va >= vb),
                    SortDirection::Up => prop_assert!(va <= vb),
                    SortDirection::Undefined => {}
                }
            }
        }
    }
}
