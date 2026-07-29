# Project Specification: Lithox Storage Engine

## 1. Overview
Lithox implements a persistent, on-disk B+ Tree storage engine in Rust. By utilizing a Buffer Pool Manager and Disk Manager, the architecture avoids Rust's traditional graph/tree pointer limitations, relying entirely on sequential Page IDs to link nodes.

## 2. Goals & Objectives
- **Memory Safety & Rust Mastery:** Leverage Rust's ownership, borrowing, and concurrency paradigms in a systems context.
- **Persistence:** Ensure data is durable and securely written to disk pages rather than remaining purely in-memory.
- **Scalability:** Handle datasets larger than available RAM through intelligent page caching and eviction.
- **Concurrency (Future):** Support concurrent read/write operations using lock coupling (crabbing) and prepare for ACID transaction compliance.

## 3. System Architecture

The database is structured in three primary layers, operating strictly bottom-up:

### 3.1. Disk Manager (Storage Layer)
Responsible for raw file I/O operations.
- **Page Size:** 4096 Bytes (4KB).
- **Functionality:** Maps a `PageId` (`u32`) to a physical offset in the database file (Offset = `PageId` * 4096).
- **Core Methods:** 
  - `read_page(page_id: PageId) -> Result<Page, Error>`
  - `write_page(page_id: PageId, page: &Page) -> Result<(), Error>`
  - `allocate_page() -> PageId`
  - `deallocate_page(page_id: PageId)` (Uses an intrusive linked list via Page 0 for O(1) recycling).

### 3.2. Buffer Pool Manager (Caching Layer)
Acts as the middleman between memory and disk, minimizing slow disk I/O.
- **Capacity:** Fixed memory limit (e.g., 1024 pages).
- **State Management:** Tracks dirty pages (modified in memory) and pinned pages (currently in use by the tree).
- **Eviction Policy:** Clock algorithm (Second Chance) to swap out unpinned, clean pages when the pool reaches capacity.
- **Core Methods:**
  - `fetch_page(page_id: PageId) -> Option<Arc<RwLock<Page>>>`
  - `unpin_page(page_id: PageId, is_dirty: bool)`
  - `new_page() -> Option<(PageId, Arc<RwLock<Page>>)>`

### 3.3. B+ Tree Index (Data Layer)
The core logic for searching, inserting, and deleting records.
- **Nodes:** Differentiated into `InternalNode` (routing keys) and `LeafNode` (actual key-value pairs).
- **Pointers:** Uses `PageId` instead of memory pointers (`Box` or `Rc`).
- **Core Methods:**
  - `insert(key: K, value: V)`: Finds the target leaf, handles splits recursively if the leaf capacity is exceeded.
  - `search(key: &K) -> Option<V>`: Traverses internal nodes to locate the target leaf and retrieves the value.
  - `delete(key: &K)`: Marks records as deleted or handles node merging/borrowing (coalescing) if capacity drops below 50%.

## 4. Page & Data Layout

Data must be serialized into 4KB byte arrays.

### Common Page Header (24 Bytes)
| Field | Size | Description |
| :--- | :--- | :--- |
| `page_id` | 4 Bytes | The ID of the current page |
| `lsn` | 4 Bytes | Log Sequence Number (for recovery) |
| `page_type` | 1 Byte | 0: Invalid, 1: Leaf, 2: Internal |
| `parent_id` | 4 Bytes | `PageId` of the parent node |
| `keys_count`| 2 Bytes | Number of keys currently in this node |
| `max_keys` | 2 Bytes | Maximum capacity of the node |
| `reserved` | 7 Bytes | Padding for future use / alignment |

### Leaf Node Layout
- **Header:** 24 Bytes (as defined above).
- **Next/Prev Page IDs:** 8 Bytes (Double linked list for range scans).
- **Payload:** Array of `(Key, Value)` pairs.

### Internal Node Layout
- **Header:** 24 Bytes (as defined above).
- **Payload:** Array of `(Key, PageId)` pairs.

## 5. Development Phases
1. **Phase 1: Storage Engine Bootstrap** - Implement `DiskManager` and standard `Page` definitions. *(Complete)*
2. **Phase 2: Memory Management** - Implement `BufferPoolManager` with a `ClockReplacer`. *(Complete)*
3. **Phase 3: Serialization/Deserialization** - Write utilities to convert Rust structs to/from 4KB `[u8; 4096]` arrays. *(Complete)*
4. **Phase 4: B+ Tree Logic** - Implement Search, Insert (with splitting), and Delete (with merging, borrowing, and dynamic root collapsing). *(Complete)*
5. **Phase 5: Database REPL (v0.2.0)** - Build an interactive terminal prompt to query and manipulate the database in real-time. *(Pending)*
6. **Phase 6: Concurrency Control (v0.3.0)** - Add page-level read/write latches (`RwLock`) and implement crabbing. *(Pending)*

## 6. References & Inspiration
- CMU 15-445 Database Systems (BusTub Architecture)
- ARIES: A Transaction Recovery Method
- Rust Standard Library Documentation