mod common;

use common::*;
use vector_flow_core::node::{NodeDef, ParamValue};
use vector_flow_core::types::{DataType, NodeId};

#[test]
fn empty_graph_produces_empty_scene() {
    let mut t = SceneTest::new();
    let scene = t.collect_all();
    assert_shape_count(&scene, 0);
    assert_image_count(&scene, 0);
    assert_text_count(&scene, 0);
}

#[test]
fn circle_produces_shape() {
    let mut t = SceneTest::new();
    // A lone Circle outputs a Path, which collect_scene promotes to a shape.
    let _circle = add_node(&mut t, NodeDef::circle);
    let scene = t.collect_all();
    assert_shape_count(&scene, 1);
}

#[test]
fn circle_with_fill() {
    let mut t = SceneTest::new();
    let circle = add_node(&mut t, NodeDef::circle);
    let fill = add_node(&mut t, NodeDef::set_fill);
    let output = t.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "output".into(),
        DataType::Any,
    ));

    // Set fill color to red
    t.graph.node_mut(fill).unwrap().inputs[1].default_value =
        Some(ParamValue::Color([1.0, 0.0, 0.0, 1.0]));

    connect(&mut t, circle, 0, fill, 0);
    connect(&mut t, fill, 0, output, 0);

    // GraphOutput filter: only the GraphOutput node's output is collected (1 shape).
    let scene = t.collect();
    assert_shape_count(&scene, 1);

    let shape = &scene.shapes[0].shape;
    let color = shape.fill.expect("expected fill color");
    assert!((color.r - 1.0).abs() < 1e-5, "fill red channel should be 1.0");
    assert!(color.g.abs() < 1e-5, "fill green channel should be 0.0");
    assert!(color.b.abs() < 1e-5, "fill blue channel should be 0.0");
}

#[test]
fn graph_output_filters_visibility() {
    let mut t = SceneTest::new();

    // Chain A: circle → fill → GraphOutput (visible via filter)
    let circle_a = add_node(&mut t, NodeDef::circle);
    let fill_a = add_node(&mut t, NodeDef::set_fill);
    let output = t.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));
    connect(&mut t, circle_a, 0, fill_a, 0);
    connect(&mut t, fill_a, 0, output, 0);

    // Chain B: circle → fill (NOT connected to GraphOutput)
    let circle_b = add_node(&mut t, NodeDef::circle);
    let fill_b = add_node(&mut t, NodeDef::set_fill);
    connect(&mut t, circle_b, 0, fill_b, 0);

    // collect() uses GraphOutput filter — only the GraphOutput node's output
    let scene = t.collect();
    assert_shape_count(&scene, 1);

    // collect_all() sees all 5 nodes' outputs:
    // circle_a(Path) + fill_a(Shape) + output(Shape) + circle_b(Path) + fill_b(Shape) = 5
    let scene_all = t.collect_all();
    assert_shape_count(&scene_all, 5);
}

#[test]
fn stroke_applied() {
    let mut t = SceneTest::new();
    let circle = add_node(&mut t, NodeDef::circle);
    let stroke = add_node(&mut t, NodeDef::set_stroke);
    let output = t.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));
    connect(&mut t, circle, 0, stroke, 0);
    connect(&mut t, stroke, 0, output, 0);

    let scene = t.collect();
    assert_shape_count(&scene, 1);

    let shape = &scene.shapes[0].shape;
    assert!(shape.stroke.is_some(), "expected stroke to be set");
}

#[test]
fn translate_moves_shape() {
    let mut t = SceneTest::new();
    let circle = add_node(&mut t, NodeDef::circle);
    let fill = add_node(&mut t, NodeDef::set_fill);
    let translate = add_node(&mut t, NodeDef::translate);
    let output = t.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));

    // Set translation offset to (100, 200)
    t.graph.node_mut(translate).unwrap().inputs[1].default_value =
        Some(ParamValue::Vec2([100.0, 200.0]));

    connect(&mut t, circle, 0, fill, 0);
    connect(&mut t, fill, 0, translate, 0);
    connect(&mut t, translate, 0, output, 0);

    let scene = t.collect();
    assert_shape_count(&scene, 1);

    let transform = scene.shapes[0].shape.transform;
    let tx = transform.translation.x;
    let ty = transform.translation.y;
    assert!(
        (tx - 100.0).abs() < 1e-3 && (ty - 200.0).abs() < 1e-3,
        "expected translation (100, 200), got ({}, {})",
        tx,
        ty
    );
}

#[test]
fn prepare_produces_geometry() {
    let mut t = SceneTest::new();
    let circle = add_node(&mut t, NodeDef::circle);
    let fill = add_node(&mut t, NodeDef::set_fill);
    let output = t.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));
    connect(&mut t, circle, 0, fill, 0);
    connect(&mut t, fill, 0, output, 0);

    let prepared = t.prepare();
    assert!(!prepared.vertices.is_empty(), "expected non-empty vertices");
    assert!(!prepared.indices.is_empty(), "expected non-empty indices");
    assert!(!prepared.batches.is_empty(), "expected non-empty batches");
}

#[test]
fn parameter_change_produces_different_output() {
    // Evaluate a circle with radius 100
    let mut t1 = SceneTest::new();
    let circle1 = add_node(&mut t1, NodeDef::circle);
    let fill1 = add_node(&mut t1, NodeDef::set_fill);
    let output1 = t1.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));
    connect(&mut t1, circle1, 0, fill1, 0);
    connect(&mut t1, fill1, 0, output1, 0);
    let prepared1 = t1.prepare();

    // Evaluate a circle with radius 10
    let mut t2 = SceneTest::new();
    let circle2 = add_node(&mut t2, NodeDef::circle);
    t2.graph.node_mut(circle2).unwrap().inputs[0].default_value =
        Some(ParamValue::Float(10.0));
    let fill2 = add_node(&mut t2, NodeDef::set_fill);
    let output2 = t2.graph.add_node(NodeDef::graph_output(
        NodeId(0),
        "out".into(),
        DataType::Any,
    ));
    connect(&mut t2, circle2, 0, fill2, 0);
    connect(&mut t2, fill2, 0, output2, 0);
    let prepared2 = t2.prepare();

    // Both produce geometry
    assert!(!prepared1.vertices.is_empty());
    assert!(!prepared2.vertices.is_empty());

    // Vertex positions must differ (radius 100 vs 10)
    let any_differ = prepared1
        .vertices
        .iter()
        .zip(prepared2.vertices.iter())
        .any(|(a, b)| {
            (a.position[0] - b.position[0]).abs() > 1e-3
                || (a.position[1] - b.position[1]).abs() > 1e-3
        });
    assert!(
        any_differ || prepared1.vertices.len() != prepared2.vertices.len(),
        "expected different geometry for different radii"
    );
}
