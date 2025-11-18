use std::sync::Arc;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use merc_lts::read_aut;
use merc_ltsgraph_lib::GraphLayout;
use merc_ltsgraph_lib::SkiaRenderer;
use merc_ltsgraph_lib::Viewer;
use tiny_skia::Pixmap;
use tiny_skia::PixmapMut;

/// Render the alternating bit protocol with some settings.
pub fn criterion_benchmark_viewer(c: &mut Criterion) {
    let file = include_str!("../../../../../examples/lts/abp.aut");
    let lts = Arc::new(read_aut(file.as_bytes(), vec![]).unwrap());

    let viewer = Viewer::new(lts.clone());
    let mut renderer = SkiaRenderer::new(lts);

    let mut pixel_buffer = Pixmap::new(800, 600).unwrap();

    c.bench_function("ltsgraph viewer", |bencher| {
        bencher.iter(|| {
            renderer.render(
                &mut PixmapMut::from_bytes(pixel_buffer.data_mut(), 800, 600).unwrap(),
                &viewer,
                true,
                5.0,
                0.0,
                0.0,
                800,
                600,
                1.0,
                14.0,
            );
        });
    });

    c.bench_function("ltsgraph viewer (no text)", |bencher| {
        bencher.iter(|| {
            renderer.render(
                &mut PixmapMut::from_bytes(pixel_buffer.data_mut(), 800, 600).unwrap(),
                &viewer,
                false,
                5.0,
                0.0,
                0.0,
                800,
                600,
                1.0,
                14.0,
            );
        });
    });
}

/// Perform layouting the alternating bit protocol with some settings.
pub fn criterion_benchmark_layout(c: &mut Criterion) {
    let file = include_str!("../../../../../examples/lts/abp.aut");
    let lts = Arc::new(read_aut(file.as_bytes(), vec![]).unwrap());

    let mut layout = GraphLayout::new(lts);

    c.bench_function("ltsgraph layout", |bencher| {
        bencher.iter(|| {
            layout.update(5.0, 1.0, 0.01);
        });
    });
}

criterion_group!(benches, criterion_benchmark_viewer, criterion_benchmark_layout,);
criterion_main!(benches);
