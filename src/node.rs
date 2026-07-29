use crate::disk_manager::PageId;
use bytemuck::{Pod, Zeroable};

/// Represents the type of a B+ Tree node.
/// We use constants instead of a standard Rust Enum because bytemuck
/// needs to safely cast raw bytes into these exact values.
pub const NODE_TYPE_INVALID: u8 = 0;
pub const NODE_TYPE_LEAF: u8 = 1;
pub const NODE_TYPE_INTERNAL: u8 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
pub struct BPlusTreeHeader {
    /// The ID of the current page
    pub page_id: PageId, // Offset 0..4

    /// Log Sequence Number (for recovery)
    pub lsn: u32, // Offset 4..8

    /// 0: Invalid, 1: Leaf, 2: Internal
    pub page_type: u8, // Offset 8..9

    /// Explicit padding to align the next u32 to a 4-byte boundary
    pub _padding1: [u8; 3], // Offset 9..12

    /// PageId of the parent node
    pub parent_id: PageId, // Offset 12..16

    /// Number of keys currently in this node
    pub keys_count: u16, // Offset 16..18

    /// Maximum capacity of the node
    pub max_keys: u16, // Offset 18..20

    /// Padding to bring the total header size to exactly 24 bytes
    pub _padding2: [u8; 4], // Offset 20..24
}

impl BPlusTreeHeader {
    /// Creates a new header with default values.
    pub fn new(page_id: PageId, page_type: u8, max_keys: u16) -> Self {
        Self {
            page_id,
            lsn: 0,
            page_type,
            _padding1: [0; 3],
            parent_id: 0,
            keys_count: 0,
            max_keys,
            _padding2: [0; 4],
        }
    }

    /// Helper to safely check if the node is a leaf
    pub fn is_leaf(&self) -> bool {
        self.page_type == NODE_TYPE_LEAF
    }

