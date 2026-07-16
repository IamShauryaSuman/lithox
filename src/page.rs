use crate::disk_manager::{PAGE_SIZE, PageId};

/// Represents a single page in memory.
pub struct Page {
    /// The actual raw data from the disk.
    data: [u8; PAGE_SIZE],

    /// The logical ID of this page. None if the page is empty/unused.
    page_id: Option<PageId>,

    /// Number of threads currently using this page.
    /// A page cannot be evicted from memory if pin_count > 0.
    pin_count: usize,

    /// True if the page was modified in memory and differs from the disk version.
    is_dirty: bool,
}

impl Page {
    /// Creates a new, empty page.
    pub fn new() -> Self {
        Self {
            data: [0; PAGE_SIZE],
            page_id: None,
            pin_count: 0,
            is_dirty: false,
        }
    }

    /// Returns an immutable reference to the raw byte array.
    pub fn get_data(&self) -> &[u8; PAGE_SIZE] {
        &self.data
    }

    /// Returns a mutable reference to the raw byte array.
    /// Our B+ Tree will use this to write headers and records.
    pub fn get_data_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        &mut self.data
    }

    /// Gets the logical ID of the page.
    pub fn get_page_id(&self) -> Option<PageId> {
        self.page_id
    }

    /// Sets the logical ID of the page.
    pub fn set_page_id(&mut self, page_id: Option<PageId>) {
        self.page_id = page_id;
    }

    /// Gets the current pin count.
    pub fn get_pin_count(&self) -> usize {
        self.pin_count
    }

    /// Increases the pin count, preventing the page from being evicted.
    pub fn pin(&mut self) {
        self.pin_count += 1;
    }

    /// Decreases the pin count.
    pub fn unpin(&mut self) {
        if self.pin_count > 0 {
            self.pin_count -= 1;
        }
    }

    /// Checks if the page is dirty.
    pub fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// Sets the dirty flag.
    pub fn set_dirty(&mut self, is_dirty: bool) {
        self.is_dirty = is_dirty;
    }

    /// Completely wipes the page state so it can be safely reused
    /// by the Buffer Pool Manager for a completely different PageId.
    pub fn reset_memory(&mut self) {
        self.data.fill(0);
        self.page_id = None;
        self.pin_count = 0;
        self.is_dirty = false;
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}
