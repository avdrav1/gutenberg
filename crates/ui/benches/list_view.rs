use criterion::{criterion_group, criterion_main, Criterion};
use gutenberg_ui::bench::ListFixture;

/// Budget: 10k articles scroll at 60 fps (S3.1). Building the list view is
/// the part of a frame that grows with the article count; it must stay a
/// small slice of 16.7 ms. Rendering is measured in the app itself with
/// GUTENBERG_SCROLL_BENCH.
fn list_view_10k(c: &mut Criterion) {
    // A 4K tile: 2136 px of list.
    let mut fixture = ListFixture::new(10_000, 2136.0);
    c.bench_function("list_view_10k", |b| b.iter(|| fixture.scroll_and_view()));
}

/// The same in the interesting order, where each row on screen also asks
/// what lifted it.
fn list_view_10k_interesting(c: &mut Criterion) {
    let mut fixture = ListFixture::interesting(10_000, 2136.0);
    c.bench_function("list_view_10k_interesting", |b| {
        b.iter(|| fixture.scroll_and_view())
    });
}

/// A reload newest first, to compare with the interesting order's.
fn reload_10k(c: &mut Criterion) {
    let mut fixture = ListFixture::new(10_000, 2136.0);
    c.bench_function("reload_10k", |b| b.iter(|| fixture.reload()));
}

/// A reload in the interesting order: the list read, the model learned
/// from 4k signals and 10k rows scored and sorted. It runs when a refresh
/// stores articles, not per frame.
fn reload_10k_interesting(c: &mut Criterion) {
    let mut fixture = ListFixture::interesting(10_000, 2136.0);
    c.bench_function("reload_10k_interesting", |b| b.iter(|| fixture.reload()));
}

criterion_group!(
    benches,
    list_view_10k,
    list_view_10k_interesting,
    reload_10k,
    reload_10k_interesting
);
criterion_main!(benches);