    /// Helper to safely check if the node is the root (no parent)
    pub fn is_root(&self) -> bool {
        self.parent_id == 0
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
pub struct InternalEntry {
    /// The routing key
    pub key: u64, // 8 bytes

    /// The page ID of the child node
    pub page_id: PageId, // 4 bytes

    /// Explicit padding to align to 8-byte boundaries
    pub _padding: u32, // 4 bytes (Total: 16 bytes)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InternalNode {
    pub header: BPlusTreeHeader,       // 24 bytes
    pub entries: [InternalEntry; 254], // 4064 bytes
    pub _padding: [u8; 8],             // 8 bytes (Total: 4096 bytes)
}

unsafe impl Zeroable for InternalNode {}
unsafe impl Pod for InternalNode {}

impl InternalNode {
    pub fn value_at(&self, index: usize) -> PageId {
        self.entries[index].page_id
    }

    pub fn key_at(&self, index: usize) -> u64 {
        self.entries[index].key
    }

    /// Performs a binary search to find the correct child page ID for a given key.
    pub fn lookup(&self, search_key: u64) -> PageId {
        let count = self.header.keys_count as usize;

        if count <= 1 {
            return self.entries[0].page_id;
        }

        let valid_entries = &self.entries[1..count];

        let pos = valid_entries.partition_point(|entry| entry.key <= search_key);

        self.entries[pos].page_id
    }

    /// Inserts a new routing key and child page ID into the internal node.
    /// Returns an Error if the node is at maximum capacity.
    pub fn insert_routing(&mut self, key: u64, page_id: PageId) -> Result<(), &'static str> {
        let count = self.header.keys_count as usize;

        if count >= self.header.max_keys as usize {
            return Err("Internal node is full, requires splitting");
        }

        let valid_entries = &self.entries[1..count];

        let insert_idx = match valid_entries.binary_search_by_key(&key, |entry| entry.key) {
            Ok(pos) => pos + 1,
            Err(pos) => pos + 1,
        };

        for i in (insert_idx..count).rev() {
            self.entries[i + 1] = self.entries[i];
        }

        self.entries[insert_idx].key = key;
        self.entries[insert_idx].page_id = page_id;

        self.header.keys_count += 1;

        Ok(())
    }

    /// Finds the child pointer and updates its routing key.
    pub fn update_routing_key(&mut self, target_page_id: PageId, new_key: u64) {
        let count = self.header.keys_count as usize;

        for i in 1..count {
            if self.entries[i].page_id == target_page_id {
                self.entries[i].key = new_key;
                return;
            }
        }
    }

    /// Removes a routing key and its associated child pointer.
    pub fn remove_routing(&mut self, target_page_id: PageId) {
        let count = self.header.keys_count as usize;
        let mut remove_idx = 0;

        for i in 1..count {
            if self.entries[i].page_id == target_page_id {
                remove_idx = i;
                break;
            }
        }

        if remove_idx == 0 {
            return;
        }

        for i in remove_idx..(count - 1) {
            self.entries[i] = self.entries[i + 1];
        }

        self.header.keys_count -= 1;
    }

    /// For borrowing from the left sibling. Returns the last routing key and pointer.
    pub fn pop_back(&mut self) -> Option<(u64, PageId)> {
        let count = self.header.keys_count as usize;
        if count <= 1 {
            return None;
        }

        let entry = &self.entries[count - 1];
        let kv = (entry.key, entry.page_id);

        self.header.keys_count -= 1;
        Some(kv)
    }

    /// For borrowing from the right sibling. Returns the left-most pointer and the first routing key.
    pub fn pop_front(&mut self) -> Option<(PageId, u64)> {
        let count = self.header.keys_count as usize;
        if count <= 1 {
            return None;
        }

        let leftmost_ptr = self.entries[0].page_id;
        let first_key = self.entries[1].key;

        for i in 0..(count - 1) {
            self.entries[i].page_id = self.entries[i + 1].page_id;
            self.entries[i].key = self.entries[i + 1].key;
        }

        self.header.keys_count -= 1;
        Some((leftmost_ptr, first_key))
    }

    /// When borrowing from the left, a new pointer and routing key come in from the front.
    pub fn push_front(&mut self, new_leftmost_ptr: PageId, routing_key: u64) {
        let count = self.header.keys_count as usize;

        for i in (0..count).rev() {
            self.entries[i + 1].key = self.entries[i].key;
            self.entries[i + 1].page_id = self.entries[i].page_id;
        }

        self.entries[1].key = routing_key;

        self.entries[0].page_id = new_leftmost_ptr;
        self.header.keys_count += 1;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
pub struct LeafEntry {
    /// The actual search key
    pub key: u64, // 8 bytes

    /// The value (e.g., a Record ID)
    pub value: u64, // 8 bytes (Total: 16 bytes)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LeafNode {
    pub header: BPlusTreeHeader,   // 24 bytes
    pub next_page_id: PageId,      // 4 bytes
    pub prev_page_id: PageId,      // 4 bytes
    pub entries: [LeafEntry; 254], // 4064 bytes (Total: 4096 bytes!)
}

unsafe impl Zeroable for LeafNode {}
unsafe impl Pod for LeafNode {}

impl LeafNode {
    /// Gets the key at a specific index
    pub fn key_at(&self, index: usize) -> u64 {
        self.entries[index].key
    }

    /// Gets the value at a specific index
    pub fn value_at(&self, index: usize) -> u64 {
        self.entries[index].value
    }

    /// Performs a binary search to find an exact key match.
    /// Returns `Some(value)` if found, `None` otherwise.
    pub fn lookup(&self, search_key: u64) -> Option<u64> {
        let count = self.header.keys_count as usize;
        if count == 0 {
            return None;
        }

        let valid_entries = &self.entries[0..count];

        match valid_entries.binary_search_by_key(&search_key, |entry| entry.key) {
            Ok(index) => Some(valid_entries[index].value),
            Err(_) => None,
        }
    }

    /// Inserts a key-value pair into the leaf, keeping the entries sorted.
    /// Returns an Error if the leaf is full or if the key already exists.
    pub fn insert_kv(&mut self, key: u64, value: u64) -> Result<(), &'static str> {
        let count = self.header.keys_count as usize;

        if count >= self.header.max_keys as usize {
            return Err("Leaf node is full, requires splitting");
        }

        let insert_idx = match self.entries[0..count].binary_search_by_key(&key, |entry| entry.key)
        {
            Ok(_) => return Err("Duplicate keys are not supported"),
            Err(pos) => pos,
        };

        for i in (insert_idx..count).rev() {
            self.entries[i + 1] = self.entries[i];
        }

        self.entries[insert_idx].key = key;
        self.entries[insert_idx].value = value;

        self.header.keys_count += 1;

        Ok(())
    }

    /// Removes a key-value pair from the leaf node.
    /// Returns true if the key was found and removed, false otherwise.
    pub fn remove_kv(&mut self, search_key: u64) -> bool {
        let count = self.header.keys_count as usize;

        if count == 0 {
            return false;
        }

        let valid_entries = &self.entries[0..count];
        let remove_idx = match valid_entries.binary_search_by_key(&search_key, |entry| entry.key) {
            Ok(idx) => idx,
            Err(_) => return false,
        };

        for i in remove_idx..(count - 1) {
            self.entries[i] = self.entries[i + 1];
        }

        self.header.keys_count -= 1;
        true
    }

    /// Removes and returns the largest key-value pair (the last element).
    /// Used when this node acts as a left sibling lending to an underflowed right sibling.
    pub fn pop_back(&mut self) -> Option<(u64, u64)> {
        let count = self.header.keys_count as usize;

        if count == 0 {
            return None;
        }

        let entry = &self.entries[count - 1];
        let kv = (entry.key, entry.value);

        self.header.keys_count -= 1;

        Some(kv)
    }

    /// Removes and returns the smallest key-value pair (the first element).
    /// Used when this node acts as a right sibling lending to an underflowed left sibling.
    pub fn pop_front(&mut self) -> Option<(u64, u64)> {
        let count = self.header.keys_count as usize;

        if count == 0 {
            return None;
        }

        let entry = &self.entries[0];
        let kv = (entry.key, entry.value);

        for i in 0..(count - 1) {
            self.entries[i] = self.entries[i + 1];
        }

        self.header.keys_count -= 1;

        Some(kv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{from_bytes, from_bytes_mut};

    #[test]
    fn test_header_serialization() {
        let mut raw_page_data = [0u8; 4096];

        let header_slice = &mut raw_page_data[0..24];
        let header: &mut BPlusTreeHeader = from_bytes_mut(header_slice);

        header.page_id = 42;
        header.page_type = NODE_TYPE_LEAF;
        header.keys_count = 5;

        assert_eq!(raw_page_data[0], 42);
        assert_eq!(raw_page_data[8], NODE_TYPE_LEAF);

        let read_header: &BPlusTreeHeader = from_bytes(&raw_page_data[0..24]);
        assert_eq!(read_header.page_id, 42);
        assert_eq!(read_header.keys_count, 5);
        assert!(read_header.is_leaf());
    }

    #[test]
    fn test_internal_node_layout() {
        #[repr(C, align(8))]
        struct AlignedArray([u8; 4096]);

        let mut aligned_memory = AlignedArray([0u8; 4096]);
        let raw_page_data = &mut aligned_memory.0;

        {
            let internal_node: &mut InternalNode = from_bytes_mut(raw_page_data);

            internal_node.header.page_id = 1;
            internal_node.header.page_type = NODE_TYPE_INTERNAL;
            internal_node.header.keys_count = 2;

            internal_node.entries[0].page_id = 2;

            internal_node.entries[1].key = 50;
            internal_node.entries[1].page_id = 3;

            internal_node.entries[2].key = 100;
            internal_node.entries[2].page_id = 4;

            assert_eq!(internal_node.value_at(0), 2);
            assert_eq!(internal_node.key_at(1), 50);
            assert_eq!(internal_node.value_at(2), 4);
        }

        assert_eq!(raw_page_data[40], 50);
    }

    #[test]
    fn test_leaf_node_layout() {
        #[repr(C, align(8))]
        struct AlignedArray([u8; 4096]);

        let mut aligned_memory = AlignedArray([0u8; 4096]);
        let raw_page_data = &mut aligned_memory.0;

        {
            let leaf_node: &mut LeafNode = from_bytes_mut(raw_page_data);

            leaf_node.header.page_id = 5;
            leaf_node.header.page_type = NODE_TYPE_LEAF;
            leaf_node.header.keys_count = 2;

            leaf_node.next_page_id = 6;
            leaf_node.prev_page_id = 4;

            leaf_node.entries[0].key = 10;
            leaf_node.entries[0].value = 1000;

            leaf_node.entries[1].key = 20;
            leaf_node.entries[1].value = 2000;

            assert_eq!(leaf_node.key_at(0), 10);
            assert_eq!(leaf_node.value_at(1), 2000);
        }

        assert_eq!(raw_page_data[32], 10);
        assert_eq!(raw_page_data[40], 232);
        assert_eq!(raw_page_data[41], 3);
    }

    #[test]
    fn test_internal_node_lookup() {
        #[repr(C, align(8))]
        struct AlignedArray([u8; 4096]);
        let mut aligned_memory = AlignedArray([0u8; 4096]);

        let internal_node: &mut InternalNode = bytemuck::from_bytes_mut(&mut aligned_memory.0);
        internal_node.header.keys_count = 3;

        // [Pointer 2] < 50 <= [Pointer 3] < 100 <= [Pointer 4]
        internal_node.entries[0].page_id = 2; // Left-most pointer

        internal_node.entries[1].key = 50;
        internal_node.entries[1].page_id = 3;

        internal_node.entries[2].key = 100;
        internal_node.entries[2].page_id = 4;

        // Test the routing logic!
        assert_eq!(internal_node.lookup(10), 2); // < 50, follow left-most
        assert_eq!(internal_node.lookup(50), 3); // == 50, follow index 1
        assert_eq!(internal_node.lookup(75), 3); // >= 50 and < 100, follow index 1
        assert_eq!(internal_node.lookup(100), 4); // == 100, follow index 2
        assert_eq!(internal_node.lookup(999), 4); // >= 100, follow index 2
    }
}
