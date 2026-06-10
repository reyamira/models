use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::render::compare_colors;
use crate::benchmarks::multi::format_metric_value;
use crate::benchmarks::schema::{MetricDef, ModelRow, SourceFile};
use crate::formatting::format_tokens;
use crate::tui::app::App;
use crate::tui::widgets::scrollable_panel::ScrollablePanel;

// ── H2H comparison table ────────────────────────────────────────────────────

/// Extract the raw value of metric `metric_idx` for `model`, or `None` when the
/// model has no score for that metric.
fn metric_value(file: &SourceFile, model: &ModelRow, metric_idx: usize) -> Option<f64> {
    let metric = file.metrics.get(metric_idx)?;
    model.scores.get(&metric.id).map(|cell| cell.value)
}

/// Rank extracted values: 1 = best, None for missing data.
fn rank_values(values: &[Option<f64>], higher_is_better: bool) -> Vec<Option<u32>> {
    let mut indexed: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|val| (i, val)))
        .collect();

    if higher_is_better {
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    }

    let mut ranks = vec![None; values.len()];
    for (rank, (idx, _)) in indexed.iter().enumerate() {
        ranks[*idx] = Some(rank as u32 + 1);
    }
    ranks
}

pub(super) fn draw_h2h_table_generic(f: &mut Frame, area: Rect, app: &App) {
    let Some(file) = app.active_benchmark_file() else {
        return;
    };
    let selections = &app.selections;

    if selections.len() < 2 {
        return;
    }

    let is_focused = app.benchmarks_app.focus == super::app::BenchmarkFocus::Compare;

    // Use a temporary inner rect to check minimum dimensions before building lines.
    // We subtract 2 for borders to approximate inner dimensions.
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    if inner_w < 20 || inner_h < 3 {
        return;
    }

    let label_w = 14_u16;
    let num_models = selections.len();
    let available = inner_w.saturating_sub(label_w);
    let col_w = (available as usize / num_models).max(10);
    let total_w = inner_w as usize;

    // Resolve the selected models once (indices into the active file's models).
    let models: Vec<Option<&ModelRow>> =
        selections.iter().map(|&idx| file.models.get(idx)).collect();

    // Header row: model names
    let mut header_spans: Vec<Span> = vec![Span::styled(
        format!("{:<width$}", "", width = label_w as usize),
        Style::default(),
    )];
    for (i, model) in models.iter().enumerate() {
        let name = model.map(|m| m.display_name.as_str()).unwrap_or("?");
        let color = compare_colors(i);
        let truncated = if name.width() > col_w - 1 {
            format!("{:.width$}", name, width = col_w - 2)
        } else {
            name.to_string()
        };
        header_spans.push(Span::styled(
            format!("{:>width$}", truncated, width = col_w),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }

    let mut lines: Vec<Line> = vec![Line::from(header_spans)];

    // Separator
    let sep = "\u{2500}".repeat(total_w);
    lines.push(Line::from(Span::styled(
        sep,
        Style::default().fg(Color::DarkGray),
    )));

    // ── Pre-compute win counts (need them near the top) ──
    let mut win_counts = vec![0u32; num_models];
    for (mi, metric) in file.metrics.iter().enumerate() {
        let values: Vec<Option<f64>> = models
            .iter()
            .map(|m| m.and_then(|model| metric_value(file, model, mi)))
            .collect();
        let ranks = rank_values(&values, metric.higher_is_better);
        for (i, rank) in ranks.iter().enumerate() {
            if *rank == Some(1) {
                win_counts[i] += 1;
            }
        }
    }

    // ── Win count (right under model names) ──
    let mut wins_spans: Vec<Span> = vec![Span::styled(
        format!("{:<width$}", "\u{2605} Wins", width = label_w as usize),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    let max_wins = win_counts.iter().copied().max().unwrap_or(0);
    for (i, &count) in win_counts.iter().enumerate() {
        let color = compare_colors(i);
        let label = if count == max_wins && max_wins > 0 {
            format!("{} \u{2605}", count)
        } else {
            format!("{}", count)
        };
        let style = if count == max_wins && max_wins > 0 {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        wins_spans.push(Span::styled(
            format!("{:>width$}", label, width = col_w),
            style,
        ));
    }
    lines.push(Line::from(wins_spans));

    // ── Model Info section ──
    let info_header = "\u{2500}\u{2500}\u{2500} Model Info \u{2500}".to_string();
    lines.push(Line::from(Span::styled(
        format!("{:<width$}", info_header, width = total_w),
        Style::default().fg(Color::DarkGray),
    )));

    // Helper to render an info row with per-value colors
    let render_info_row = |lines: &mut Vec<Line>, label: &str, values: Vec<(String, Color)>| {
        let mut spans: Vec<Span> = vec![Span::styled(
            format!("{:<width$}", label, width = label_w as usize),
            Style::default().fg(Color::DarkGray),
        )];
        for (val, color) in values.iter() {
            let truncated = if val.width() > col_w - 1 {
                format!("{:.width$}", val, width = col_w - 2)
            } else {
                val.clone()
            };
            spans.push(Span::styled(
                format!("{:>width$}", truncated, width = col_w),
                Style::default().fg(*color),
            ));
        }
        lines.push(Line::from(spans));
    };

    let openness = app.benchmarks_app.creator_openness();

    // Creator
    let creators: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            let name = m
                .map(|model| {
                    if !model.creator_name.is_empty() {
                        model.creator_name.clone()
                    } else {
                        model.creator.clone()
                    }
                })
                .unwrap_or_default();
            (name, Color::White)
        })
        .collect();
    render_info_row(&mut lines, "Creator", creators);

    // Source (Open/Closed) with color — model-level open_weights first, then the
    // creator-openness map as a fallback.
    let sources: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            let open = m.and_then(|model| {
                model
                    .open_weights
                    .or_else(|| openness.get(&model.creator).copied())
            });
            match open {
                Some(true) => ("Open".to_string(), Color::Green),
                Some(false) => ("Closed".to_string(), Color::Red),
                None => ("\u{2014}".to_string(), Color::DarkGray),
            }
        })
        .collect();
    render_info_row(&mut lines, "Source", sources);

    // Region with creator region colors
    let regions: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.map(|model| {
                let region = super::app::CreatorRegion::from_creator(&model.creator);
                (region.label().to_string(), region.color())
            })
            .unwrap_or_default()
        })
        .collect();
    render_info_row(&mut lines, "Region", regions);

    // Type with creator type colors
    let types: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.map(|model| {
                let ct = super::app::CreatorType::from_creator(&model.creator);
                (ct.label().to_string(), ct.color())
            })
            .unwrap_or_default()
        })
        .collect();
    render_info_row(&mut lines, "Type", types);

    // Release date
    let dates: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            let d = m
                .and_then(|model| model.release_date.clone())
                .unwrap_or_else(|| "\u{2014}".to_string());
            (d, Color::White)
        })
        .collect();
    render_info_row(&mut lines, "Released", dates);

    // Reasoning status with color
    let reasoning_vals: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.map(|model| {
                use crate::benchmarks::ReasoningStatus;
                match model.reasoning_status {
                    ReasoningStatus::Reasoning => ("Reasoning".to_string(), Color::Cyan),
                    ReasoningStatus::NonReasoning => ("Non-reasoning".to_string(), Color::DarkGray),
                    ReasoningStatus::Adaptive => ("Adaptive".to_string(), Color::Yellow),
                    ReasoningStatus::None => ("\u{2014}".to_string(), Color::DarkGray),
                }
            })
            .unwrap_or_else(|| ("\u{2014}".to_string(), Color::DarkGray))
        })
        .collect();
    render_info_row(&mut lines, "Reasoning", reasoning_vals);

    // Effort level (if any model has one)
    let effort_vals: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.and_then(|model| model.effort_level.as_ref())
                .map(|lvl| (lvl.clone(), Color::White))
                .unwrap_or_else(|| ("\u{2014}".to_string(), Color::DarkGray))
        })
        .collect();
    if effort_vals.iter().any(|(v, _)| v != "\u{2014}") {
        render_info_row(&mut lines, "Effort", effort_vals);
    }

    // Variant tag (if any model has one)
    let variant_vals: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.and_then(|model| model.variant_tag.as_ref())
                .map(|tag| (tag.clone(), Color::White))
                .unwrap_or_else(|| ("\u{2014}".to_string(), Color::DarkGray))
        })
        .collect();
    if variant_vals.iter().any(|(v, _)| v != "\u{2014}") {
        render_info_row(&mut lines, "Variant", variant_vals);
    }

    // Context window (if any model has one)
    let ctx_vals: Vec<(String, Color)> = models
        .iter()
        .map(|m| {
            m.and_then(|model| model.context_window)
                .map(|v| (format_tokens(v), Color::White))
                .unwrap_or_else(|| ("\u{2014}".to_string(), Color::DarkGray))
        })
        .collect();
    if ctx_vals.iter().any(|(v, _)| v != "\u{2014}") {
        render_info_row(&mut lines, "Context", ctx_vals);
    }

    // ── Metric rows grouped by section ──
    for group in crate::benchmarks::multi::groups_in_order(file) {
        let header = format!("\u{2500}\u{2500}\u{2500} {} \u{2500}", group);
        lines.push(Line::from(Span::styled(
            format!("{:<width$}", header, width = total_w),
            Style::default().fg(Color::DarkGray),
        )));

        for mi in crate::benchmarks::multi::metric_indices_in_group(file, group) {
            let metric: &MetricDef = &file.metrics[mi];
            let values: Vec<Option<f64>> = models
                .iter()
                .map(|m| m.and_then(|model| metric_value(file, model, mi)))
                .collect();
            let ranks = rank_values(&values, metric.higher_is_better);

            let label = truncate_label(&metric.label, label_w as usize);
            let mut row_spans: Vec<Span> = vec![Span::styled(
                format!("{:<width$}", label, width = label_w as usize),
                Style::default().fg(Color::DarkGray),
            )];

            for (i, (val, rank)) in values.iter().zip(ranks.iter()).enumerate() {
                let color = compare_colors(i);
                match val {
                    Some(v) => {
                        let formatted = format_metric_value(metric.kind, *v);
                        if *rank == Some(1) {
                            // Best: value ★
                            let value_and_star = format!("{} \u{2605}", formatted);
                            let padded = format!("{:>width$}", value_and_star, width = col_w);
                            let star_pos = padded.rfind('\u{2605}').unwrap_or(padded.len());
                            row_spans.push(Span::styled(
                                padded[..star_pos].to_string(),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ));
                            row_spans.push(Span::styled(
                                "\u{2605}",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        } else {
                            // Non-best: value in model color, rank in medal colors
                            let rank_num = rank.unwrap_or(0);
                            let suffix = format!(" #{}", rank_num);
                            let rank_color = match rank_num {
                                2 => Color::Indexed(250), // silver
                                3 => Color::Indexed(172), // bronze
                                _ => Color::DarkGray,
                            };

                            let combined = format!("{}{}", formatted, suffix);
                            let padded = format!("{:>width$}", combined, width = col_w);
                            let suffix_start = padded.len().saturating_sub(suffix.len());
                            row_spans.push(Span::styled(
                                padded[..suffix_start].to_string(),
                                Style::default().fg(color),
                            ));
                            row_spans.push(Span::styled(
                                padded[suffix_start..].to_string(),
                                Style::default().fg(rank_color),
                            ));
                        }
                    }
                    None => {
                        row_spans.push(Span::styled(
                            format!("{:>width$}", "\u{2014}", width = col_w),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                }
            }

            lines.push(Line::from(row_spans));
        }
    }

    ScrollablePanel::new(
        "Head-to-Head",
        lines,
        &app.benchmarks_app.h2h_scroll,
        is_focused,
    )
    .with_wrap(false)
    .render(f, area);
}

/// Truncate a metric label to fit the H2H label column (unicode-aware).
fn truncate_label(label: &str, width: usize) -> String {
    // Reserve 1 trailing space inside the column so the label never abuts the
    // value columns.
    let max = width.saturating_sub(1);
    if label.width() <= max {
        label.to_string()
    } else {
        let mut out = String::new();
        let mut w = 0;
        for ch in label.chars() {
            let cw = ch.to_string().width();
            if w + cw > max {
                break;
            }
            out.push(ch);
            w += cw;
        }
        out
    }
}

pub(super) fn draw_scatter(f: &mut Frame, area: Rect, app: &App) {
    use ratatui::symbols::Marker;
    use ratatui::widgets::{Axis, Chart, Dataset, GraphType};

    let Some(file) = app.active_benchmark_file() else {
        let block = Block::default().borders(Borders::ALL).title(" Scatter ");
        f.render_widget(block, area);
        return;
    };

    // Need at least two metrics to form a scatter plot.
    if file.metrics.len() < 2 {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Scatter (need 2 metrics) ");
        f.render_widget(block, area);
        return;
    }

    if file.models.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(" Scatter ");
        f.render_widget(block, area);
        return;
    }

    let x_idx = app.benchmarks_app.scatter_x.min(file.metrics.len() - 1);
    let y_idx = app.benchmarks_app.scatter_y.min(file.metrics.len() - 1);
    let x_metric = &file.metrics[x_idx];
    let y_metric = &file.metrics[y_idx];

    let x_value = |model: &ModelRow| metric_value(file, model, x_idx);
    let y_value = |model: &ModelRow| metric_value(file, model, y_idx);

    // Collect all points with both x and y values present
    let mut all_points: Vec<(f64, f64)> = Vec::new();
    for model in file.models.iter() {
        if let (Some(x), Some(y)) = (x_value(model), y_value(model)) {
            all_points.push((x, y));
        }
    }

    if all_points.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Scatter (no data) ");
        f.render_widget(block, area);
        return;
    }

    // Split area: chart on top, legend box at bottom (if selections exist)
    let has_selections = !app.selections.is_empty();
    let legend_height = if has_selections {
        (app.selections.len() as u16 + 2).min(area.height / 3) // +2 for borders
    } else {
        0
    };
    let (chart_area, legend_area) = if has_selections {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Length(legend_height)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    // Auto log scale for skewed axes
    let f64_cmp = |a: &f64, b: &f64| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    let mut x_vals: Vec<f64> = all_points.iter().map(|p| p.0).collect();
    let mut y_vals: Vec<f64> = all_points.iter().map(|p| p.1).collect();
    x_vals.sort_by(f64_cmp);
    y_vals.sort_by(f64_cmp);

    fn is_skewed(sorted: &[f64]) -> bool {
        if sorted.len() < 5 {
            return false;
        }
        let mid = sorted[sorted.len() / 2];
        let max = sorted[sorted.len() - 1];
        mid > 0.0 && max / mid > 5.0
    }

    let x_log = is_skewed(&x_vals);
    let y_log = is_skewed(&y_vals);

    let log_transform = |v: f64, use_log: bool| -> f64 {
        if use_log {
            (v.max(0.001)).ln()
        } else {
            v
        }
    };

    let display_points: Vec<(f64, f64)> = all_points
        .iter()
        .map(|&(x, y)| (log_transform(x, x_log), log_transform(y, y_log)))
        .collect();

    let x_min = display_points
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min);
    let x_max = display_points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let y_min = display_points
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min);
    let y_max = display_points
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max);

    // Snap non-log bounds to nice round numbers so ticks land on whole values.
    let nice_bounds = |lo: f64, hi: f64, num_ticks: usize| -> [f64; 2] {
        let range = hi - lo;
        let raw_step = range / (num_ticks - 1) as f64;
        let mag = 10_f64.powf(raw_step.log10().floor());
        let nice_step = if raw_step / mag < 1.5 {
            mag
        } else if raw_step / mag < 3.5 {
            mag * 2.0
        } else if raw_step / mag < 7.5 {
            mag * 5.0
        } else {
            mag * 10.0
        };
        let nice_lo = (lo / nice_step).floor() * nice_step;
        let nice_hi = (hi / nice_step).ceil() * nice_step;
        [nice_lo.max(0.0), nice_hi]
    };

    let x_pad = (x_max - x_min).max(0.1) * 0.05;
    let y_pad = (y_max - y_min).max(0.1) * 0.05;
    let num_ticks = 7_usize;
    let x_bounds = if x_log {
        [x_min - x_pad, x_max + x_pad]
    } else {
        nice_bounds(x_min - x_pad, x_max + x_pad, num_ticks)
    };
    let y_bounds = if y_log {
        [y_min - y_pad, y_max + y_pad]
    } else {
        nice_bounds(y_min - y_pad, y_max + y_pad, num_ticks)
    };

    // Compute independent averages (each axis uses all models with data for that metric)
    let (x_sum, x_count) = file.models.iter().fold((0.0_f64, 0_u32), |(s, c), m| {
        if let Some(v) = x_value(m) {
            (s + log_transform(v, x_log), c + 1)
        } else {
            (s, c)
        }
    });
    let (y_sum, y_count) = file.models.iter().fold((0.0_f64, 0_u32), |(s, c), m| {
        if let Some(v) = y_value(m) {
            (s + log_transform(v, y_log), c + 1)
        } else {
            (s, c)
        }
    });
    let avg_x = if x_count > 0 {
        x_sum / x_count as f64
    } else {
        (x_bounds[0] + x_bounds[1]) / 2.0
    };
    let avg_y = if y_count > 0 {
        y_sum / y_count as f64
    } else {
        (y_bounds[0] + y_bounds[1]) / 2.0
    };

    let v_line = vec![(avg_x, y_bounds[0]), (avg_x, y_bounds[1])];
    let h_line = vec![(x_bounds[0], avg_y), (x_bounds[1], avg_y)];

    // Build selected model point sets + legend
    #[allow(clippy::type_complexity)]
    let mut legend_entries: Vec<(String, Color, u8, Option<f64>, Option<f64>)> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut selected_data: Vec<(String, Vec<(f64, f64)>, Color)> = Vec::new();

    for (sel_idx, &model_idx) in app.selections.iter().enumerate() {
        let color = compare_colors(sel_idx);
        if let Some(model) = file.models.get(model_idx) {
            let name = model.display_name.clone();
            let raw_x = x_value(model);
            let raw_y = y_value(model);
            if let (Some(x), Some(y)) = (raw_x, raw_y) {
                let tx = log_transform(x, x_log);
                let ty = log_transform(y, y_log);
                let in_range = tx >= x_bounds[0]
                    && tx <= x_bounds[1]
                    && ty >= y_bounds[0]
                    && ty <= y_bounds[1];
                selected_data.push((model.display_name.clone(), vec![(tx, ty)], color));
                legend_entries.push((name, color, if in_range { 1 } else { 2 }, raw_x, raw_y));
            } else {
                legend_entries.push((name, color, 0, raw_x, raw_y));
            }
        }
    }

    // Build datasets — crosshairs, background, then selected
    let mut datasets = vec![
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Indexed(242)))
            .data(&v_line),
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Indexed(242)))
            .data(&h_line),
        Dataset::default()
            .marker(Marker::Dot)
            .graph_type(GraphType::Scatter)
            .style(Style::default().fg(Color::DarkGray))
            .data(&display_points),
    ];

    for (_, points, color) in &selected_data {
        datasets.push(
            Dataset::default()
                .marker(Marker::HalfBlock)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(*color))
                .data(points),
        );
    }

    let x_label = x_metric.label.as_str();
    let y_label = y_metric.label.as_str();

    // Generate evenly-spaced tick labels for an axis.
    // ratatui distributes labels uniformly across the axis, so values must be evenly spaced.
    let make_ticks = |lo: f64, hi: f64, use_log: bool, n: usize| -> Vec<String> {
        let n = n.max(2);
        let step = (hi - lo) / (n - 1) as f64;
        let raw: Vec<f64> = (0..n).map(|i| lo + step * i as f64).collect();

        if use_log {
            // Format log-scale ticks: convert back to real values, ensure no duplicates
            let reals: Vec<f64> = raw.iter().map(|v| v.exp()).collect();
            // Pick precision that avoids duplicate labels
            for decimals in 0..=3 {
                let labels: Vec<String> = reals
                    .iter()
                    .map(|v| {
                        if decimals == 0 && *v >= 1.0 {
                            format!("{}", v.round() as i64)
                        } else {
                            format!("{:.prec$}", v, prec = decimals)
                        }
                    })
                    .collect();
                let unique: std::collections::HashSet<&String> = labels.iter().collect();
                if unique.len() == labels.len() {
                    return labels;
                }
            }
            // Fallback: 3 decimal places
            reals.iter().map(|v| format!("{:.3}", v)).collect()
        } else {
            raw.iter()
                .map(|v| {
                    if v.fract().abs() < 0.01 {
                        format!("{}", v.round() as i64)
                    } else {
                        format!("{:.1}", v)
                    }
                })
                .collect()
        }
    };

    let x_ticks = make_ticks(x_bounds[0], x_bounds[1], x_log, num_ticks);
    let y_ticks = make_ticks(y_bounds[0], y_bounds[1], y_log, num_ticks);

    let x_suffix = if x_log { " [log]" } else { "" };
    let y_suffix = if y_log { " [log]" } else { "" };

    // Format average for display (use original scale for log axes)
    let fmt_avg = |avg: f64, use_log: bool| -> String {
        let v = if use_log { avg.exp() } else { avg };
        if v >= 100.0 {
            format!("{}", v.round() as i64)
        } else {
            format!("{:.1}", v)
        }
    };
    let avg_style = Style::default().fg(Color::Indexed(242));

    let x_title = Line::from(vec![
        Span::styled(
            format!("{x_label}{x_suffix}"),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(format!("  avg:{}", fmt_avg(avg_x, x_log)), avg_style),
    ]);
    let y_title = Line::from(vec![
        Span::styled(
            format!("{y_label}{y_suffix}"),
            Style::default().fg(Color::Gray),
        ),
        Span::styled(format!("  avg:{}", fmt_avg(avg_y, y_log)), avg_style),
    ]);

    let compare_focused = app.benchmarks_app.focus == super::app::BenchmarkFocus::Compare;
    let scatter_border = if compare_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(scatter_border))
                .title(format!(" {y_label} vs {x_label} ")),
        )
        .x_axis(
            Axis::default()
                .title(x_title)
                .style(Style::default().fg(Color::Gray))
                .bounds(x_bounds)
                .labels(x_ticks),
        )
        .y_axis(
            Axis::default()
                .title(y_title)
                .style(Style::default().fg(Color::Gray))
                .bounds(y_bounds)
                .labels(y_ticks),
        )
        .legend_position(None);

    f.render_widget(chart, chart_area);

    // Format a raw value for legend display
    let fmt_val = |v: f64| -> String {
        if v >= 100.0 {
            format!("{}", v.round() as i64)
        } else if v >= 1.0 {
            format!("{:.1}", v)
        } else {
            format!("{:.2}", v)
        }
    };

    // Legend box below the chart
    if let Some(leg_area) = legend_area {
        use crate::tui::widgets::comparison_legend::{ComparisonLegend, LegendEntry, LegendMetric};

        let entries: Vec<LegendEntry> = legend_entries
            .iter()
            .map(|(name, color, status, raw_x, raw_y)| {
                let (marker, fg): (&'static str, Color) = if *status > 0 {
                    ("\u{25cf} ", *color)
                } else {
                    ("\u{25cb} ", Color::DarkGray)
                };
                let x_str = raw_x.map(&fmt_val).unwrap_or_else(|| "\u{2014}".into());
                let y_str = raw_y.map(&fmt_val).unwrap_or_else(|| "\u{2014}".into());
                let suffix = if *status == 2 { " (off-chart)" } else { "" };
                let y_with_suffix = format!("{}{}", y_str, suffix);

                LegendEntry::new(name.clone(), fg)
                    .marker(marker)
                    .metrics(vec![
                        LegendMetric::new(x_label.to_string(), x_str),
                        LegendMetric::new(y_label.to_string(), y_with_suffix),
                    ])
            })
            .collect();

        ComparisonLegend::new(entries).render(f, leg_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::schema::{MetricKind, ReasoningStatus, ScoreCell, SourceMeta};
    use std::collections::BTreeMap;

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
            release_date: Some("2026-01-01".into()),
            reasoning_status: ReasoningStatus::Reasoning,
            effort_level: None,
            variant_tag: None,
            open_weights: None,
            context_window: None,
            scores: score_map,
        }
    }

    fn sample_file() -> SourceFile {
        SourceFile {
            source: meta(),
            metrics: vec![
                metric("intelligence_index", MetricKind::Index, "Indexes", true),
                metric("price_input", MetricKind::UsdPerMTok, "Pricing", false),
            ],
            models: vec![
                model(
                    "alpha",
                    &[("intelligence_index", 70.0), ("price_input", 2.0)],
                ),
                model(
                    "beta",
                    &[("intelligence_index", 60.0), ("price_input", 1.0)],
                ),
                model("gamma", &[("intelligence_index", 80.0)]),
            ],
        }
    }

    #[test]
    fn metric_value_present_and_missing() {
        let file = sample_file();
        // alpha has intelligence_index = 70.0
        assert_eq!(metric_value(&file, &file.models[0], 0), Some(70.0));
        // gamma lacks price_input (idx 1)
        assert_eq!(metric_value(&file, &file.models[2], 1), None);
        // out-of-range metric index
        assert_eq!(metric_value(&file, &file.models[0], 9), None);
    }

    #[test]
    fn rank_values_higher_is_better() {
        // [70, 60, 80] -> ranks [2, 3, 1]
        let ranks = rank_values(&[Some(70.0), Some(60.0), Some(80.0)], true);
        assert_eq!(ranks, vec![Some(2), Some(3), Some(1)]);
    }

    #[test]
    fn rank_values_lower_is_better() {
        // [2.0, 1.0, missing] lower-is-better -> ranks [2, 1, None]
        let ranks = rank_values(&[Some(2.0), Some(1.0), None], false);
        assert_eq!(ranks, vec![Some(2), Some(1), None]);
    }

    #[test]
    fn rank_values_all_missing() {
        let ranks = rank_values(&[None, None], true);
        assert_eq!(ranks, vec![None, None]);
    }

    #[test]
    fn truncate_label_fits() {
        assert_eq!(truncate_label("GPQA", 14), "GPQA");
    }

    #[test]
    fn truncate_label_clips() {
        // 14-char column reserves 1 trailing space -> max 13 visible chars.
        let clipped = truncate_label("ThisLabelIsWayTooLong", 14);
        assert!(clipped.width() <= 13);
        assert_eq!(clipped, "ThisLabelIsWa");
    }
}
