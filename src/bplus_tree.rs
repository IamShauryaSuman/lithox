use bytemuck::from_bytes;
use std::sync::{Arc, Mutex, RwLock};

use crate::buffer_pool_manager::BufferPoolManager;
use crate::disk_manager::PageId;
use crate::node::{BPlusTreeHeader, InternalNode, LeafNode};

/// The orchestrator for the B+ Tree index.
pub struct BPlusTree {
    /// A thread-safe reference to the Buffer Pool
    bpm: Arc<Mutex<BufferPoolManager>>,

    /// The Page ID of the root node.
    /// It is wrapped in an RwLock because the root page ID changes
    /// if the root node splits and a new root is created!
    /// It is an `Option` because a brand new tree has no root yet.
    root_page_id: RwLock<Option<PageId>>,

    /// The maximum number of keys a node can hold before it must split.
    max_keys: u16,
}

impl BPlusTree {
    /// Initializes a new, empty B+ Tree index.
    pub fn new(bpm: Arc<Mutex<BufferPoolManager>>, max_keys: u16) -> Self {
        Self {
            bpm,
            root_page_id: RwLock::new(None),
            max_keys,
        }
    }

    /// Safely retrieves the current root page ID.
    pub fn get_root_id(&self) -> Option<PageId> {
        *self.root_page_id.read().unwrap()
    }

    /// Searches the B+ Tree for a given key and returns its value if found.
    pub fn search(&self, key: u64) -> Option<u64> {
        let mut current_page_id = self.get_root_id()?;

        loop {
            let page_arc = self.bpm.lock().unwrap().fetch_page(current_page_id)?;

            let next_page_id: Option<PageId>;
            let search_result: Option<u64>;

            {
                let page = page_arc.read().unwrap();
                let page_data = page.get_data();

                let header: &BPlusTreeHeader = from_bytes(&page_data[0..24]);

                if header.is_leaf() {
                    let leaf_node: &LeafNode = from_bytes(page_data);
                    search_result = leaf_node.lookup(key);
                    next_page_id = None;
                } else {
                    let internal_node: &InternalNode = from_bytes(page_data);
                    next_page_id = Some(internal_node.lookup(key));
                    search_result = None;
                }
            }

            self.bpm.lock().unwrap().unpin_page(current_page_id, false);

            match next_page_id {
                Some(id) => current_page_id = id,
                None => return search_result,
            }
        }
    }

    /// Inserts a key-value pair into the B+ Tree.
    pub fn insert(&self, key: u64, value: u64) {
        let root_id_opt = self.get_root_id();

        // SCENARIO 1: The tree is completely empty. We need to create the root.
        if root_id_opt.is_none() {
            self.start_new_tree(key, value);
            return;
        }

        // SCENARIO 2: Tree exists. Find the target leaf node.
        let leaf_page_id = self
            .find_leaf_page(key)
            .expect("Tree exists but leaf not found");

        let mut bpm = self.bpm.lock().unwrap();
        let page_arc = bpm
            .fetch_page(leaf_page_id)
            .expect("Failed to fetch leaf page");

        let mut needs_split = false;

        {
            let mut page = page_arc.write().unwrap();
            let page_data = page.get_data_mut();

            use bytemuck::from_bytes_mut;
            let leaf_node: &mut crate::node::LeafNode = from_bytes_mut(page_data);

            if leaf_node.insert_kv(key, value).is_err() {
                needs_split = true;
            }
        }

        bpm.unpin_page(leaf_page_id, true);

        drop(bpm);

        // SCENARIO 3: The Leaf is Full.
        if needs_split {
            self.split_leaf_node(leaf_page_id, key, value);
        }
    }

    /// Helper method to initialize a brand new tree when it is completely empty.
    fn start_new_tree(&self, key: u64, value: u64) {
        let mut bpm = self.bpm.lock().unwrap();

        if let Some((page_id, page_arc)) = bpm.new_page() {
            {
                let mut page = page_arc.write().unwrap();
                let page_data = page.get_data_mut();

                use bytemuck::from_bytes_mut;
                let leaf_node: &mut LeafNode = from_bytes_mut(page_data);

                leaf_node.header.page_id = page_id;
                leaf_node.header.page_type = crate::node::NODE_TYPE_LEAF;
                leaf_node.header.max_keys = self.max_keys;
                leaf_node.header.keys_count = 0;
                leaf_node.next_page_id = 0;
                leaf_node.prev_page_id = 0;

                leaf_node
                    .insert_kv(key, value)
                    .expect("Brand new leaf should not be full");
            }

            *self.root_page_id.write().unwrap() = Some(page_id);

            bpm.unpin_page(page_id, true);
        }
    }

