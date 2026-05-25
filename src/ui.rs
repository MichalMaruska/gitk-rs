// ui.rs — egui application.  Faithful port of gitk's three-pane layout.
//
//   ┌──────────────────────────────────────────────────────┐
//   │ Menu bar │ Search / find                              │
//   ├────────────────────────┬─────────────────────────────┤
//   │  Branch sidebar        │  Commit graph (top)         │
//   │  (collapsible)         ├───────────────┬─────────────┤
//   │                        │ Commit detail │ File list   │
//   └────────────────────────┴───────────────┴─────────────┘

use eframe::egui::{self, Color32, FontId, RichText, Stroke};
use std::sync::Arc;

use crate::git::{CommitDiff, GitRepo, RefKind, BlameLine};
use crate::graph::{branch_color, GraphLayout};

// ──────────────────────────────────────────────
// Application state
// ──────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum BottomTab { Diff, Blame }

pub struct GitkApp {
    repo:        Option<Arc<GitRepo>>,
    repo_path:   String,
    error:       Option<String>,

    commits:     Vec<crate::git::Commit>,
    graph:       Option<GraphLayout>,

    selected_idx:  Option<usize>,
    diff_cache:    Option<(String, CommitDiff)>,
    blame_cache:   Option<(String, String, Vec<BlameLine>)>, // (commit_id, file, lines)

    search_query:   String,
    search_matches: Vec<usize>,
    search_cursor:  usize,
    search_field:   String,    // "summary" | "author" | "sha"

    // layout fractions
    sidebar_open:    bool,
    sidebar_w:       f32,
    top_frac:        f32,
    bottom_split:    f32,

    show_dates:      bool,
    show_remotes:    bool,
    max_commits:     usize,

    selected_file:   Option<usize>,
    bottom_tab:      BottomTab,

    branches:        Vec<String>,
    branch_filter:   String,

    // context menu
    ctx_menu_open:   bool,
    ctx_menu_idx:    Option<usize>,
    ctx_menu_pos:    egui::Pos2,

    status_msg:      String,
    copied_flash:    f32, // countdown timer for "Copied!" flash
}

impl GitkApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, repo_path: &str) -> Self {
        let mut app = Self {
            repo: None,
            repo_path: repo_path.to_string(),
            error: None,
            commits: vec![],
            graph: None,
            selected_idx: None,
            diff_cache: None,
            blame_cache: None,
            search_query: String::new(),
            search_matches: vec![],
            search_cursor: 0,
            search_field: "summary".into(),
            sidebar_open: true,
            sidebar_w: 180.0,
            top_frac: 0.55,
            bottom_split: 0.62,
            show_dates: true,
            show_remotes: false,
            max_commits: 5_000,
            selected_file: None,
            bottom_tab: BottomTab::Diff,
            branches: vec![],
            branch_filter: "(all)".into(),
            ctx_menu_open: false,
            ctx_menu_idx: None,
            ctx_menu_pos: egui::Pos2::ZERO,
            status_msg: String::new(),
            copied_flash: 0.0,
        };
        app.load_repo(repo_path);
        app
    }

    // ── Repo loading ──────────────────────────

    fn load_repo(&mut self, path: &str) {
        match GitRepo::open(path) {
            Ok(repo) => {
                let repo = Arc::new(repo);
                self.status_msg = format!("Loading {}…", repo.name());
                let commits = repo.load_commits(self.max_commits);
                let graph   = GraphLayout::compute(&commits);
                self.branches = repo.all_branches();
                self.status_msg = format!(
                    "{} on {} — {} commits",
                    repo.name(), repo.head_branch(), commits.len()
                );
                self.commits     = commits;
                self.graph       = Some(graph);
                self.repo        = Some(repo);
                self.error       = None;
                self.diff_cache  = None;
                self.blame_cache = None;
                self.selected_idx = None;
                self.search_matches.clear();
                if !self.commits.is_empty() {
                    self.select_commit(0);
                }
            }
            Err(e) => {
                self.error = Some(format!("Cannot open repository: {e}"));
            }
        }
    }

    fn select_commit(&mut self, idx: usize) {
        self.selected_idx  = Some(idx);
        self.selected_file = None;
        self.bottom_tab    = BottomTab::Diff;
        if let Some(repo) = &self.repo {
            let id = self.commits[idx].id.clone();
            if self.diff_cache.as_ref().map_or(true, |(k, _)| k != &id) {
                let diff = repo.load_diff(&id);
                self.diff_cache = Some((id, diff));
            }
        }
    }

    // ── Search ────────────────────────────────

    fn run_search(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() { self.search_matches.clear(); return; }
        let field = self.search_field.clone();
        self.search_matches = self.commits.iter().enumerate()
            .filter(|(_, c)| match field.as_str() {
                "author"  => c.author.to_lowercase().contains(&q) || c.email.to_lowercase().contains(&q),
                "sha"     => c.id.starts_with(&q) || c.short_id.starts_with(&q),
                _         => c.summary.to_lowercase().contains(&q) || c.body.to_lowercase().contains(&q),
            })
            .map(|(i, _)| i)
            .collect();
        self.search_cursor = 0;
        if let Some(&first) = self.search_matches.first() { self.select_commit(first); }
    }

    fn search_step(&mut self, forward: bool) {
        if self.search_matches.is_empty() { return; }
        if forward {
            self.search_cursor = (self.search_cursor + 1) % self.search_matches.len();
        } else {
            self.search_cursor = self.search_cursor.checked_sub(1)
                .unwrap_or(self.search_matches.len() - 1);
        }
        let idx = self.search_matches[self.search_cursor];
        self.select_commit(idx);
    }

    // ── Blame ─────────────────────────────────

    fn load_blame_for_selected_file(&mut self) {
        let (commit_id, file_path) = match (self.selected_idx, self.selected_file.as_ref()) {
            (Some(ci), Some(&fi)) => {
                let cid  = self.commits[ci].id.clone();
                let path = self.diff_cache.as_ref()
                    .map(|(_, d)| d.files[fi].path.clone())
                    .unwrap_or_default();
                (cid, path)
            }
            _ => return,
        };
        if self.blame_cache.as_ref()
            .map_or(false, |(cid, fp, _)| cid == &commit_id && fp == &file_path)
        { return; }

        if let Some(repo) = &self.repo {
            let lines = repo.blame_file(&file_path, Some(&commit_id));
            self.blame_cache = Some((commit_id, file_path, lines));
        }
    }
}

