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

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::encoder_syntax::SuperblockEdge;
use splot_parallel::WorkerPool;
use splot_parallel::prelude::Scope;

use crate::error::{Error, Result};
use crate::intra_frame::coding;
use crate::loop_filter::{CodedTile, FrameDecisions};
use crate::tile_writer::{KeyFrameTileWriter, ResidualCoder, SuperblockDecision};

/// The facade, coder, row context and import position of one superblock row.
struct Row<C, X> {
    writer: KeyFrameTileWriter,
    coder: C,
    context: X,
    imported: usize,
}

/// The superblock search of one tile on row facades: in a wavefront, a
/// superblock waits for its left and top-right neighbours and prices from
/// CDFs averaged from them; in raster order, a row waits for the row above
/// and starts from the CDFs it ended with. `row_writer(row)` is a fresh
/// writer of the tile with the row's context, `coder()` a fresh residual
/// coder, and `search` plans and searches the next superblock of a writer.
pub(crate) struct TileSearch<C, X, W, N, S> {
    rows: usize,
    cols: usize,
    raster: bool,
    row_writer: W,
    coder: N,
    search: S,
    row_state: Vec<Mutex<Option<Row<C, X>>>>,
    edges: Vec<Mutex<Option<SuperblockEdge>>>,
    row_ends: Vec<Mutex<Option<SuperblockEdge>>>,
    committed: Vec<Mutex<Option<FrameDecisions>>>,
    #[cfg(test)]
    checks: Vec<Mutex<Option<SuperblockEdge>>>,
}