    /// Traverses the tree to find the Leaf Page ID where a key belongs.
    fn find_leaf_page(&self, key: u64) -> Option<PageId> {
        let mut current_page_id = self.get_root_id()?;

        loop {
            let page_arc = self.bpm.lock().unwrap().fetch_page(current_page_id)?;
            let next_page_id: Option<PageId>;
            let found_leaf: bool;

            {
                let page = page_arc.read().unwrap();
                let page_data = page.get_data();

                use bytemuck::from_bytes;
                let header: &crate::node::BPlusTreeHeader = from_bytes(&page_data[0..24]);

                if header.is_leaf() {
                    found_leaf = true;
                    next_page_id = None;
                } else {
                    found_leaf = false;
                    let internal_node: &crate::node::InternalNode = from_bytes(page_data);
                    next_page_id = Some(internal_node.lookup(key));
                }
            }

            self.bpm.lock().unwrap().unpin_page(current_page_id, false);

            if found_leaf {
                return Some(current_page_id);
            }

            current_page_id = next_page_id.unwrap();
        }
    }

    /// Splits a full leaf node in half, inserts the new key, and propagates the split upwards.
    fn split_leaf_node(&self, old_leaf_page_id: PageId, key: u64, value: u64) {
        let mut bpm = self.bpm.lock().unwrap();

        let (new_leaf_page_id, new_page_arc) =
            bpm.new_page().expect("Failed to allocate page for split");
        let old_page_arc = bpm
            .fetch_page(old_leaf_page_id)
            .expect("Failed to fetch old leaf");

        let middle_key: u64;
        let parent_id: PageId;

        {
            let mut old_page = old_page_arc.write().unwrap();
            let mut new_page = new_page_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let old_leaf: &mut crate::node::LeafNode = from_bytes_mut(old_page.get_data_mut());
            let new_leaf: &mut crate::node::LeafNode = from_bytes_mut(new_page.get_data_mut());

            new_leaf.header.page_id = new_leaf_page_id;
            new_leaf.header.page_type = crate::node::NODE_TYPE_LEAF;
            new_leaf.header.max_keys = self.max_keys;
            parent_id = old_leaf.header.parent_id;
            new_leaf.header.parent_id = parent_id;

            let total_keys = old_leaf.header.keys_count as usize;
            let split_idx = total_keys / 2;
            let move_count = total_keys - split_idx;

            for i in 0..move_count {
                new_leaf.entries[i] = old_leaf.entries[split_idx + i];
            }

            new_leaf.header.keys_count = move_count as u16;
            old_leaf.header.keys_count = split_idx as u16;

            if key < new_leaf.entries[0].key {
                old_leaf
                    .insert_kv(key, value)
                    .expect("Old leaf failed to accept key post-split");
            } else {
                new_leaf
                    .insert_kv(key, value)
                    .expect("New leaf failed to accept key post-split");
            }

            middle_key = new_leaf.entries[0].key;

            new_leaf.next_page_id = old_leaf.next_page_id;
            new_leaf.prev_page_id = old_leaf_page_id;
            old_leaf.next_page_id = new_leaf_page_id;
        }

        bpm.unpin_page(old_leaf_page_id, true);
        bpm.unpin_page(new_leaf_page_id, true);

        drop(bpm);

        self.insert_into_parent(old_leaf_page_id, middle_key, new_leaf_page_id, parent_id);
    }