// ──────────────────────────────────────────────
// eframe::App
// ──────────────────────────────────────────────

impl eframe::App for GitkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Dark style ────────────────────────
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill        = Color32::from_rgb(0x1e, 0x1e, 0x1e);
        style.visuals.window_fill       = Color32::from_rgb(0x28, 0x28, 0x28);
        style.visuals.extreme_bg_color  = Color32::from_rgb(0x12, 0x12, 0x12);
        style.visuals.code_bg_color     = Color32::from_rgb(0x2a, 0x2a, 0x2a);
        ctx.set_style(style);

        // ── Keyboard shortcuts ─────────────────
        ctx.input(|inp| {
            if inp.key_pressed(egui::Key::ArrowDown) {
                if let Some(i) = self.selected_idx {
                    if i + 1 < self.commits.len() { self.select_commit(i + 1); }
                }
            }
            if inp.key_pressed(egui::Key::ArrowUp) {
                if let Some(i) = self.selected_idx {
                    if i > 0 { self.select_commit(i - 1); }
                }
            }
            if inp.modifiers.ctrl && inp.key_pressed(egui::Key::F) {
                // focus search — handled implicitly since we always render it
            }
            if inp.modifiers.ctrl && inp.key_pressed(egui::Key::R) {
                let path = self.repo_path.clone();
                self.load_repo(&path);
            }
            if inp.key_pressed(egui::Key::F5) {
                let path = self.repo_path.clone();
                self.load_repo(&path);
            }
        });

        // tick copied flash
        if self.copied_flash > 0.0 {
            self.copied_flash -= ctx.input(|i| i.stable_dt);
            ctx.request_repaint();
        }

        // ── Menu bar ──────────────────────────
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Reload  (F5 / Ctrl+R)").clicked() {
                        let p = self.repo_path.clone();
                        self.load_repo(&p);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.sidebar_open, "Branch sidebar");
                    ui.checkbox(&mut self.show_dates,   "Show dates");
                    ui.checkbox(&mut self.show_remotes, "Show remote branches");
                    ui.separator();
                    ui.label("Max commits:");
                    for &n in &[500usize, 2000, 5000, 10_000] {
                        if ui.selectable_label(self.max_commits == n, format!("{n}")).clicked() {
                            self.max_commits = n;
                            let p = self.repo_path.clone();
                            self.load_repo(&p);
                            ui.close_menu();
                        }
                    }
                });

                // ── Search bar ───────────────────
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("▼").on_hover_text("Next match").clicked() { self.search_step(true); }
                    if ui.small_button("▲").on_hover_text("Prev match").clicked() { self.search_step(false); }

                    let cnt = if !self.search_matches.is_empty() {
                        format!("{}/{}", self.search_cursor + 1, self.search_matches.len())
                    } else { String::new() };
                    ui.label(RichText::new(cnt).color(Color32::from_rgb(0x8a, 0xc8, 0xff)).size(11.0));

                    let prev = self.search_query.clone();
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .desired_width(200.0)
                            .hint_text("Search…"),
                    );
                    if resp.changed() && self.search_query != prev { self.run_search(); }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.search_step(true);
                    }

                    // search field selector
                    egui::ComboBox::from_id_source("search_field")
                        .selected_text(&self.search_field)
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for f in &["summary", "author", "sha"] {
                                if ui.selectable_label(self.search_field == *f, *f).clicked() {
                                    self.search_field = f.to_string();
                                    self.run_search();
                                }
                            }
                        });
                    ui.label("Find:");
                });
            });
        });

        // ── Status bar ────────────────────────
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::from_rgb(0xff, 0x60, 0x60), e);
                } else {
                    ui.label(
                        RichText::new(&self.status_msg)
                            .color(Color32::from_rgb(0xaa, 0xaa, 0xaa))
                            .size(11.0),
                    );
                }
                if self.copied_flash > 0.0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(Color32::from_rgb(0x5c, 0xc8, 0x6a), "✓ Copied to clipboard");
                    });
                }
            });
        });

        // ── Sidebar (branch filter) ────────────
        if self.sidebar_open {
            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(self.sidebar_w)
                .width_range(120.0..=300.0)
                .show(ctx, |ui| {
                    self.sidebar_w = ui.available_width();
                    ui.add_space(4.0);
                    ui.label(RichText::new("Branches").color(Color32::from_rgb(0x80, 0x80, 0xa0)).size(11.0));
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let branches = self.branches.clone();
                        for br in &branches {
                            if !self.show_remotes && br.contains('/') && br != "(all)" { continue; }
                            let sel = &self.branch_filter == br;
                            let col = if sel {
                                Color32::WHITE
                            } else if br.starts_with("origin/") || br.contains('/') {
                                Color32::from_rgb(0x8a, 0xa0, 0xc8)
                            } else {
                                Color32::from_rgb(0xd4, 0xd4, 0xd4)
                            };
                            let label = RichText::new(br).color(col).size(11.5);
                            if ui.selectable_label(sel, label).clicked() {
                                self.branch_filter = br.clone();
                                // TODO: filter commits by branch
                            }
                        }
                    });
                });
        }

        // ── Central: graph + detail ────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            let total_h = ui.available_height();
            let top_h   = (total_h * self.top_frac).max(60.0).min(total_h - 60.0);
            let bot_h   = (total_h - top_h - 5.0).max(60.0);

            // Graph panel
            egui::Frame::none()
                .fill(Color32::from_rgb(0x18, 0x18, 0x18))
                .show(ui, |ui| {
                    ui.set_height(top_h);
                    self.draw_graph(ui, ctx);
                });

            // Horizontal splitter
            let sep = ui.allocate_rect(
                egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 5.0)),
                egui::Sense::drag(),
            );
            if sep.dragged() {
                self.top_frac = ((top_h + sep.drag_delta().y) / total_h).clamp(0.15, 0.85);
            }
            ui.painter().rect_filled(sep.rect, 0.0, Color32::from_rgb(0x30, 0x30, 0x38));

            // Bottom split
            ui.horizontal(|ui| {
                let bw = ui.available_width();
                let lw = (bw * self.bottom_split).max(60.0);
                let rw = (bw - lw - 5.0).max(60.0);

                egui::Frame::none()
                    .fill(Color32::from_rgb(0x1a, 0x1a, 0x1a))
                    .show(ui, |ui| {
                        ui.set_width(lw);
                        ui.set_height(bot_h);
                        self.draw_detail(ui);
                    });

                let vs = ui.allocate_rect(
                    egui::Rect::from_min_size(ui.cursor().min, egui::vec2(5.0, bot_h)),
                    egui::Sense::drag(),
                );
                if vs.dragged() {
                    self.bottom_split = ((lw + vs.drag_delta().x) / bw).clamp(0.2, 0.85);
                }
                ui.painter().rect_filled(vs.rect, 0.0, Color32::from_rgb(0x30, 0x30, 0x38));

                egui::Frame::none()
                    .fill(Color32::from_rgb(0x1c, 0x1c, 0x1c))
                    .show(ui, |ui| {
                        ui.set_width(rw);
                        ui.set_height(bot_h);
                        self.draw_files(ui);
                    });
            });
        });

        // ── Context menu ──────────────────────
        self.draw_context_menu(ctx);
    }
}

