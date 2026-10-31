// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wavefront superblock search inside one tile.
//!
//! Every superblock row searches on a facade of its own, built fresh for the
//! row, so nothing it reads depends on which thread ran what. A superblock
//! starts once its left and top-right neighbours are committed: the row
//! facade first imports the bottom edge of the row above up to the top-right
//! superblock, and prices from its own CDFs (where the left neighbour left
//! them) averaged three to one with the top-right neighbour's. Committing
//! codes the decision into the row facade with adapting rates and keeps its
//! transform decisions. One writer then codes the committed superblocks in
//! raster order with the exact CDFs, replaying those decisions, so its
//! reconstruction is the row facades'.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::encoder_syntax::SuperblockEdge;
use splot_parallel::WorkerPool;
use splot_parallel::prelude::Scope;

use crate::error::{Error, Result};
use crate::intra_frame::coding;
use crate::loop_filter::{CodedTile, FrameDecisions};
use crate::tile_writer::{KeyFrameTileWriter, ResidualCoder, SuperblockDecision};

/// The facade, coder and import position of one superblock row.
struct Row<C> {
    writer: KeyFrameTileWriter,
    coder: C,
    imported: usize,
}

/// Codes the tile of `writer` by a wavefront search. `row_writer(row)` is a
/// fresh writer of the same tile, `coder()` a fresh residual coder, and
/// `search` plans and searches the next superblock of a writer.
pub(crate) fn code_tile<C: ResidualCoder + Send>(
    pool: Option<&WorkerPool>,
    writer: KeyFrameTileWriter,
    row_writer: impl Fn() -> Result<KeyFrameTileWriter> + Sync,
    coder: impl Fn() -> C + Sync,
    search: impl Fn(&mut KeyFrameTileWriter, &mut C) -> Result<(SuperblockDecision, Option<u32>)> + Sync,
) -> Result<CodedTile> {
    let (rows, cols) = writer.superblock_grid();
    let count = rows * cols;
    let row_state: Vec<Mutex<Option<Row<C>>>> = slots(rows);
    let edges: Vec<Mutex<Option<SuperblockEdge>>> = slots(count);
    let committed: Vec<Mutex<Option<FrameDecisions>>> = slots(count);
    #[cfg(test)]
    let checks: Vec<Mutex<Option<SuperblockEdge>>> = slots(count);
    let master = Mutex::new((writer, FrameDecisions::default()));
    let superblock = |index: usize| -> Result<()> {
        let (r, c) = (index / cols, index % cols);
        let mut state = lock(&row_state[r])?;
        let row = match &mut *state {
            Some(row) => row,
            empty => {
                let mut writer = row_writer()?;
                writer.start_at_row(r);
                empty.insert(Row {
                    writer,
                    coder: coder(),
                    imported: 0,
                })
            }
        };
        let root = row
            .writer
            .next_superblock_root()
            .ok_or(Error::Unsupported {
                what: "a wavefront past its tile",
            })?;
        let syntax = row.writer.syntax_mut();
        syntax.begin_superblock(root);
        if r > 0 {
            while row.imported <= (c + 1).min(cols - 1) {
                let edge = lock(&edges[index - cols - c + row.imported])?
                    .take()
                    .ok_or(Error::Unsupported {
                        what: "a superblock edge imported twice",
                    })?;
                syntax.import_edge(&edge).map_err(coding("edge import"))?;
                if row.imported == c + 1 {
                    syntax.take_edge_cdfs(&edge, c > 0);
                }
                row.imported += 1;
            }
        }
        let decided = search(&mut row.writer, &mut row.coder)?;
        let mut decisions = FrameDecisions::default();
        decisions
            .commit_superblock(&mut row.writer, decided, &mut row.coder)
            .map_err(coding("superblock commit"))?;
        if r + 1 < rows {
            let edge = row.writer.syntax_mut().superblock_edge(root);
            *lock(&edges[index])? = Some(edge.map_err(coding("superblock edge"))?);
        }
        #[cfg(test)]
        {
            let edge = row.writer.syntax_mut().superblock_edge(root);
            *lock(&checks[index])? = Some(edge.map_err(coding("superblock edge"))?);
        }
        *lock(&committed[index])? = Some(decisions);
        if c + 1 == cols {
            *state = None;
        }
        Ok(())
    };
    let write = |index: usize| -> Result<()> {
        let decisions = lock(&committed[index])?.take().ok_or(Error::Unsupported {
            what: "a superblock written twice",
        })?;
        let (writer, kept) = &mut *lock(&master)?;
        #[cfg(test)]
        let root = writer.next_superblock_root();
        kept.write_committed(writer, decisions)
            .map_err(coding("superblock"))?;
        #[cfg(test)]
        if let (Some(root), Some(worker)) = (root, lock(&checks[index])?.take()) {
            let edge = writer.syntax_mut().superblock_edge(root);
            if !edge
                .map_err(coding("superblock edge"))?
                .same_reconstruction(&worker)
            {
                return Err(Error::Unsupported {
                    what: "a wavefront reconstruction that differs from the writer's",
                });
            }
        }
        Ok(())
    };
    let preds = predecessors(rows, cols);
    run_graph(pool, &preds, |node| {
        if node < count {
            superblock(node)
        } else {
            write(node - count)
        }
    })?;
    let (writer, kept) = master.into_inner().map_err(|_| poisoned())?;
    CodedTile::finish(writer, kept).map_err(coding("tile finish"))
}