    /// Pushes a new routing key and right-child pointer up to the parent node.
    fn insert_into_parent(
        &self,
        left_page_id: PageId,
        key: u64,
        right_page_id: PageId,
        parent_id: PageId,
    ) {
        // SCENARIO 1: We split the Root! The tree must grow one level taller.
        if parent_id == 0 {
            let mut bpm = self.bpm.lock().unwrap();
            let (new_root_id, new_root_arc) = bpm.new_page().expect("Failed to allocate new root");

            {
                let mut root_page = new_root_arc.write().unwrap();
                use bytemuck::from_bytes_mut;
                let root_node: &mut crate::node::InternalNode =
                    from_bytes_mut(root_page.get_data_mut());

                root_node.header.page_id = new_root_id;
                root_node.header.page_type = crate::node::NODE_TYPE_INTERNAL;
                root_node.header.max_keys = self.max_keys;
                root_node.header.parent_id = 0;

                root_node.header.keys_count = 2;

                root_node.entries[0].page_id = left_page_id;
                root_node.entries[1].key = key;
                root_node.entries[1].page_id = right_page_id;
            }

            *self.root_page_id.write().unwrap() = Some(new_root_id);
            bpm.unpin_page(new_root_id, true);

            let left_arc = bpm
                .fetch_page(left_page_id)
                .expect("Failed to fetch left child");
            {
                let mut left_page = left_arc.write().unwrap();
                let header: &mut crate::node::BPlusTreeHeader =
                    bytemuck::from_bytes_mut(&mut left_page.get_data_mut()[0..24]);
                header.parent_id = new_root_id;
            }
            bpm.unpin_page(left_page_id, true);

            let right_arc = bpm
                .fetch_page(right_page_id)
                .expect("Failed to fetch right child");
            {
                let mut right_page = right_arc.write().unwrap();
                let header: &mut crate::node::BPlusTreeHeader =
                    bytemuck::from_bytes_mut(&mut right_page.get_data_mut()[0..24]);
                header.parent_id = new_root_id;
            }
            bpm.unpin_page(right_page_id, true);

            return;
        }

        // SCENARIO 2: A parent exists. We must fetch it and insert the routing key.
        let mut bpm = self.bpm.lock().unwrap();
        let parent_arc = bpm
            .fetch_page(parent_id)
            .expect("Failed to fetch parent page");

        let mut needs_split = false;

        {
            let mut parent_page = parent_arc.write().unwrap();
            use bytemuck::from_bytes_mut;
            let parent_node: &mut crate::node::InternalNode =
                from_bytes_mut(parent_page.get_data_mut());

            if parent_node.insert_routing(key, right_page_id).is_err() {
                needs_split = true;
            }
        }

        bpm.unpin_page(parent_id, true);

        drop(bpm);

        // SCENARIO 3: The parent is ALSO full!
        if needs_split {
            self.split_internal_node(parent_id, key, right_page_id);
        }
    }

    /// Splits a full internal node, pushes the middle key up, and distributes the pointers.
    fn split_internal_node(&self, old_page_id: PageId, pending_key: u64, pending_page_id: PageId) {
        let mut bpm = self.bpm.lock().unwrap();

        let (new_page_id, new_page_arc) = bpm
            .new_page()
            .expect("Failed to allocate internal node split");
        let old_page_arc = bpm
            .fetch_page(old_page_id)
            .expect("Failed to fetch old internal node");

        let middle_key: u64;
        let parent_id: PageId;

        let mut children_to_update = Vec::new();

        {
            let mut old_page = old_page_arc.write().unwrap();
            let mut new_page = new_page_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let old_node: &mut crate::node::InternalNode = from_bytes_mut(old_page.get_data_mut());
            let new_node: &mut crate::node::InternalNode = from_bytes_mut(new_page.get_data_mut());

            new_node.header.page_id = new_page_id;
            new_node.header.page_type = crate::node::NODE_TYPE_INTERNAL;
            new_node.header.max_keys = self.max_keys;
            parent_id = old_node.header.parent_id;
            new_node.header.parent_id = parent_id;

            let total_keys = old_node.header.keys_count as usize;
            let split_idx = total_keys / 2;

            middle_key = old_node.entries[split_idx].key;
            new_node.entries[0].page_id = old_node.entries[split_idx].page_id;

            let move_count = total_keys - split_idx - 1;
            for i in 0..move_count {
                new_node.entries[i + 1] = old_node.entries[split_idx + 1 + i];
            }

            old_node.header.keys_count = split_idx as u16;
            new_node.header.keys_count = (move_count + 1) as u16;

            if pending_key < middle_key {
                old_node
                    .insert_routing(pending_key, pending_page_id)
                    .expect("Failed to insert into old internal node");
            } else {
                new_node
                    .insert_routing(pending_key, pending_page_id)
                    .expect("Failed to insert into new internal node");
            }

            let new_count = new_node.header.keys_count as usize;
            for i in 0..new_count {
                children_to_update.push(new_node.entries[i].page_id);
            }
        }

        bpm.unpin_page(old_page_id, true);
        bpm.unpin_page(new_page_id, true);

        for child_page_id in children_to_update {
            let child_arc = bpm
                .fetch_page(child_page_id)
                .expect("Failed to fetch child page");
            {
                let mut child_page = child_arc.write().unwrap();
                let header: &mut crate::node::BPlusTreeHeader =
                    bytemuck::from_bytes_mut(&mut child_page.get_data_mut()[0..24]);
                header.parent_id = new_page_id;
            }
            bpm.unpin_page(child_page_id, true);
        }

        drop(bpm);

        self.insert_into_parent(old_page_id, middle_key, new_page_id, parent_id);
    }