// ──────────────────────────────────────────────
// Graph panel
// ──────────────────────────────────────────────

const ROW_H:  f32 = 21.0;
const LANE_W: f32 = 18.0;
const DOT_R:  f32 = 5.0;

impl GitkApp {
    fn draw_graph(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.commits.is_empty() {
            ui.centered_and_justified(|ui| { ui.label("No commits"); });
            return;
        }

        // Extract all data from self into locals BEFORE any closure,
        // so no borrow of self survives into the show_rows closure.
        let (graph_nodes, max_lanes) = match &self.graph {
            Some(g) => (g.nodes.clone(), g.max_lanes),
            None => return,
        };
        let n              = self.commits.len();
        let show_dates     = self.show_dates;
        let selected_idx   = self.selected_idx;
        let search_matches = self.search_matches.clone();
        // Per-row display data — avoids borrowing self.commits inside the closure.
        let commit_rows: Vec<(String, String, String, Vec<crate::git::RefLabel>)> =
            self.commits.iter().map(|c| (
                c.summary.clone(),
                c.author.clone(),
                c.date_str.clone(),
                c.refs.clone(),
            )).collect();

        let graph_w  = (max_lanes as f32 * LANE_W + 8.0).max(60.0);
        let date_w   = if show_dates { 138.0 } else { 0.0 };
        let author_w = 170.0;

        // Column headers
        let hdr_bg = Color32::from_rgb(0x22, 0x22, 0x28);
        egui::Frame::none().fill(hdr_bg).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(graph_w);
                ui.label(RichText::new("Summary").color(Color32::from_rgb(0x70, 0x70, 0x90)).size(10.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if show_dates {
                        ui.label(RichText::new("Date").color(Color32::from_rgb(0x70, 0x70, 0x90)).size(10.5));
                        ui.add_space(date_w - 30.0);
                    }
                    ui.label(RichText::new("Author").color(Color32::from_rgb(0x70, 0x70, 0x90)).size(10.5));
                    ui.add_space(author_w - 44.0);
                });
            });
        });

        let avail_w = ui.available_width();

        // Collect interactions from inside the closure, apply to self after.
        let mut clicked_row: Option<usize> = None;
        let mut right_clicked: Option<(usize, egui::Pos2)> = None;

        egui::ScrollArea::vertical()
            .id_source("graph_scroll")
            .auto_shrink([false; 2])
            .show_rows(ui, ROW_H, n, |ui, vis| {
                let painter = ui.painter().clone();
                let origin  = ui.cursor().min;

                // ── Draw edges ────────────────────────────────────────────
                let r0 = vis.start.saturating_sub(1);
                let r1 = (vis.end + 1).min(n);
                for row in r0..r1 {
                    let node = &graph_nodes[row];
                    let yc   = origin.y + (row as f32 + 0.5) * ROW_H;
                    let yn   = origin.y + (row as f32 + 1.5) * ROW_H;

                    for edge in &node.edges {
                        let col = branch_color(edge.color_idx);
                        let x1  = origin.x + 4.0 + edge.from_lane as f32 * LANE_W + LANE_W / 2.0;
                        let x2  = origin.x + 4.0 + edge.to_lane   as f32 * LANE_W + LANE_W / 2.0;
                        if (x1 - x2).abs() < 0.5 {
                            painter.line_segment([egui::pos2(x1, yc), egui::pos2(x2, yn)],
                                Stroke::new(1.8_f32, col));
                        } else {
                            let mid = (yc + yn) / 2.0;
                            painter.line_segment([egui::pos2(x1, yc), egui::pos2(x1, mid)], Stroke::new(1.8_f32, col));
                            painter.line_segment([egui::pos2(x1, mid), egui::pos2(x2, mid)], Stroke::new(1.8_f32, col));
                            painter.line_segment([egui::pos2(x2, mid), egui::pos2(x2, yn)], Stroke::new(1.8_f32, col));
                        }
                    }
                }

                // ── Draw rows ─────────────────────────────────────────────
                for row in vis {
                    let node   = &graph_nodes[row];
                    let (summary, author, date_str, refs) = &commit_rows[node.commit_idx];
                    let is_sel = selected_idx == Some(row);
                    let is_hit = search_matches.contains(&row);

                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x, origin.y + row as f32 * ROW_H),
                        egui::vec2(avail_w, ROW_H),
                    );

                    // Background
                    let bg = if is_sel {
                        Color32::from_rgb(0x24, 0x40, 0x70)
                    } else if is_hit {
                        Color32::from_rgb(0x40, 0x30, 0x08)
                    } else if row % 2 == 0 {
                        Color32::from_rgb(0x1a, 0x1a, 0x1e)
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        painter.rect_filled(row_rect, 0.0, bg);
                    }

                    // Dot — use row_rect so position is correct inside scroll area
                    let dot_col = branch_color(node.color_idx);
                    let dx = row_rect.left() + 4.0 + node.lane as f32 * LANE_W + LANE_W / 2.0;
                    let dy = row_rect.center().y;
                    painter.circle_filled(egui::pos2(dx, dy), DOT_R, dot_col);
                    painter.circle_stroke(egui::pos2(dx, dy), DOT_R,
                        Stroke::new(1.0_f32, Color32::from_rgb(0xff, 0xff, 0xff)));

                    // Text area — use row_rect for correct coordinates inside scroll area
                    let tx  = row_rect.left() + graph_w;
                    let ty  = row_rect.top() + 3.0;
                    let rx  = row_rect.right();

                    // Ref badges
                    let mut bx = tx;
                    for rl in refs {
                        let (bg_col, fg_col) = match rl.kind {
                            RefKind::Head   => (Color32::from_rgb(0x10, 0x60, 0x10), Color32::from_rgb(0x90, 0xff, 0x90)),
                            RefKind::Tag    => (Color32::from_rgb(0x60, 0x40, 0x00), Color32::from_rgb(0xff, 0xd0, 0x50)),
                            RefKind::Remote => (Color32::from_rgb(0x00, 0x30, 0x60), Color32::from_rgb(0x80, 0xb8, 0xff)),
                            RefKind::Branch => (Color32::from_rgb(0x10, 0x30, 0x60), Color32::from_rgb(0x80, 0xc8, 0xff)),
                        };
                        let g = painter.layout_no_wrap(
                            rl.name.clone(),
                            FontId::proportional(10.0),
                            fg_col,
                        );
                        let bw = g.size().x + 6.0;
                        let br = egui::Rect::from_min_size(egui::pos2(bx + 1.0, ty - 1.0), egui::vec2(bw, ROW_H - 4.0));
                        painter.rect_filled(br, 2.5, bg_col);
                        painter.rect_stroke(br, 2.5, Stroke::new(0.5_f32, fg_col.linear_multiply(0.5)));
                        painter.galley(egui::pos2(bx + 4.0, ty + 1.0), g, fg_col);
                        bx += bw + 3.0;
                    }

                    // Summary — painter clips naturally at panel edge
                    let sum_col = if is_sel { Color32::WHITE } else { Color32::from_rgb(0xd4, 0xd4, 0xd4) };
                    painter.text(
                        egui::pos2(bx + 4.0, ty),
                        egui::Align2::LEFT_TOP,
                        summary.as_str(),
                        FontId::proportional(13.0),
                        sum_col,
                    );

                    // Author
                    painter.text(
                        egui::pos2(rx - date_w - author_w, ty + 1.0),
                        egui::Align2::LEFT_TOP,
                        author.as_str(),
                        FontId::proportional(11.0),
                        Color32::from_rgb(0x80, 0xc0, 0x80),
                    );

                    // Date
                    if show_dates {
                        painter.text(
                            egui::pos2(rx - date_w + 4.0, ty + 1.0),
                            egui::Align2::LEFT_TOP,
                            date_str.as_str(),
                            FontId::proportional(11.0),
                            Color32::from_rgb(0x80, 0xa0, 0xc8),
                        );
                    }

                    // Click / right-click — record intent, apply to self after closure
                    let resp = ui.allocate_rect(row_rect, egui::Sense::click());
                    if resp.clicked() && selected_idx != Some(row) {
                        clicked_row = Some(row);
                    }
                    if resp.secondary_clicked() {
                        let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default();
                        right_clicked = Some((row, pos));
                    }
                    if resp.hovered() && !is_sel {
                        painter.rect_filled(row_rect, 0.0,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 8));
                    }
                }
            });

        // Apply interactions now that the closure (and its borrows) are done
        if let Some(row) = clicked_row {
            self.select_commit(row);
        }
        if let Some((row, pos)) = right_clicked {
            self.ctx_menu_open = true;
            self.ctx_menu_idx  = Some(row);
            self.ctx_menu_pos  = pos;
            if self.selected_idx != Some(row) {
                self.select_commit(row);
            }
        }
    }
}

