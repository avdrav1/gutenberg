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

criterion_group!(benches, list_view_10k);
criterion_main!(benches);
