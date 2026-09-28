use unshit::core::cell_grid::CellGrid;

#[test]
fn cell_metrics_are_isolated_between_test_threads() {
    CellGrid::publish_cell_metrics(10.0, 24.0);
    let other_metrics = std::thread::spawn(|| {
        CellGrid::publish_cell_metrics(9.5, 18.0);
        (CellGrid::global_cell_w(), CellGrid::global_cell_h())
    })
    .join()
    .unwrap();

    assert_eq!(other_metrics, (9.5, 18.0));
    assert_eq!(
        (CellGrid::global_cell_w(), CellGrid::global_cell_h()),
        (10.0, 24.0)
    );
    assert_eq!(
        std::thread::spawn(|| (CellGrid::global_cell_w(), CellGrid::global_cell_h()))
            .join()
            .unwrap(),
        (0.0, 0.0),
        "a fresh test thread must not inherit another test's metrics"
    );
}