    /// Deletes a key from the B+ Tree.
    pub fn delete(&self, key: u64) {
        let root_id_opt = self.get_root_id();

        // SCENARIO 1: The tree is completely empty. Nothing to delete.
        if root_id_opt.is_none() {
            return;
        }

        let leaf_page_id = self
            .find_leaf_page(key)
            .expect("Tree exists but leaf not found");

        let mut bpm = self.bpm.lock().unwrap();
        let page_arc = bpm
            .fetch_page(leaf_page_id)
            .expect("Failed to fetch leaf page");

        let mut needs_rebalance = false;
        let is_root;

        {
            let mut page = page_arc.write().unwrap();
            let page_data = page.get_data_mut();

            use bytemuck::from_bytes_mut;
            let leaf_node: &mut crate::node::LeafNode = from_bytes_mut(page_data);

            is_root = leaf_node.header.parent_id == 0;

            if leaf_node.remove_kv(key) {
                let min_keys = self.max_keys / 2;

                if leaf_node.header.keys_count < min_keys {
                    needs_rebalance = true;
                }
            }
        }

        bpm.unpin_page(leaf_page_id, true);

        drop(bpm);

        // SCENARIO 2: The Leaf has underflowed.
        if needs_rebalance {
            if is_root {
                self.handle_root_underflow(leaf_page_id);
            } else {
                self.handle_leaf_underflow(leaf_page_id);
            }
        }
    }

    /// Fetches the parent and updates the routing key for a specific child page.
    fn update_parent_routing(&self, parent_id: PageId, child_page_id: PageId, new_key: u64) {
        if parent_id == 0 {
            return;
        }

        let mut bpm = self.bpm.lock().unwrap();
        let parent_arc = bpm.fetch_page(parent_id).expect("Failed to fetch parent");

        {
            let mut parent_page = parent_arc.write().unwrap();
            use bytemuck::from_bytes_mut;
            let parent_node: &mut crate::node::InternalNode =
                from_bytes_mut(parent_page.get_data_mut());

            parent_node.update_routing_key(child_page_id, new_key);
        }

        bpm.unpin_page(parent_id, true);
    }

    /// Attempts to borrow a record from the Left Sibling. Returns true if successful.
    fn attempt_leaf_borrow_left(
        &self,
        leaf_page_id: PageId,
        left_page_id: PageId,
        parent_id: PageId,
        min_keys: u16,
    ) -> bool {
        let mut bpm = self.bpm.lock().unwrap();
        let left_arc = bpm.fetch_page(left_page_id).unwrap();

        let can_borrow = {
            let left_page = left_arc.read().unwrap();
            let left_node: &crate::node::LeafNode = bytemuck::from_bytes(left_page.get_data());
            left_node.header.keys_count > min_keys
        };

        if !can_borrow {
            bpm.unpin_page(left_page_id, false);
            return false;
        }

        let leaf_arc = bpm.fetch_page(leaf_page_id).unwrap();
        let new_smallest_key: u64;

        {
            let mut left_page = left_arc.write().unwrap();
            let mut leaf_page = leaf_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let left_node: &mut crate::node::LeafNode = from_bytes_mut(left_page.get_data_mut());
            let leaf_node: &mut crate::node::LeafNode = from_bytes_mut(leaf_page.get_data_mut());

            let (k, v) = left_node.pop_back().unwrap();
            leaf_node.insert_kv(k, v).unwrap();

            new_smallest_key = leaf_node.entries[0].key;
        }

        bpm.unpin_page(left_page_id, true);
        bpm.unpin_page(leaf_page_id, true);
        drop(bpm);

        self.update_parent_routing(parent_id, leaf_page_id, new_smallest_key);
        true
    }

    /// Attempts to borrow a record from the Right Sibling. Returns true if successful.
    fn attempt_leaf_borrow_right(
        &self,
        leaf_page_id: PageId,
        right_page_id: PageId,
        parent_id: PageId,
        min_keys: u16,
    ) -> bool {
        let mut bpm = self.bpm.lock().unwrap();
        let right_arc = bpm.fetch_page(right_page_id).unwrap();

        let can_borrow = {
            let right_page = right_arc.read().unwrap();
            let right_node: &crate::node::LeafNode = bytemuck::from_bytes(right_page.get_data());
            right_node.header.keys_count > min_keys
        };

        if !can_borrow {
            bpm.unpin_page(right_page_id, false);
            return false;
        }

        let leaf_arc = bpm.fetch_page(leaf_page_id).unwrap();
        let new_right_smallest_key: u64;

        {
            let mut right_page = right_arc.write().unwrap();
            let mut leaf_page = leaf_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let right_node: &mut crate::node::LeafNode = from_bytes_mut(right_page.get_data_mut());
            let leaf_node: &mut crate::node::LeafNode = from_bytes_mut(leaf_page.get_data_mut());

            let (k, v) = right_node.pop_front().unwrap();
            leaf_node.insert_kv(k, v).unwrap();

            new_right_smallest_key = right_node.entries[0].key;
        }

        bpm.unpin_page(right_page_id, true);
        bpm.unpin_page(leaf_page_id, true);
        drop(bpm);

        self.update_parent_routing(parent_id, right_page_id, new_right_smallest_key);
        true
    }

