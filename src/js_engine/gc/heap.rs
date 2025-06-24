use super::{GcObject, ObjectId};
use std::collections::HashMap;

pub struct Heap {
    objects: HashMap<ObjectId, Box<dyn GcObject>>,
    free_list: Vec<ObjectId>,
    next_id: usize,
    total_allocated: usize,
    collection_count: usize,
}

impl Heap {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            free_list: Vec::new(),
            next_id: 0,
            total_allocated: 0,
            collection_count: 0,
        }
    }

    pub fn allocate(&mut self, object: Box<dyn GcObject>) -> ObjectId {
        let size = object.size();
        let id = if let Some(reused_id) = self.free_list.pop() {
            reused_id
        } else {
            let id = ObjectId::new(self.next_id);
            self.next_id += 1;
            id
        };

        self.objects.insert(id, object);
        self.total_allocated += size;
        id
    }

    pub fn get_object(&self, id: ObjectId) -> Option<&dyn GcObject> {
        self.objects.get(&id).map(|obj| obj.as_ref())
    }

    pub fn get_object_mut(&mut self, id: ObjectId) -> Option<&mut (dyn GcObject + '_)> {
        Some(self.objects.get_mut(&id)?.as_mut())
    }

    pub fn sweep(&mut self, marked: &std::collections::HashSet<ObjectId>) -> usize {
        let mut freed_bytes = 0;
        let mut to_remove = Vec::new();

        for (&id, object) in &mut self.objects {
            if !marked.contains(&id) {
                freed_bytes += object.size();
                object.finalize();
                to_remove.push(id);
            }
        }

        for id in to_remove {
            self.objects.remove(&id);
            self.free_list.push(id);
        }

        self.collection_count += 1;
        freed_bytes
    }

    pub fn total_allocated(&self) -> usize {
        self.total_allocated
    }

    pub fn live_objects(&self) -> usize {
        self.objects.len()
    }

    pub fn collection_count(&self) -> usize {
        self.collection_count
    }

    pub fn defragment(&mut self) {
        let mut new_objects = HashMap::new();
        let mut id_mapping = HashMap::new();
        let mut next_id = 0;

        for (old_id, object) in self.objects.drain() {
            let new_id = ObjectId::new(next_id);
            new_objects.insert(new_id, object);
            id_mapping.insert(old_id, new_id);
            next_id += 1;
        }

        self.objects = new_objects;
        self.next_id = next_id;
        self.free_list.clear();
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HeapSnapshot {
    pub total_size: usize,
    pub object_count: usize,
    pub fragmentation_ratio: f64,
    pub largest_free_block: usize,
}

impl Heap {
    pub fn snapshot(&self) -> HeapSnapshot {
        let fragmentation_ratio = self.free_list.len() as f64 / self.next_id as f64;
        
        HeapSnapshot {
            total_size: self.total_allocated,
            object_count: self.objects.len(),
            fragmentation_ratio,
            largest_free_block: 0,
        }
    }
}