use crate::ir::AbsenceValue;
use crate::protocol::schema::UnknownReason;

pub const MAX_SCC_ITERATIONS: usize = 8;

#[derive(Debug, Clone)]
pub struct SccResult {
    pub sccs: Vec<Vec<usize>>,
    pub node_to_scc: Vec<usize>,
    pub back_edges: Vec<(usize, usize, u32)>,
}

/// Tarjan's strongly connected components algorithm.
/// Returns SCCs in reverse topological order (callees/dependencies first).
pub fn tarjan_scc(adj: &[Vec<usize>]) -> SccResult {
    let n = adj.len();
    let mut index = 0usize;
    let mut indices = vec![None; n];
    let mut lowlink = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut sccs = Vec::new();

    struct State<'a> {
        adj: &'a [Vec<usize>],
        index: &'a mut usize,
        indices: &'a mut [Option<usize>],
        lowlink: &'a mut [usize],
        on_stack: &'a mut [bool],
        stack: &'a mut Vec<usize>,
        sccs: &'a mut Vec<Vec<usize>>,
    }

    fn strongconnect(v: usize, st: &mut State<'_>) {
        st.indices[v] = Some(*st.index);
        st.lowlink[v] = *st.index;
        *st.index += 1;
        st.stack.push(v);
        st.on_stack[v] = true;

        for &w in &st.adj[v] {
            if w >= st.adj.len() {
                continue;
            }
            if st.indices[w].is_none() {
                strongconnect(w, st);
                st.lowlink[v] = st.lowlink[v].min(st.lowlink[w]);
            } else if st.on_stack[w] {
                st.lowlink[v] = st.lowlink[v].min(st.indices[w].unwrap());
            }
        }

        if st.lowlink[v] == st.indices[v].unwrap() {
            let mut scc = Vec::new();
            loop {
                let w = st.stack.pop().unwrap();
                st.on_stack[w] = false;
                scc.push(w);
                if w == v {
                    break;
                }
            }
            st.sccs.push(scc);
        }
    }

    let mut st = State {
        adj,
        index: &mut index,
        indices: &mut indices,
        lowlink: &mut lowlink,
        on_stack: &mut on_stack,
        stack: &mut stack,
        sccs: &mut sccs,
    };

    for v in 0..n {
        if st.indices[v].is_none() {
            strongconnect(v, &mut st);
        }
    }

    let mut node_to_scc = vec![0usize; n];
    for (scc_id, scc) in sccs.iter().enumerate() {
        for &node in scc {
            node_to_scc[node] = scc_id;
        }
    }

    let mut back_edges = Vec::new();
    for (u, neighbors) in adj.iter().enumerate() {
        for &v in neighbors {
            if v < n && node_to_scc[u] == node_to_scc[v] {
                let scc_id = node_to_scc[u] as u32;
                if sccs[scc_id as usize].len() > 1 || u == v {
                    back_edges.push((u, v, scc_id));
                }
            }
        }
    }

    SccResult {
        sccs,
        node_to_scc,
        back_edges,
    }
}

/// Bounded monotone worklist iteration over SCCs (max 8 iterations before widening to `Unknown`).
pub fn solve_scc_fixed_point<F>(
    scc_result: &SccResult,
    initial: Vec<AbsenceValue>,
    mut transfer: F,
) -> (Vec<AbsenceValue>, Vec<u32>)
where
    F: FnMut(usize, &[AbsenceValue]) -> AbsenceValue,
{
    let mut values = initial;
    let mut widened_sccs = Vec::new();

    for (scc_id, scc) in scc_result.sccs.iter().enumerate() {
        let is_cyclic = scc.len() > 1
            || scc_result
                .back_edges
                .iter()
                .any(|&(_, _, id)| id == scc_id as u32);

        if !is_cyclic {
            for &node in scc {
                values[node] = transfer(node, &values);
            }
            continue;
        }

        let mut converged = false;
        for _iter in 0..MAX_SCC_ITERATIONS {
            let mut changed = false;
            for &node in scc {
                let next = values[node].join(transfer(node, &values));
                if next != values[node] {
                    values[node] = next;
                    changed = true;
                }
            }
            if !changed {
                converged = true;
                break;
            }
        }

        if !converged {
            widened_sccs.push(scc_id as u32);
            for &node in scc {
                values[node] = values[node].with_unknown(UnknownReason::MaxDepth);
            }
        }
    }

    (values, widened_sccs)
}