// ──────────────────────────────────────────────
// Context menu
// ──────────────────────────────────────────────

impl GitkApp {
    fn draw_context_menu(&mut self, ctx: &egui::Context) {
        if !self.ctx_menu_open { return; }
        let idx = match self.ctx_menu_idx { Some(i) => i, None => return };

        // Clone all data out of self before any closure captures it.
        let sha      = self.commits[idx].id.clone();
        let short    = self.commits[idx].short_id.clone();
        let author   = self.commits[idx].author.clone();
        let summary  = self.commits[idx].summary.clone();
        let body     = self.commits[idx].body.clone();
        let menu_pos = self.ctx_menu_pos;

        // Deferred mutations — populated inside the closure, applied after.
        let mut close               = false;
        let mut copy_text: Option<String> = None;
        let mut flash               = false;
        let mut find_author         = false;

        egui::Area::new("ctx_menu".into())
            .fixed_pos(menu_pos)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::popup(&ctx.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.label(RichText::new(format!("Commit {short}"))
                        .color(Color32::from_rgb(0x80, 0xb8, 0xff)).size(11.0));
                    ui.separator();
                    if ui.button("📋 Copy full SHA").clicked() {
                        copy_text = Some(sha.clone()); flash = true; close = true;
                    }
                    if ui.button("📋 Copy short SHA").clicked() {
                        copy_text = Some(short.clone()); flash = true; close = true;
                    }
                    if ui.button("📋 Copy commit message").clicked() {
                        let msg = if body.is_empty() {
                            summary.clone()
                        } else {
                            format!("{}

{}", summary, body)
                        };
                        copy_text = Some(msg); flash = true; close = true;
                    }
                    ui.separator();
                    if ui.button("👤 Find commits by this author").clicked() {
                        find_author = true; close = true;
                    }
                    ui.separator();
                    if ui.button("✖ Close").clicked() { close = true; }
                });
            });

        // Apply mutations after the closure (all borrows dropped).
        if let Some(text) = copy_text {
            ctx.output_mut(|o| o.copied_text = text);
        }
        if flash { self.copied_flash = 2.5; }
        if find_author {
            self.search_query = author;
            self.search_field = "author".into();
            self.run_search();
        }
        if ctx.input(|i| i.pointer.any_click()) && !ctx.is_pointer_over_area() {
            close = true;
        }
        if close { self.ctx_menu_open = false; }
    }
}

