use criterion::{criterion_group, criterion_main, Criterion};
use gutenberg_store::{bench, Store};

/// Budget: "Harrisburg" across 100k articles in under 50 ms (S3.6).
fn search_100k(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("gutenberg.db")).unwrap();
    let corpus = bench::search_corpus(&store, 100_000);
    store.insert_articles(&corpus).unwrap();

    c.bench_function("search_harrisburg_100k", |b| {
        b.iter(|| {
            let found = store.search("Harrisburg", 100_000).unwrap();
            assert_eq!(found.len(), bench::HARRISBURG);
        })
    });
    // What the list asks for as you type: the 500 newest matches. A
    // common word, and a single letter, the worst first keystroke.
    for typed in ["hospital", "h"] {
        c.bench_function(&format!("search_{typed}_100k"), |b| {
            b.iter(|| store.search(typed, 500).unwrap())
        });
    }
}

criterion_group!(benches, search_100k);
criterion_main!(benches);
