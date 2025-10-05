pub mod heap;

pub use heap::*;

use std::collections::HashSet;
use std::sync::Arc;
use tokio::runtime::{Builder, Handle};
use tokio::sync::RwLock;
use tracing::log;

pub struct GarbageCollector {
    heap: Arc<RwLock<Heap>>,
    roots: Arc<RwLock<HashSet<ObjectId>>>,
    #[allow(dead_code)]
    mark_stack: Vec<ObjectId>,
    collection_threshold: usize,
    allocation_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(usize);

pub trait GcObject: Send + Sync {
    fn mark(&self, marker: &mut GcMarker);
    fn finalize(&mut self);
    fn size(&self) -> usize;
}

pub struct GcMarker {
    marked: HashSet<ObjectId>,
    mark_stack: Vec<ObjectId>,
}

impl GarbageCollector {
    pub fn new() -> Self {
        Self {
            heap: Arc::new(RwLock::new(Heap::new())),
            roots: Arc::new(RwLock::new(HashSet::new())),
            mark_stack: Vec::new(),
            collection_threshold: 1024 * 1024,
            allocation_count: 0,
        }
    }

    pub async fn allocate<T: GcObject + 'static>(&mut self, object: T) -> ObjectId {
        let mut heap = self.heap.write().await;
        let size = object.size();
        let id = heap.allocate(Box::new(object));

        self.allocation_count += size;

        if self.allocation_count > self.collection_threshold {
            drop(heap);
            self.collect().await;
        }

        id
    }

    pub async fn add_root(&self, id: ObjectId) {
        let mut roots = self.roots.write().await;
        roots.insert(id);
    }

    pub async fn remove_root(&self, id: ObjectId) {
        let mut roots = self.roots.write().await;
        roots.remove(&id);
    }

    pub async fn collect(&mut self) {
        let start_time = std::time::Instant::now();

        let roots = self.roots.read().await.clone();
        let mut heap = self.heap.write().await;

        let scratch_stack = std::mem::take(&mut self.mark_stack);

        let mut marker = GcMarker {
            marked: HashSet::new(),
            mark_stack: scratch_stack,
        };

        for root in roots {
            if !marker.marked.contains(&root) {
                marker.mark_stack.push(root);
                self.mark_phase(&mut marker, &heap).await;
            }
        }

        let freed_bytes = self.sweep_phase(&mut marker, &mut heap).await;

        self.mark_stack = marker.mark_stack;

        self.allocation_count = self.allocation_count.saturating_sub(freed_bytes);

        let duration = start_time.elapsed();
        log::info!(
            "GC completed in {:?}, freed {} bytes",
            duration,
            freed_bytes
        );
    }

    pub fn collect_blocking(&mut self) {
        match Handle::try_current() {
            Ok(handle) => {
                handle.block_on(self.collect());
            }
            Err(_) => {
                if let Ok(rt) = Builder::new_current_thread().enable_all().build() {
                    rt.block_on(self.collect());
                }
            }
        }
    }

    async fn mark_phase(&self, marker: &mut GcMarker, heap: &Heap) {
        while let Some(id) = marker.mark_stack.pop() {
            if marker.marked.insert(id) {
                if let Some(object) = heap.get_object(id) {
                    object.mark(marker);
                }
            }
        }
    }

    async fn sweep_phase(&self, marker: &mut GcMarker, heap: &mut Heap) -> usize {
        heap.sweep(&marker.marked)
    }

    pub async fn force_collect(&mut self) {
        self.collect().await;
    }

    pub async fn get_stats(&self) -> GcStats {
        let heap = self.heap.read().await;
        GcStats {
            total_allocated: heap.total_allocated(),
            live_objects: heap.live_objects(),
            total_collections: heap.collection_count(),
            allocation_rate: self.allocation_count,
        }
    }
}

impl GcMarker {
    pub fn mark(&mut self, id: ObjectId) {
        if !self.marked.contains(&id) {
            self.mark_stack.push(id);
        }
    }
}

#[derive(Debug, Clone)]
pub struct GcStats {
    pub total_allocated: usize,
    pub live_objects: usize,
    pub total_collections: usize,
    pub allocation_rate: usize,
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }
}
