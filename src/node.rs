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
}