    /// Orchestrates fixing a leaf node that has dropped below 50% capacity.
    fn handle_leaf_underflow(&self, leaf_page_id: PageId) {
        let min_keys = self.max_keys / 2;
        let prev_page_id: PageId;
        let next_page_id: PageId;
        let parent_id: PageId;

        {
            let mut bpm = self.bpm.lock().unwrap();
            let page_arc = bpm.fetch_page(leaf_page_id).unwrap();

            {
                let page = page_arc.read().unwrap();
                let leaf: &crate::node::LeafNode = bytemuck::from_bytes(page.get_data());

                prev_page_id = leaf.prev_page_id;
                next_page_id = leaf.next_page_id;
                parent_id = leaf.header.parent_id;
            }

            bpm.unpin_page(leaf_page_id, false);
        }

        if prev_page_id != 0
            && self.attempt_leaf_borrow_left(leaf_page_id, prev_page_id, parent_id, min_keys)
        {
            return;
        }

        if next_page_id != 0
            && self.attempt_leaf_borrow_right(leaf_page_id, next_page_id, parent_id, min_keys)
        {
            return;
        }

        self.merge_leaf_nodes(leaf_page_id, prev_page_id, next_page_id, parent_id);
    }

    /// Merges two leaf nodes together and pushes the deletion up to the parent.
    fn merge_leaf_nodes(
        &self,
        leaf_page_id: PageId,
        prev_page_id: PageId,
        next_page_id: PageId,
        parent_id: PageId,
    ) {
        let mut bpm = self.bpm.lock().unwrap();

        let left_id: PageId;
        let right_id: PageId;

        if prev_page_id != 0 {
            left_id = prev_page_id;
            right_id = leaf_page_id;
        } else {
            left_id = leaf_page_id;
            right_id = next_page_id;
        }

        let right_next_page_id: PageId;

        {
            let left_arc = bpm.fetch_page(left_id).expect("Failed to fetch left node");
            let right_arc = bpm
                .fetch_page(right_id)
                .expect("Failed to fetch right node");

            let mut left_page = left_arc.write().unwrap();
            let mut right_page = right_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let left_node: &mut crate::node::LeafNode = from_bytes_mut(left_page.get_data_mut());
            let right_node: &mut crate::node::LeafNode = from_bytes_mut(right_page.get_data_mut());

            let right_count = right_node.header.keys_count as usize;
            for i in 0..right_count {
                let k = right_node.entries[i].key;
                let v = right_node.entries[i].value;
                left_node
                    .insert_kv(k, v)
                    .expect("Left node overflowed during merge!");
            }

            left_node.next_page_id = right_node.next_page_id;
            right_next_page_id = right_node.next_page_id;

            right_node.header.keys_count = 0;
        }

        bpm.unpin_page(left_id, true);
        bpm.unpin_page(right_id, true);

        if right_next_page_id != 0 {
            let next_arc = bpm
                .fetch_page(right_next_page_id)
                .expect("Failed to fetch next node");
            {
                let mut next_page = next_arc.write().unwrap();
                let next_node: &mut crate::node::LeafNode =
                    bytemuck::from_bytes_mut(next_page.get_data_mut());
                next_node.prev_page_id = left_id;
            }
            bpm.unpin_page(right_next_page_id, true);
        }

        drop(bpm);

        self.delete_from_parent(parent_id, right_id);
    }

    /// Deletes a routing key from a parent node. Triggers recursive rebalancing if needed.
    fn delete_from_parent(&self, parent_id: PageId, child_page_id: PageId) {
        if parent_id == 0 {
            return;
        }

        let mut bpm = self.bpm.lock().unwrap();
        let parent_arc = bpm.fetch_page(parent_id).expect("Failed to fetch parent");

        let mut needs_rebalance = false;
        let is_root;

        {
            let mut parent_page = parent_arc.write().unwrap();
            use bytemuck::from_bytes_mut;
            let parent_node: &mut crate::node::InternalNode =
                from_bytes_mut(parent_page.get_data_mut());

            is_root = parent_node.header.parent_id == 0;

            parent_node.remove_routing(child_page_id);

            let min_keys = self.max_keys / 2;
            if parent_node.header.keys_count < min_keys {
                needs_rebalance = true;
            }
        }

        bpm.unpin_page(parent_id, true);
        drop(bpm);

        if needs_rebalance {
            if is_root {
                self.handle_root_underflow(parent_id);
            } else {
                self.handle_internal_underflow(parent_id);
            }
        }
    }

