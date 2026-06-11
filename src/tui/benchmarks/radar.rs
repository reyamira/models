use std::collections::HashMap;
use std::f64::consts::PI;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{
        canvas::{Canvas, Line as CanvasLine},
        Block, Borders,
    },
    Frame,
};

use crate::benchmarks::multi::{format_metric_value, metric_indices_in_group, radar_groups};
use crate::benchmarks::schema::{MetricDef, ModelRow, SourceFile};

/// Maximum number of axes a single radar group can render.
const MAX_AXES: usize = 6;

/// Compute N spoke angles starting at top (-PI/2), going clockwise.
pub fn spoke_angles(n: usize) -> Vec<f64> {
    let step = 2.0 * PI / n as f64;
    (0..n).map(|i| -PI / 2.0 + step * i as f64).collect()
}

/// Given center, radius, spoke angles, and normalized values (0-1), compute vertex positions.
pub fn polygon_vertices(
    cx: f64,
    cy: f64,
    radius: f64,
    angles: &[f64],
    values: &[f64],
) -> Vec<(f64, f64)> {
    angles
        .iter()
        .zip(values.iter())
        .map(|(&angle, &val)| {
            let r = radius * val;
            (cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}

/// An axis on the radar chart, derived from a single source metric.
struct RadarAxis<'a> {
    /// Index into `file.metrics`.
    metric_idx: usize,
    metric: &'a MetricDef,
}

/// Build the radar axes for the active group: the first [`MAX_AXES`]
/// `higher_is_better` metrics of the group selected by `radar_group`.
fn axes_for_group(file: &SourceFile, radar_group: usize) -> Vec<RadarAxis<'_>> {
    let groups = radar_groups(file);
    let Some(group) = groups.get(radar_group) else {
        return Vec::new();
    };
    metric_indices_in_group(file, group)
        .into_iter()
        .filter_map(|mi| file.metrics.get(mi).map(|m| (mi, m)))
        .filter(|(_, m)| m.higher_is_better)
        .take(MAX_AXES)
        .map(|(metric_idx, metric)| RadarAxis { metric_idx, metric })
        .collect()
}

/// The active group name (used in the panel title), or `"—"` when none.
fn active_group_label(file: &SourceFile, radar_group: usize) -> String {
    radar_groups(file)
        .get(radar_group)
        .cloned()
        .unwrap_or_else(|| "\u{2014}".to_string())
}

/// Draw the radar chart in the given area.
pub fn draw_radar(f: &mut Frame, area: Rect, app: &crate::tui::app::App) {
    let Some(file) = app.active_benchmark_file() else {
        let block = Block::default().borders(Borders::ALL).title(" Radar ");
        f.render_widget(block, area);
        return;
    };

    let radar_group = app.benchmarks_app.radar_group;
    let group_label = active_group_label(file, radar_group);
    let axes = axes_for_group(file, radar_group);

    // Empty guard: need at least 3 axes and at least one selection
    if axes.len() < 3 || app.selections.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Radar [{group_label}] "));
        f.render_widget(block, area);
        return;
    }

    let angles = spoke_angles(axes.len());
    let radius: f64 = 45.0;

    // Helper: extract the raw value of an axis metric for a model.
    let axis_value = |axis: &RadarAxis, model: &ModelRow| -> Option<f64> {
        model.scores.get(&axis.metric.id).map(|cell| cell.value)
    };

    // Pre-compute max values for normalization (MUST be outside paint() closure),
    // keyed by metric index so duplicate labels don't collide.
    let mut max_values: HashMap<usize, f64> = HashMap::new();
    for model in file.models.iter() {
        for ax in &axes {
            if let Some(v) = axis_value(ax, model) {
                let current = max_values.entry(ax.metric_idx).or_insert(0.0);
                if v > *current {
                    *current = v;
                }
            }
        }
    }

    // Pre-compute axis line endpoints and labels
    let axis_lines: Vec<(f64, f64)> = angles
        .iter()
        .map(|&a| (radius * a.cos(), radius * a.sin()))
        .collect();

    let label_offset = 56.0;
    // Each axis label can be multiple lines (for wrapping long names)
    let axis_labels: Vec<Vec<(f64, f64, String)>> = angles
        .iter()
        .zip(axes.iter())
        .map(|(&a, ax)| {
            let lx = label_offset * a.cos();
            let ly = label_offset * a.sin();
            let full = ax.metric.label.clone();
            // Wrap labels longer than 16 chars at the last space before the limit
            if full.len() <= 16 {
                vec![(lx, ly, full)]
            } else if let Some(split) = full[..16].rfind(' ') {
                let line1 = full[..split].to_string();
                let line2 = full[split + 1..].to_string();
                // Offset second line down by ~4 canvas units
                vec![(lx, ly, line1), (lx, ly - 4.0, line2)]
            } else {
                vec![(lx, ly, full)]
            }
        })
        .collect();

    // Pre-compute polygon data and legend entries for each selected model
    let mut polygons: Vec<(Vec<(f64, f64)>, Color)> = Vec::new();
    let mut legend_entries: Vec<(String, Color, Vec<Option<f64>>)> = Vec::new();

    for (sel_idx, &model_idx) in app.selections.iter().enumerate() {
        if let Some(model) = file.models.get(model_idx) {
            let color = super::render::compare_colors(sel_idx);

            let raw_values: Vec<Option<f64>> =
                axes.iter().map(|ax| axis_value(ax, model)).collect();

            // Normalize values for this model
            let values: Vec<f64> = axes
                .iter()
                .map(|ax| {
                    let raw = axis_value(ax, model).unwrap_or(0.0);
                    let max = max_values.get(&ax.metric_idx).copied().unwrap_or(1.0);
                    if max > 0.0 {
                        raw / max
                    } else {
                        0.0
                    }
                })
                .collect();

            let vertices = polygon_vertices(0.0, 0.0, radius, &angles, &values);
            polygons.push((vertices, color));
            legend_entries.push((model.display_name.clone(), color, raw_values));
        }
    }

    // Compute average polygon (baseline reference) — always uses all models
    let avg_values: Vec<f64> = axes
        .iter()
        .map(|ax| {
            let (sum, count) = file.models.iter().fold((0.0, 0usize), |(s, c), model| {
                if let Some(v) = axis_value(ax, model) {
                    (s + v, c + 1)
                } else {
                    (s, c)
                }
            });
            if count > 0 {
                let raw_avg = sum / count as f64;
                let max = max_values.get(&ax.metric_idx).copied().unwrap_or(1.0);
                if max > 0.0 {
                    raw_avg / max
                } else {
                    0.0
                }
            } else {
                0.0
            }
        })
        .collect();

    // Raw average values for labels
    let avg_raw_values: Vec<f64> = axes
        .iter()
        .map(|ax| {
            let (sum, count) = file.models.iter().fold((0.0, 0usize), |(s, c), model| {
                if let Some(v) = axis_value(ax, model) {
                    (s + v, c + 1)
                } else {
                    (s, c)
                }
            });
            if count > 0 {
                sum / count as f64
            } else {
                0.0
            }
        })
        .collect();

    let avg_vertices = polygon_vertices(0.0, 0.0, radius, &angles, &avg_values);

    // avg_raw_values used in legend table below

    let compare_focused = app.benchmarks_app.focus == super::app::BenchmarkFocus::Compare;
    let radar_border = if compare_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    // Split area: canvas on top, legend box at bottom (+1 for avg row)
    let legend_height = (legend_entries.len() as u16 + 3).min(area.height / 3); // +2 borders +1 avg
    let (canvas_area, legend_area) = if !legend_entries.is_empty() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(legend_height)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(radar_border))
                .title(format!(" Radar [{group_label}] ")),
        )
        .x_bounds([-65.0, 65.0])
        .y_bounds([-62.0, 62.0])
        .marker(ratatui::symbols::Marker::Braille)
        .paint(move |ctx| {
            // Draw axis spokes
            for &(ex, ey) in &axis_lines {
                ctx.draw(&CanvasLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: ex,
                    y2: ey,
                    color: Color::DarkGray,
                });
            }

            // Draw average baseline polygon
            let n_avg = avg_vertices.len();
            for i in 0..n_avg {
                let (x1, y1) = avg_vertices[i];
                let (x2, y2) = avg_vertices[(i + 1) % n_avg];
                ctx.draw(&CanvasLine {
                    x1,
                    y1,
                    x2,
                    y2,
                    color: Color::Indexed(242),
                });
            }

            // Draw axis labels (may be multi-line for long names)
            for lines in &axis_labels {
                for (lx, ly, label) in lines {
                    ctx.print(
                        *lx,
                        *ly,
                        Span::styled(label.clone(), Style::default().fg(Color::Gray)),
                    );
                }
            }

            // Draw model polygons
            for (vertices, color) in &polygons {
                let n = vertices.len();
                for i in 0..n {
                    let (x1, y1) = vertices[i];
                    let (x2, y2) = vertices[(i + 1) % n];
                    ctx.draw(&CanvasLine {
                        x1,
                        y1,
                        x2,
                        y2,
                        color: *color,
                    });
                }
            }
        });

    f.render_widget(canvas, canvas_area);

    // Legend table below the radar chart
    if let Some(leg_area) = legend_area {
        use crate::tui::widgets::comparison_legend::{ComparisonLegend, LegendEntry, LegendMetric};

        let fmt_axis_val = |v: Option<f64>, axis: &RadarAxis| -> String {
            match v {
                Some(val) => format_metric_value(axis.metric.kind, val),
                None => "\u{2014}".into(),
            }
        };

        // Short per-axis labels for the legend column headers: truncate the metric
        // label to keep the table compact.
        let short_label = |label: &str| -> String { label.chars().take(5).collect::<String>() };

        let mut entries: Vec<LegendEntry> = legend_entries
            .iter()
            .map(|(name, color, raw_vals)| {
                let metrics: Vec<LegendMetric> = axes
                    .iter()
                    .enumerate()
                    .map(|(i, ax)| {
                        LegendMetric::new(
                            short_label(&ax.metric.label),
                            fmt_axis_val(raw_vals.get(i).copied().flatten(), ax),
                        )
                    })
                    .collect();
                LegendEntry::new(name.clone(), *color).metrics(metrics)
            })
            .collect();

        // Average row
        let avg_color = Color::Indexed(250);
        let avg_style = Style::default().fg(avg_color);
        let avg_metrics: Vec<LegendMetric> = axes
            .iter()
            .enumerate()
            .map(|(i, ax)| {
                LegendMetric::new(
                    short_label(&ax.metric.label),
                    fmt_axis_val(Some(avg_raw_values[i]), ax),
                )
                .value_style(avg_style)
            })
            .collect();
        entries.push(
            LegendEntry::new("Avg", avg_color)
                .marker("\u{2505} ")
                .metrics(avg_metrics),
        );

        ComparisonLegend::new(entries)
            .value_width(6)
            .render(f, leg_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::{MetricKind, ReasoningStatus, ScoreCell, SourceMeta};
    use std::collections::BTreeMap;

    #[test]
    fn spoke_angles_start_at_top() {
        let angles = spoke_angles(6);
        assert!((angles[0] - (-PI / 2.0)).abs() < 1e-10);
    }

    #[test]
    fn spoke_angles_evenly_spaced() {
        let angles = spoke_angles(4);
        let expected_gap = 2.0 * PI / 4.0;
        for i in 0..3 {
            let gap = angles[i + 1] - angles[i];
            assert!((gap - expected_gap).abs() < 1e-10);
        }
    }

    #[test]
    fn polygon_vertex_at_max_reaches_radius() {
        let angles = spoke_angles(4);
        let values = vec![1.0, 0.5, 1.0, 0.5];
        let vertices = polygon_vertices(50.0, 50.0, 40.0, &angles, &values);
        assert!((vertices[0].0 - 50.0).abs() < 1e-10);
        assert!((vertices[0].1 - 10.0).abs() < 1e-10);
    }

    #[test]
    fn polygon_vertex_at_zero_stays_at_center() {
        let angles = spoke_angles(4);
        let values = vec![0.0, 0.0, 0.0, 0.0];
        let vertices = polygon_vertices(50.0, 50.0, 40.0, &angles, &values);
        for &(x, y) in &vertices {
            assert!((x - 50.0).abs() < 1e-10);
            assert!((y - 50.0).abs() < 1e-10);
        }
    }

    fn meta() -> SourceMeta {
        SourceMeta {
            id: "test".into(),
            name: "Test".into(),
            url: "https://example.com".into(),
            fetched_at: "2026-06-10T00:00:00+00:00".into(),
            verified: true,
        }
    }

    fn metric(id: &str, kind: MetricKind, group: &str, hib: bool) -> MetricDef {
        MetricDef {
            id: id.into(),
            label: id.to_uppercase(),
            kind,
            group: group.into(),
            higher_is_better: hib,
            last_updated: None,
            description: None,
        }
    }

    fn model(id: &str, scores: &[(&str, f64)]) -> ModelRow {
        let mut score_map = BTreeMap::new();
        for (mid, v) in scores {
            score_map.insert(
                (*mid).to_string(),
                ScoreCell {
                    value: *v,
                    date: None,
                    ci: None,
                },
            );
        }
        ModelRow {
            id: id.into(),
            name: id.into(),
            display_name: id.into(),
            creator: "openai".into(),
            creator_name: "OpenAI".into(),
            release_date: None,
            reasoning_status: ReasoningStatus::None,
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: None,
            scores: score_map,
        }
    }

    /// A file with one radar-eligible group ("Indexes": 3 higher-is-better
    /// metrics) and one non-eligible group ("Pricing": lower-is-better).
    fn sample_file() -> SourceFile {
        SourceFile {
            source: meta(),
            metrics: vec![
                metric("intelligence_index", MetricKind::Index, "Indexes", true),
                metric("coding_index", MetricKind::Index, "Indexes", true),
                metric("math_index", MetricKind::Index, "Indexes", true),
                metric("price_input", MetricKind::UsdPerMTok, "Pricing", false),
            ],
            models: vec![model(
                "alpha",
                &[
                    ("intelligence_index", 70.0),
                    ("coding_index", 60.0),
                    ("math_index", 50.0),
                ],
            )],
        }
    }

    #[test]
    fn axes_for_group_filters_to_higher_is_better() {
        let file = sample_file();
        // Group 0 = "Indexes" (Pricing is not radar-eligible, so it's not group 0).
        let axes = axes_for_group(&file, 0);
        assert_eq!(axes.len(), 3);
        assert_eq!(axes[0].metric_idx, 0);
        assert_eq!(axes[2].metric_idx, 2);
    }

    #[test]
    fn axes_for_group_caps_at_max_axes() {
        let mut file = sample_file();
        // Add 5 more higher-is-better metrics to "Indexes" (total 8) and confirm
        // the axis count caps at MAX_AXES.
        for i in 0..5 {
            file.metrics.push(metric(
                &format!("extra_{i}"),
                MetricKind::Index,
                "Indexes",
                true,
            ));
        }
        let axes = axes_for_group(&file, 0);
        assert_eq!(axes.len(), MAX_AXES);
    }

    #[test]
    fn axes_for_group_out_of_range_is_empty() {
        let file = sample_file();
        assert!(axes_for_group(&file, 99).is_empty());
    }

    #[test]
    fn active_group_label_resolves() {
        let file = sample_file();
        assert_eq!(active_group_label(&file, 0), "Indexes");
        assert_eq!(active_group_label(&file, 99), "\u{2014}");
    }
}