impl<C, X, W, N, S> TileSearch<C, X, W, N, S>
where
    C: ResidualCoder + Send,
    X: Send,
    W: Fn(usize) -> Result<(KeyFrameTileWriter, X)> + Sync,
    N: Fn() -> C + Sync,
    S: Fn(&mut KeyFrameTileWriter, &mut C, &X) -> Result<(SuperblockDecision, Option<u32>)> + Sync,
{
    pub(crate) fn new(
        (rows, cols): (usize, usize),
        raster: bool,
        row_writer: W,
        coder: N,
        search: S,
    ) -> Self {
        Self {
            rows,
            cols,
            raster,
            row_writer,
            coder,
            search,
            row_state: slots(rows),
            edges: slots(rows * cols),
            row_ends: slots(rows),
            committed: slots(rows * cols),
            #[cfg(test)]
            checks: slots(rows * cols),
        }
    }

    /// The superblocks the search of superblock `index` waits for.
    pub(crate) fn predecessors(&self, index: usize) -> Vec<usize> {
        let (r, c) = (index / self.cols, index % self.cols);
        let left = (c > 0).then(|| index - 1);
        let above = (r > 0).then(|| {
            if self.raster {
                r * self.cols - 1
            } else {
                (r - 1) * self.cols + (c + 1).min(self.cols - 1)
            }
        });
        left.into_iter().chain(above).collect()
    }

    /// Searches superblock `index` and commits it into its row facade.
    pub(crate) fn superblock(&self, index: usize) -> Result<()> {
        let (rows, cols) = (self.rows, self.cols);
        let (r, c) = (index / cols, index % cols);
        let mut state = lock(&self.row_state[r])?;
        let row = match &mut *state {
            Some(row) => row,
            empty => {
                let (mut writer, context) = (self.row_writer)(r)?;
                writer.start_at_row(r);
                empty.insert(Row {
                    writer,
                    coder: (self.coder)(),
                    context,
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
                let edge = lock(&self.edges[index - cols - c + row.imported])?
                    .take()
                    .ok_or(Error::Unsupported {
                        what: "a superblock edge imported twice",
                    })?;
                syntax.import_edge(&edge).map_err(coding("edge import"))?;
                if !self.raster && row.imported == c + 1 {
                    syntax.take_edge_cdfs(&edge, c > 0);
                }
                row.imported += 1;
            }
            if self.raster && c == 0 {
                let end = lock(&self.row_ends[r - 1])?
                    .take()
                    .ok_or(Error::Unsupported {
                        what: "a superblock row read twice",
                    })?;
                syntax.take_edge_cdfs(&end, false);
            }
        }
        let decided = (self.search)(&mut row.writer, &mut row.coder, &row.context)?;
        let mut decisions = FrameDecisions::default();
        decisions
            .commit_superblock(&mut row.writer, decided, &mut row.coder)
            .map_err(coding("superblock commit"))?;
        if r + 1 < rows {
            let edge = row.writer.syntax_mut().superblock_edge(root);
            *lock(&self.edges[index])? = Some(edge.map_err(coding("superblock edge"))?);
            if self.raster && c + 1 == cols {
                let edge = row.writer.syntax_mut().superblock_edge(root);
                *lock(&self.row_ends[r])? = Some(edge.map_err(coding("superblock edge"))?);
            }
        }
        #[cfg(test)]
        {
            let edge = row.writer.syntax_mut().superblock_edge(root);
            *lock(&self.checks[index])? = Some(edge.map_err(coding("superblock edge"))?);
        }
        *lock(&self.committed[index])? = Some(decisions);
        if c + 1 == cols {
            *state = None;
        }
        Ok(())
    }

    /// The committed decisions of superblock `index`, once.
    pub(crate) fn take(&self, index: usize) -> Result<FrameDecisions> {
        lock(&self.committed[index])?
            .take()
            .ok_or(Error::Unsupported {
                what: "a superblock written twice",
            })
    }

    /// Checks in tests that `writer`, which just wrote superblock `index` at
    /// `root`, reconstructed it as its row facade did.
    #[cfg(test)]
    pub(crate) fn check(
        &self,
        index: usize,
        writer: &mut KeyFrameTileWriter,
        root: Option<splot_decode::encoder_syntax::PartitionNode>,
    ) -> Result<()> {
        let missing = Error::Unsupported {
            what: "a wavefront superblock without its check",
        };
        let worker = lock(&self.checks[index])?.take().ok_or(missing)?;
        let edge = writer
            .syntax_mut()
            .superblock_edge(root.ok_or(Error::Unsupported {
                what: "a write past its tile",
            })?);
        if edge
            .map_err(coding("superblock edge"))?
            .same_reconstruction(&worker)
        {
            Ok(())
        } else {
            Err(Error::Unsupported {
                what: "a wavefront reconstruction that differs from the writer's",
            })
        }
    }
}

/// Codes the tile of `writer` by a wavefront search. `row_writer()` is a
/// fresh writer of the same tile, `coder()` a fresh residual coder, and
/// `search` plans and searches the next superblock of a writer.
pub(crate) fn code_tile<C: ResidualCoder + Send>(
    pool: Option<&WorkerPool>,
    writer: KeyFrameTileWriter,
    row_writer: impl Fn() -> Result<KeyFrameTileWriter> + Sync,
    coder: impl Fn() -> C + Sync,
    search: impl Fn(&mut KeyFrameTileWriter, &mut C) -> Result<(SuperblockDecision, Option<u32>)> + Sync,
) -> Result<CodedTile> {
    let grid = writer.superblock_grid();
    let tile = TileSearch::new(
        grid,
        false,
        |_| Ok((row_writer()?, ())),
        coder,
        |writer: &mut KeyFrameTileWriter, coder: &mut C, (): &()| search(writer, coder),
    );
    let master = Mutex::new((writer, FrameDecisions::default()));
    let write = |index: usize| -> Result<()> {
        let decisions = tile.take(index)?;
        let (writer, kept) = &mut *lock(&master)?;
        #[cfg(test)]
        let root = writer.next_superblock_root();
        kept.write_committed(writer, decisions)
            .map_err(coding("superblock"))?;
        #[cfg(test)]
        tile.check(index, writer, root)?;
        Ok(())
    };
    let preds = predecessors(grid.0, grid.1);
    run_graph(pool, &preds, |node| {
        if node % 2 == 0 {
            tile.superblock(node / 2)
        } else {
            write(node / 2)
        }
    })?;
    let (writer, kept) = master.into_inner().map_err(|_| poisoned())?;
    CodedTile::finish(writer, kept).map_err(coding("tile finish"))
}

/// The predecessors of each node of a `rows x cols` superblock wavefront:
/// node `2 * i` searches superblock `i` in raster order after its left and
/// top-right (or, in the last column, top) neighbours; node `2 * i + 1`
/// writes it after the superblocks before it. Every predecessor has a
/// smaller index, so index order runs one superblock row at a time.
fn predecessors(rows: usize, cols: usize) -> Vec<Vec<usize>> {
    (0..rows * cols)
        .flat_map(|i| {
            let (r, c) = (i / cols, i % cols);
            let left = (c > 0).then(|| 2 * (i - 1));
            let above = (r > 0).then(|| 2 * ((r - 1) * cols + (c + 1).min(cols - 1)));
            let previous = i.checked_sub(1).map(|w| 2 * w + 1);
            [
                left.into_iter().chain(above).collect(),
                core::iter::once(2 * i).chain(previous).collect(),
            ]
        })
        .collect()
}

/// Runs `node` on every node of the graph whose predecessors are `preds`,
/// each once all its predecessors have run: on `pool` as they become ready,
/// else in index order, which must have every predecessor first. Stops at
/// the first error.
pub(crate) fn run_graph(
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
        for (index, preds) in preds.iter().enumerate() {
            if preds.iter().any(|&pred| pred >= index) {
                return Err(Error::Unsupported {
                    what: "a wavefront graph out of index order",
                });
            }
            node(index)?;
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
        assert_eq!(preds[10], vec![8, 4]);
        assert_eq!(preds[14], vec![12, 6]);
        assert_eq!(preds[11], vec![10, 9]);
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