    /// Handles the special case where the root node underflows.
    /// Collapses the tree height if the root becomes redundant.
    fn handle_root_underflow(&self, root_page_id: PageId) {
        let mut bpm = self.bpm.lock().unwrap();
        let root_arc = bpm.fetch_page(root_page_id).unwrap();

        let new_root_id: Option<PageId>;
        let mut tree_is_empty = false;

        {
            let page = root_arc.read().unwrap();
            use bytemuck::from_bytes;
            let header: &crate::node::BPlusTreeHeader = from_bytes(&page.get_data()[0..24]);

            if header.is_leaf() {
                if header.keys_count == 0 {
                    // SCENARIO 1: The very last record was deleted.
                    tree_is_empty = true;
                    new_root_id = None;
                } else {
                    new_root_id = Some(root_page_id);
                }
            } else {
                let internal_node: &crate::node::InternalNode = from_bytes(page.get_data());
                if internal_node.header.keys_count == 1 {
                    // SCENARIO 2: The internal root is redundant. Promote its only child!
                    new_root_id = Some(internal_node.entries[0].page_id);
                } else {
                    new_root_id = Some(root_page_id);
                }
            }
        }

        bpm.unpin_page(root_page_id, false);

        if tree_is_empty {
            *self.root_page_id.write().unwrap() = None;
            return;
        }

        if let Some(child_id) = new_root_id
            && child_id != root_page_id
        {
            *self.root_page_id.write().unwrap() = Some(child_id);

            let child_arc = bpm.fetch_page(child_id).unwrap();
            {
                let mut child_page = child_arc.write().unwrap();
                let header: &mut crate::node::BPlusTreeHeader =
                    bytemuck::from_bytes_mut(&mut child_page.get_data_mut()[0..24]);
                header.parent_id = 0;
            }
            bpm.unpin_page(child_id, true);
        }
    }

    /// Attempts to borrow a pointer from the Left Sibling for an Internal Node.
    fn attempt_internal_borrow_left(
        &self,
        internal_id: PageId,
        left_id: PageId,
        parent_id: PageId,
        min_keys: u16,
    ) -> bool {
        let mut bpm = self.bpm.lock().unwrap();

        let left_arc = bpm.fetch_page(left_id).unwrap();

        let can_borrow = {
            let left_page = left_arc.read().unwrap();
            let left_node: &crate::node::InternalNode = bytemuck::from_bytes(left_page.get_data());
            left_node.header.keys_count > min_keys
        };

        if !can_borrow {
            bpm.unpin_page(left_id, false);
            return false;
        }

        let internal_arc = bpm.fetch_page(internal_id).unwrap();
        let parent_arc = bpm.fetch_page(parent_id).unwrap();
        let transferred_child_id: PageId;

        {
            let mut left_page = left_arc.write().unwrap();
            let mut internal_page = internal_arc.write().unwrap();
            let mut parent_page = parent_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let left_node: &mut crate::node::InternalNode =
                from_bytes_mut(left_page.get_data_mut());
            let internal_node: &mut crate::node::InternalNode =
                from_bytes_mut(internal_page.get_data_mut());
            let parent_node: &mut crate::node::InternalNode =
                from_bytes_mut(parent_page.get_data_mut());

            let mut parent_idx = 0;
            for i in 1..(parent_node.header.keys_count as usize) {
                if parent_node.entries[i].page_id == internal_id {
                    parent_idx = i;
                    break;
                }
            }

            let parent_key = parent_node.entries[parent_idx].key;

            let (left_last_key, left_last_ptr) = left_node.pop_back().unwrap();
            transferred_child_id = left_last_ptr;

            internal_node.push_front(left_last_ptr, parent_key);

            parent_node.entries[parent_idx].key = left_last_key;
        }

        bpm.unpin_page(left_id, true);
        bpm.unpin_page(internal_id, true);
        bpm.unpin_page(parent_id, true);

        let child_arc = bpm.fetch_page(transferred_child_id).unwrap();
        {
            let mut child_page = child_arc.write().unwrap();
            let header: &mut crate::node::BPlusTreeHeader =
                bytemuck::from_bytes_mut(&mut child_page.get_data_mut()[0..24]);
            header.parent_id = internal_id;
        }
        bpm.unpin_page(transferred_child_id, true);

        true
    }

