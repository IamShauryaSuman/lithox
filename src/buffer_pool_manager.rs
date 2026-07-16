use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::disk_manager::{DiskManager, PageId};
use crate::page::Page;
use crate::replacer::{ClockReplacer, FrameId};

pub struct BufferPoolManager {
    disk_manager: DiskManager,
    replacer: ClockReplacer,
    pages: Vec<Arc<RwLock<Page>>>,
    page_table: HashMap<PageId, FrameId>,
    free_list: Vec<FrameId>,
}

impl BufferPoolManager {
    /// Initializes a new Buffer Pool Manager
    pub fn new(pool_size: usize, disk_manager: DiskManager) -> Self {
        let mut pages = Vec::with_capacity(pool_size);
        let mut free_list = Vec::with_capacity(pool_size);

        for i in 0..pool_size {
            pages.push(Arc::new(RwLock::new(Page::new())));
            free_list.push(i);
        }

        Self {
            disk_manager,
            replacer: ClockReplacer::new(pool_size),
            pages,
            page_table: HashMap::new(),
            free_list,
        }
    }

    /// Internal Helper: Finds a free frame or evicts an old one.
    /// Returns None if all frames are currently pinned.
    fn get_available_frame(&mut self) -> Option<FrameId> {
        if let Some(frame_id) = self.free_list.pop() {
            return Some(frame_id);
        }

        if let Some(victim_frame_id) = self.replacer.victim() {
            let mut page = self.pages[victim_frame_id].write().unwrap();

            if page.is_dirty() {
                let page_id = page.get_page_id().expect("Dirty page must have an ID");
                self.disk_manager
                    .write_page(page_id, page.get_data())
                    .unwrap();
                page.set_dirty(false);
            }

            if let Some(old_page_id) = page.get_page_id() {
                self.page_table.remove(&old_page_id);
            }

            return Some(victim_frame_id);
        }

        None
    }

    /// Fetches a page from the buffer pool. If it's not in memory, fetches it from disk.
    pub fn fetch_page(&mut self, page_id: PageId) -> Option<Arc<RwLock<Page>>> {
        if let Some(&frame_id) = self.page_table.get(&page_id) {
            let page_ref = Arc::clone(&self.pages[frame_id]);
            let mut page = page_ref.write().unwrap();

            page.pin();
            self.replacer.pin(frame_id);

            return Some(page_ref.clone());
        }

        let frame_id = self.get_available_frame()?;
        let page_ref = Arc::clone(&self.pages[frame_id]);

        let mut page = page_ref.write().unwrap();

        page.reset_memory();

        self.disk_manager
            .read_page(page_id, page.get_data_mut())
            .unwrap();

        page.set_page_id(Some(page_id));
        page.pin();

        self.page_table.insert(page_id, frame_id);
        self.replacer.pin(frame_id);

        Some(page_ref.clone())
    }

    /// Allocates a completely new page on disk and brings it into memory.
    pub fn new_page(&mut self) -> Option<(PageId, Arc<RwLock<Page>>)> {
        let frame_id = self.get_available_frame()?;

        let new_page_id = self.disk_manager.allocate_page();

        let page_ref = Arc::clone(&self.pages[frame_id]);
        let mut page = page_ref.write().unwrap();

        page.reset_memory();
        page.set_page_id(Some(new_page_id));
        page.pin();

        self.page_table.insert(new_page_id, frame_id);
        self.replacer.pin(frame_id);

        Some((new_page_id, page_ref.clone()))
    }

    /// Unpins a page, alerting the replacer that it is eligible for eviction.
    pub fn unpin_page(&mut self, page_id: PageId, is_dirty: bool) {
        if let Some(&frame_id) = self.page_table.get(&page_id) {
            let mut page = self.pages[frame_id].write().unwrap();

            page.unpin();
            if is_dirty {
                page.set_dirty(true);
            }

            if page.get_pin_count() == 0 {
                self.replacer.unpin(frame_id);
            }
        }
    }
}
