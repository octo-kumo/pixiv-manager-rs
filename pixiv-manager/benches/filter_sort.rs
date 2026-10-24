use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use pixiv_manager::app::filter::{FilterState, OptionFilters, Tristate};
use pixiv_manager::app::sort::{SortDirection, SortKey, SortState};
use pixiv_manager::test_util::mock_illust;
use pixivj_rs::model::*;

fn bench_filter(c: &mut Criterion) {
    let filter = FilterState {
        search_text: "cat".to_string(),
        options: OptionFilters {
            r18: Tristate::Deselected,
            ai: Tristate::Deselected,
            ..Default::default()
        },
        selected_tags: vec!["cat".into()],
        ..Default::default()
    };

    let mut group = c.benchmark_group("filter");
    for size in [100usize, 1000] {
        let illusts: Vec<Illustration> = (1..=size as u64).map(mock_illust).collect();
        group.bench_with_input(BenchmarkId::from_parameter(size), &illusts, |b, data| {
            b.iter(|| filter.apply(black_box(data), &Default::default()))
        });
    }
    group.finish();
}

fn bench_sort(c: &mut Criterion) {
    let sort = SortState {
        multi_sort: true,
        active: vec![
            (SortKey::Views, SortDirection::Down),
            (SortKey::Bookmarks, SortDirection::Down),
            (SortKey::Id, SortDirection::Up),
        ],
        color: None,
        color_level: 0,
        custom_color_text: None,
    };

    let mut group = c.benchmark_group("sort");
    for size in [100usize, 1000] {
        let illusts: Vec<Illustration> = (1..=size as u64).map(mock_illust).collect();
        group.bench_with_input(BenchmarkId::from_parameter(size), &illusts, |b, data| {
            b.iter(|| {
                let mut data = data.clone();
                sort.sort(black_box(&mut data), &Default::default());
                data
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_filter, bench_sort);
criterion_main!(benches);