    /// Attempts to borrow a pointer from the Right Sibling for an Internal Node.
    fn attempt_internal_borrow_right(
        &self,
        internal_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        min_keys: u16,
    ) -> bool {
        let mut bpm = self.bpm.lock().unwrap();

        let right_arc = bpm.fetch_page(right_id).unwrap();

        let can_borrow = {
            let right_page = right_arc.read().unwrap();
            let right_node: &crate::node::InternalNode =
                bytemuck::from_bytes(right_page.get_data());
            right_node.header.keys_count > min_keys
        };

        if !can_borrow {
            bpm.unpin_page(right_id, false);
            return false;
        }

        let internal_arc = bpm.fetch_page(internal_id).unwrap();
        let parent_arc = bpm.fetch_page(parent_id).unwrap();
        let transferred_child_id: PageId;

        {
            let mut right_page = right_arc.write().unwrap();
            let mut internal_page = internal_arc.write().unwrap();
            let mut parent_page = parent_arc.write().unwrap();

            use bytemuck::from_bytes_mut;
            let right_node: &mut crate::node::InternalNode =
                from_bytes_mut(right_page.get_data_mut());
            let internal_node: &mut crate::node::InternalNode =
                from_bytes_mut(internal_page.get_data_mut());
            let parent_node: &mut crate::node::InternalNode =
                from_bytes_mut(parent_page.get_data_mut());

            let mut parent_idx = 0;
            for i in 1..(parent_node.header.keys_count as usize) {
                if parent_node.entries[i].page_id == right_id {
                    parent_idx = i;
                    break;
                }
            }

            let parent_key = parent_node.entries[parent_idx].key;

            let (right_first_ptr, right_first_key) = right_node.pop_front().unwrap();
            transferred_child_id = right_first_ptr;

            internal_node
                .insert_routing(parent_key, right_first_ptr)
                .unwrap();

            parent_node.entries[parent_idx].key = right_first_key;
        }

        bpm.unpin_page(right_id, true);
        bpm.unpin_page(internal_id, true);
        bpm.unpin_page(parent_id, true);

        let child_arc = bpm.fetch_page(transferred_child_id).unwrap();
        {
            let mut child_page = child_arc.write().unwrap();
            let header: &mut crate::node::BPlusTreeHeader =
                bytemuck::from_bytes_mut(&mut child_page.get_data_mut()[0..24]);
            header.parent_id = internal_id;
        }
        bpm.unpin_page(transferred_child_id, true);

        true
    }

    /// Orchestrates fixing an internal node that has dropped below 50% capacity.
    fn handle_internal_underflow(&self, internal_page_id: PageId) {
        let min_keys = self.max_keys / 2;
        let parent_id: PageId;

        {
            let mut bpm = self.bpm.lock().unwrap();
            let page_arc = bpm.fetch_page(internal_page_id).unwrap();
            {
                let page = page_arc.read().unwrap();
                let header: &crate::node::BPlusTreeHeader =
                    bytemuck::from_bytes(&page.get_data()[0..24]);
                parent_id = header.parent_id;
            }
            bpm.unpin_page(internal_page_id, false);
        }

        if parent_id == 0 {
            return;
        }

        let mut left_sibling_id = 0;
        let mut right_sibling_id = 0;

        {
            let mut bpm = self.bpm.lock().unwrap();
            let parent_arc = bpm.fetch_page(parent_id).unwrap();
            {
                let page = parent_arc.read().unwrap();
                let parent_node: &crate::node::InternalNode = bytemuck::from_bytes(page.get_data());

                let count = parent_node.header.keys_count as usize;
                for i in 0..count {
                    if parent_node.entries[i].page_id == internal_page_id {
                        if i > 0 {
                            left_sibling_id = parent_node.entries[i - 1].page_id;
                        }
                        if i < count - 1 {
                            right_sibling_id = parent_node.entries[i + 1].page_id;
                        }
                        break;
                    }
                }
            }
            bpm.unpin_page(parent_id, false);
        }

        if left_sibling_id != 0
            && self.attempt_internal_borrow_left(
                internal_page_id,
                left_sibling_id,
                parent_id,
                min_keys,
            )
        {
            return;
        }

        if right_sibling_id != 0
            && self.attempt_internal_borrow_right(
                internal_page_id,
                right_sibling_id,
                parent_id,
                min_keys,
            )
        {
            return;
        }

        self.merge_internal_nodes(
            internal_page_id,
            left_sibling_id,
            right_sibling_id,
            parent_id,
        );
    }