// ──────────────────────────────────────────────
// Detail panel (commit info + diff/blame tabs)
// ──────────────────────────────────────────────

impl GitkApp {
    fn draw_detail(&mut self, ui: &mut egui::Ui) {
        let idx = match self.selected_idx {
            Some(i) => i,
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Select a commit above").color(Color32::from_rgb(0x55, 0x55, 0x55)));
                });
                return;
            }
        };

        // Clone everything from self up front — no &self.x borrow survives into closures.
        let c_id       = self.commits[idx].id.clone();
        let c_author   = self.commits[idx].author.clone();
        let c_email    = self.commits[idx].email.clone();
        let c_date     = self.commits[idx].date_str.clone();
        let c_summary  = self.commits[idx].summary.clone();
        let c_body     = self.commits[idx].body.clone();
        let c_parents  = self.commits[idx].parents.clone();
        let c_refs     = self.commits[idx].refs.clone();
        let c_msg      = if c_body.is_empty() {
            c_summary.clone()
        } else {
            format!("{}

{}", c_summary, c_body)
        };

        let mut copy_sha  = false;
        let mut set_diff  = false;
        let mut set_blame = false;

        // Commit header panel
        egui::TopBottomPanel::top("detail_header")
            .frame(egui::Frame::none().fill(Color32::from_rgb(0x20, 0x20, 0x28)))
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&c_id)
                            .font(FontId::monospace(11.0))
                            .color(Color32::from_rgb(0x60, 0xa8, 0xe8)),
                    );
                    if ui.small_button("📋").on_hover_text("Copy SHA").clicked() {
                        copy_sha = true;
                    }
                });
                if !c_refs.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for rl in &c_refs {
                            let col = match rl.kind {
                                RefKind::Head   => Color32::from_rgb(0x90, 0xff, 0x90),
                                RefKind::Tag    => Color32::from_rgb(0xff, 0xd0, 0x50),
                                RefKind::Remote => Color32::from_rgb(0x80, 0xb8, 0xff),
                                RefKind::Branch => Color32::from_rgb(0x80, 0xc8, 0xff),
                            };
                            ui.label(RichText::new(format!("[{}]", rl.name)).color(col).size(11.0));
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Author:").color(Color32::from_rgb(0x70, 0x70, 0x70)).size(11.0));
                    ui.label(RichText::new(format!("{} <{}>", c_author, c_email))
                        .color(Color32::from_rgb(0x80, 0xc0, 0x80)).size(11.0));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Date:  ").color(Color32::from_rgb(0x70, 0x70, 0x70)).size(11.0));
                    ui.label(RichText::new(&c_date).color(Color32::from_rgb(0x80, 0xa0, 0xc8)).size(11.0));
                });
                if !c_parents.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Parents:").color(Color32::from_rgb(0x70,0x70,0x70)).size(11.0));
                        for p in &c_parents {
                            ui.label(RichText::new(&p[..8.min(p.len())])
                                .font(FontId::monospace(10.5))
                                .color(Color32::from_rgb(0x60, 0xa8, 0xe8)));
                        }
                    });
                }
                ui.add_space(4.0);
                ui.label(RichText::new(&c_msg).color(Color32::from_rgb(0xe0, 0xe0, 0xe0)).size(13.0));
                ui.add_space(4.0);
            });

        // Tab bar
        let current_tab = self.bottom_tab;
        egui::TopBottomPanel::top("detail_tabs")
            .frame(egui::Frame::none().fill(Color32::from_rgb(0x1a, 0x1a, 0x22)))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(current_tab == BottomTab::Diff,  "Diff").clicked()  { set_diff  = true; }
                    if ui.selectable_label(current_tab == BottomTab::Blame, "Blame").clicked() { set_blame = true; }
                });
            });

        // Apply tab mutations before drawing content
        if copy_sha {
            ui.output_mut(|o| o.copied_text = c_id.clone());
            self.copied_flash = 2.5;
        }
        if set_diff  { self.bottom_tab = BottomTab::Diff; }
        if set_blame {
            self.bottom_tab = BottomTab::Blame;
            self.load_blame_for_selected_file();
        }

        // Scrollable content
        let tab = self.bottom_tab;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::both()
                .id_source("detail_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    match tab {
                        BottomTab::Diff  => self.draw_diff(ui),
                        BottomTab::Blame => self.draw_blame(ui),
                    }
                });
        });
    }

    fn draw_diff(&self, ui: &mut egui::Ui) {
        let idx = match self.selected_idx { Some(i) => i, None => return };
        let cid = &self.commits[idx].id;
        let diff = match &self.diff_cache {
            Some((id, d)) if id == cid => d,
            _ => return,
        };
        if diff.patch.is_empty() {
            ui.colored_label(Color32::from_rgb(0x55, 0x55, 0x55), "(empty diff — initial commit or binary)");
            return;
        }
        for line in diff.patch.lines() {
            let col = if line.starts_with('+') && !line.starts_with("+++") {
                Color32::from_rgb(0x4e, 0xc0, 0x70)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Color32::from_rgb(0xe0, 0x60, 0x60)
            } else if line.starts_with("@@") {
                Color32::from_rgb(0x60, 0xa0, 0xd0)
            } else if line.starts_with("diff ") || line.starts_with("index ")
                   || line.starts_with("---")   || line.starts_with("+++") {
                Color32::from_rgb(0x90, 0x90, 0xd8)
            } else {
                Color32::from_rgb(0xc0, 0xc0, 0xc0)
            };
            ui.label(RichText::new(line).color(col).font(FontId::monospace(11.5)));
        }
    }

    fn draw_blame(&self, ui: &mut egui::Ui) {
        let blame = match &self.blame_cache {
            Some((_, _, lines)) => lines,
            None => {
                ui.colored_label(Color32::from_rgb(0x70, 0x70, 0x70),
                    "Select a file in the file list, then click the Blame tab.");
                return;
            }
        };
        if blame.is_empty() {
            ui.colored_label(Color32::from_rgb(0x55, 0x55, 0x55), "(binary file or blame unavailable)");
            return;
        }
        let mut last_sha = "";
        for bl in blame {
            let same_block = bl.commit_id.as_str() == last_sha;
            ui.horizontal(|ui| {
                // SHA column
                let sha_text = if same_block {
                    "        ".to_string()
                } else {
                    bl.short_id.clone()
                };
                ui.label(
                    RichText::new(&sha_text)
                        .font(FontId::monospace(10.5))
                        .color(Color32::from_rgb(0x60, 0xa8, 0xe8)),
                );
                // Author (abbreviated)
                let auth = if same_block { String::new() } else {
                    let a = &bl.author;
                    if a.len() > 12 { format!("{:.12}", a) } else { a.clone() }
                };
                ui.label(RichText::new(format!("{:<12}", auth))
                    .font(FontId::monospace(10.5))
                    .color(Color32::from_rgb(0x70, 0xb0, 0x70)));
                // Line number
                ui.label(RichText::new(format!("{:>4} ", bl.lineno))
                    .font(FontId::monospace(10.5))
                    .color(Color32::from_rgb(0x55, 0x55, 0x55)));
                // Content
                ui.label(RichText::new(&bl.content)
                    .font(FontId::monospace(11.5))
                    .color(Color32::from_rgb(0xd4, 0xd4, 0xd4)));
            });
            last_sha = &blame.last().map(|x| x.commit_id.as_str()).unwrap_or("");
            // (we re-assign on each iteration — use a simpler approach)
            let _ = last_sha;
            last_sha = bl.commit_id.as_str();
        }
    }
}

