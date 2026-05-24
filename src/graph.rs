// graph.rs — DAG lane layout (mirrors gitk's algorithm).

use crate::git::Commit;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub commit_idx: usize,
    pub lane: usize,
    pub color_idx: usize,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from_lane: usize,
    pub to_lane: usize,
    pub color_idx: usize,
}

pub struct GraphLayout {
    pub nodes: Vec<GraphNode>,
    pub max_lanes: usize,
}

impl GraphLayout {
    pub fn compute(commits: &[Commit]) -> Self {
        let id_to_idx: HashMap<&str, usize> = commits
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();

        let n = commits.len();
        let mut lane_of: Vec<Option<usize>>    = vec![None; n];
        let mut color_of: Vec<usize>           = vec![0; n];
        let mut active: Vec<Option<usize>>     = Vec::new(); // lane→commit_idx
        let mut nodes: Vec<GraphNode>          = Vec::with_capacity(n);
        let mut max_lanes                      = 0usize;
        let mut next_color                     = 0usize;

        for i in 0..n {
            // Assign lane if not inherited
            if lane_of[i].is_none() {
                let lane = free_lane(&active);
                grow_to(&mut active, lane);
                lane_of[i]  = Some(lane);
                color_of[i] = next_color;
                next_color  += 1;
            }
            let my_lane  = lane_of[i].unwrap();
            let my_color = color_of[i];
            active[my_lane] = Some(i);
            max_lanes = max_lanes.max(active.iter().filter(|x| x.is_some()).count());

            let parents: Vec<usize> = commits[i].parents.iter()
                .filter_map(|p| id_to_idx.get(p.as_str()).copied())
                .collect();

            // First parent inherits our lane
            if let Some(&p0) = parents.first() {
                if lane_of[p0].is_none() {
                    lane_of[p0]  = Some(my_lane);
                    color_of[p0] = my_color;
                }
            }

            // Extra parents get new lanes
            for &p in parents.iter().skip(1) {
                if lane_of[p].is_none() {
                    let nl = free_lane(&active);
                    grow_to(&mut active, nl);
                    lane_of[p]  = Some(nl);
                    color_of[p] = next_color;
                    next_color  += 1;
                }
            }

            // Build edges for this row
            let mut edges: Vec<Edge> = Vec::new();

            // Carry-throughs for other active lanes
            for (li, slot) in active.iter().enumerate() {
                if let Some(ci) = *slot {
                    if ci != i {
                        edges.push(Edge { from_lane: li, to_lane: li, color_idx: color_of[ci] });
                    }
                }
            }

            // Merge edges (our node → extra parents)
            for &p in parents.iter().skip(1) {
                edges.push(Edge {
                    from_lane: my_lane,
                    to_lane:   lane_of[p].unwrap(),
                    color_idx: color_of[p],
                });
            }

            // Free lane if this commit has no parents still pending
            if parents.is_empty() {
                active[my_lane] = None;
            }

            nodes.push(GraphNode { commit_idx: i, lane: my_lane, color_idx: my_color, edges });
        }

        GraphLayout { nodes, max_lanes: max_lanes.max(1) }
    }
}

fn free_lane(active: &[Option<usize>]) -> usize {
    active.iter().position(|x| x.is_none()).unwrap_or(active.len())
}

fn grow_to(active: &mut Vec<Option<usize>>, lane: usize) {
    while active.len() <= lane {
        active.push(None);
    }
}

// ── Colour palette ──────────────────────────

pub const BRANCH_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(0x4e, 0x9a, 0xe0),
    egui::Color32::from_rgb(0xe0, 0x74, 0x4e),
    egui::Color32::from_rgb(0x5c, 0xc8, 0x6a),
    egui::Color32::from_rgb(0xd1, 0x5f, 0xce),
    egui::Color32::from_rgb(0xe0, 0xc0, 0x4e),
    egui::Color32::from_rgb(0x4e, 0xd1, 0xc5),
    egui::Color32::from_rgb(0xe0, 0x4e, 0x6e),
    egui::Color32::from_rgb(0x9e, 0xd1, 0x4e),
    egui::Color32::from_rgb(0x8e, 0x8e, 0xe0),
    egui::Color32::from_rgb(0xe0, 0x9e, 0x4e),
];

pub fn branch_color(idx: usize) -> egui::Color32 {
    BRANCH_COLORS[idx % BRANCH_COLORS.len()]
}

use egui;