    /// Merges two internal nodes by pulling a routing key down from the parent.
    fn merge_internal_nodes(
        &self,
        internal_id: PageId,
        left_sibling_id: PageId,
        right_sibling_id: PageId,
        parent_id: PageId,
    ) {
        let (left_id, right_id) = if left_sibling_id != 0 {
            (left_sibling_id, internal_id)
        } else {
            (internal_id, right_sibling_id)
        };

        let parent_key: u64;
        let mut children_to_update = Vec::new();

        {
            let mut bpm = self.bpm.lock().unwrap();
            let parent_arc = bpm.fetch_page(parent_id).expect("Failed to fetch parent");
            {
                let parent_page = parent_arc.read().unwrap();
                let parent_node: &crate::node::InternalNode =
                    bytemuck::from_bytes(parent_page.get_data());

                let mut key = 0;
                for i in 1..(parent_node.header.keys_count as usize) {
                    if parent_node.entries[i].page_id == right_id {
                        key = parent_node.entries[i].key;
                        break;
                    }
                }
                parent_key = key;
            }
            bpm.unpin_page(parent_id, false);
        }

        {
            let mut bpm = self.bpm.lock().unwrap();
            let left_arc = bpm.fetch_page(left_id).unwrap();
            let right_arc = bpm.fetch_page(right_id).unwrap();

            {
                let mut left_page = left_arc.write().unwrap();
                let mut right_page = right_arc.write().unwrap();

                use bytemuck::from_bytes_mut;
                let left_node: &mut crate::node::InternalNode =
                    from_bytes_mut(left_page.get_data_mut());
                let right_node: &mut crate::node::InternalNode =
                    from_bytes_mut(right_page.get_data_mut());

                let mut left_count = left_node.header.keys_count as usize;
                let right_count = right_node.header.keys_count as usize;

                left_node.entries[left_count].key = parent_key;
                left_node.entries[left_count].page_id = right_node.entries[0].page_id;
                children_to_update.push(right_node.entries[0].page_id);
                left_count += 1;

                for i in 1..right_count {
                    left_node.entries[left_count].key = right_node.entries[i].key;
                    left_node.entries[left_count].page_id = right_node.entries[i].page_id;
                    children_to_update.push(right_node.entries[i].page_id);
                    left_count += 1;
                }

                left_node.header.keys_count = left_count as u16;
                right_node.header.keys_count = 0;
            }

            bpm.unpin_page(left_id, true);
            bpm.unpin_page(right_id, true);
        }

        {
            let mut bpm = self.bpm.lock().unwrap();
            for child_id in children_to_update {
                let child_arc = bpm.fetch_page(child_id).unwrap();
                {
                    let mut child_page = child_arc.write().unwrap();
                    let header: &mut crate::node::BPlusTreeHeader =
                        bytemuck::from_bytes_mut(&mut child_page.get_data_mut()[0..24]);
                    header.parent_id = left_id;
                }
                bpm.unpin_page(child_id, true);
            }
        }

        self.delete_from_parent(parent_id, right_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_pool_manager::BufferPoolManager;
    use crate::disk_manager::DiskManager;
    use std::fs;

    #[test]
    fn test_bplus_tree_stress_insert() {
        let test_file = "test_stress_tree.db";
        let _ = fs::remove_file(test_file);

        let disk_manager = DiskManager::new(test_file).unwrap();

        let bpm = Arc::new(Mutex::new(BufferPoolManager::new(50, disk_manager)));

        let tree = BPlusTree::new(bpm, 4);

        println!("Starting aggressive inserts...");
        for i in 1..=100 {
            tree.insert(i, i * 10);
        }

        println!("Verifying tree traversal...");
        for i in 1..=100 {
            let result = tree.search(i);
            assert_eq!(result, Some(i * 10), "Failed to find key {}", i);
        }

        assert_eq!(tree.search(0), None);
        assert_eq!(tree.search(101), None);
        assert_eq!(tree.search(999), None);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_bplus_tree_stress_delete() {
        let test_file = "test_stress_delete.db";
        let _ = fs::remove_file(test_file);

        let disk_manager = DiskManager::new(test_file).unwrap();
        let bpm = Arc::new(Mutex::new(BufferPoolManager::new(50, disk_manager)));
        let tree = BPlusTree::new(bpm, 4);

        println!("Inserting 100 records...");
        for i in 1..=100 {
            tree.insert(i, i * 10);
        }

        println!("Deleting all 100 records...");
        for i in 1..=100 {
            tree.delete(i);

            assert_eq!(tree.search(i), None, "Key {} should have been deleted!", i);
        }

        assert!(
            tree.get_root_id().is_none(),
            "The tree should be completely empty after deleting all records"
        );

        let _ = fs::remove_file(test_file);
    }
}