// ──────────────────────────────────────────────
// File list panel
// ──────────────────────────────────────────────

impl GitkApp {
    fn draw_files(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let count = self.diff_cache.as_ref().map(|(_, d)| d.files.len()).unwrap_or(0);
        ui.label(
            RichText::new(format!("Changed files ({})", count))
                .color(Color32::from_rgb(0x70, 0x70, 0x90))
                .size(11.0),
        );
        ui.separator();

        let files = match &self.diff_cache {
            Some((_, d)) => d.files.clone(),
            None => return,
        };
        if files.is_empty() {
            ui.colored_label(Color32::from_rgb(0x55, 0x55, 0x55), "(no files)");
            return;
        }

        egui::ScrollArea::vertical()
            .id_source("files_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (i, file) in files.iter().enumerate() {
                    let is_sel = self.selected_file == Some(i);
                    let status_col = match file.status {
                        'A' => Color32::from_rgb(0x4e, 0xc0, 0x70),
                        'D' => Color32::from_rgb(0xe0, 0x60, 0x60),
                        'M' => Color32::from_rgb(0x8a, 0xc8, 0xff),
                        'R' | 'C' => Color32::from_rgb(0xe0, 0xb0, 0x50),
                        _   => Color32::from_rgb(0xaa, 0xaa, 0xaa),
                    };

                    let resp = ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(" {} ", file.status))
                                .color(status_col)
                                .font(FontId::monospace(11.0)),
                        );
                        let fname = file.path.split('/').last().unwrap_or(&file.path);
                        let dir   = file.path.rfind('/').map(|p| &file.path[..p]).unwrap_or("");
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(fname)
                                    .color(if is_sel { Color32::WHITE } else { Color32::from_rgb(0xd8, 0xd8, 0xd8) })
                                    .font(FontId::monospace(11.5)),
                            );
                            if !dir.is_empty() {
                                ui.label(
                                    RichText::new(dir)
                                        .color(Color32::from_rgb(0x60, 0x60, 0x60))
                                        .size(10.0),
                                );
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(4.0);
                            if file.deletions > 0 {
                                ui.label(RichText::new(format!("-{}", file.deletions))
                                    .color(Color32::from_rgb(0xe0, 0x60, 0x60)).size(10.0));
                            }
                            if file.additions > 0 {
                                ui.label(RichText::new(format!("+{}", file.additions))
                                    .color(Color32::from_rgb(0x4e, 0xc0, 0x70)).size(10.0));
                            }
                        });
                    });

                    let full_resp = resp.response;
                    let click = ui.interact(full_resp.rect, ui.id().with(i), egui::Sense::click());
                    if click.clicked() {
                        self.selected_file = Some(i);
                        if self.bottom_tab == BottomTab::Blame {
                            self.load_blame_for_selected_file();
                        }
                    }
                    if is_sel {
                        ui.painter().rect_filled(full_resp.rect, 0.0,
                            Color32::from_rgb(0x24, 0x40, 0x70));
                    } else if click.hovered() {
                        ui.painter().rect_filled(full_resp.rect, 0.0,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 8));
                    }
                    ui.separator();
                }
            });
    }
}