/// The predecessors of each node of a `rows x cols` superblock wavefront:
/// node `i < rows * cols` searches superblock `i` in raster order after its
/// left and top-right (or, in the last column, top) neighbours; node
/// `rows * cols + i` writes it after the superblocks before it.
fn predecessors(rows: usize, cols: usize) -> Vec<Vec<usize>> {
    let count = rows * cols;
    let search = (0..count).map(|i| {
        let (r, c) = (i / cols, i % cols);
        let left = (c > 0).then(|| i - 1);
        let above = (r > 0).then(|| (r - 1) * cols + (c + 1).min(cols - 1));
        left.into_iter().chain(above).collect()
    });
    let write = (0..count).map(|i| {
        core::iter::once(i)
            .chain(i.checked_sub(1).map(|w| count + w))
            .collect()
    });
    search.chain(write).collect()
}

/// Runs `node` on every node of the graph whose predecessors are `preds`,
/// each once all its predecessors have run: on `pool` as they become ready,
/// else in turn. Stops at the first error.
fn run_graph(
    pool: Option<&WorkerPool>,
    preds: &[Vec<usize>],
    node: impl Fn(usize) -> Result<()> + Sync,
) -> Result<()> {
    let mut succs = vec![Vec::new(); preds.len()];
    for (index, preds) in preds.iter().enumerate() {
        for &pred in preds {
            succs[pred].push(index);
        }
    }
    let roots = preds
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_empty())
        .map(|(i, _)| i);
    let Some(pool) = pool else {
        let mut waiting: Vec<usize> = preds.iter().map(Vec::len).collect();
        let mut ready: VecDeque<usize> = roots.collect();
        while let Some(index) = ready.pop_front() {
            node(index)?;
            for &succ in &succs[index] {
                waiting[succ] -= 1;
                if waiting[succ] == 0 {
                    ready.push_back(succ);
                }
            }
        }
        return Ok(());
    };
    let graph = Graph {
        waiting: preds.iter().map(|p| AtomicUsize::new(p.len())).collect(),
        succs,
        node,
        failed: Mutex::new(None),
    };
    pool.scope(|scope| {
        for root in roots {
            graph.spawn(scope, root);
        }
    });
    graph
        .failed
        .into_inner()
        .map_err(|_| poisoned())?
        .map_or(Ok(()), Err)
}

struct Graph<F> {
    waiting: Vec<AtomicUsize>,
    succs: Vec<Vec<usize>>,
    node: F,
    failed: Mutex<Option<Error>>,
}

impl<F: Fn(usize) -> Result<()> + Sync> Graph<F> {
    fn spawn<'a>(&'a self, scope: &Scope<'a>, index: usize) {
        scope.spawn(move |scope| {
            let stopped = self.failed.lock().map_or(true, |failed| failed.is_some());
            if stopped {
                return;
            }
            if let Err(error) = (self.node)(index) {
                if let Ok(mut failed) = self.failed.lock() {
                    failed.get_or_insert(error);
                }
                return;
            }
            for &succ in &self.succs[index] {
                if self.waiting[succ].fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.spawn(scope, succ);
                }
            }
        });
    }
}

fn slots<T>(count: usize) -> Vec<Mutex<Option<T>>> {
    (0..count).map(|_| Mutex::new(None)).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| poisoned())
}

const fn poisoned() -> Error {
    Error::Unsupported {
        what: "a wavefront task that panicked",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn graph_runs_each_node_after_its_predecessors_on_any_pool() {
        let preds = predecessors(3, 4);
        assert_eq!(preds[5], vec![4, 2]);
        assert_eq!(preds[7], vec![6, 3]);
        assert_eq!(preds[12 + 5], vec![5, 12 + 4]);
        for threads in [None, Some(1), Some(3)] {
            let pool = threads.map(|n: usize| WorkerPool::new(n.into()).unwrap());
            let done: Vec<AtomicUsize> = (0..preds.len()).map(|_| AtomicUsize::new(0)).collect();
            let order = AtomicUsize::new(1);
            run_graph(pool.as_ref(), &preds, |node| {
                for &pred in &preds[node] {
                    assert_ne!(done[pred].load(Ordering::Acquire), 0);
                }
                done[node].store(order.fetch_add(1, Ordering::AcqRel), Ordering::Release);
                Ok(())
            })
            .unwrap();
            assert!(done.iter().all(|d| d.load(Ordering::Acquire) != 0));
        }
    }

    #[test]
    fn graph_stops_at_an_error() {
        let preds = predecessors(2, 2);
        let pool = WorkerPool::new(2usize.into()).unwrap();
        let ran = AtomicUsize::new(0);
        let result = run_graph(Some(&pool), &preds, |node| {
            ran.fetch_add(1, Ordering::AcqRel);
            if node == 1 {
                return Err(poisoned());
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(ran.load(Ordering::Acquire) < preds.len());
    }
}
