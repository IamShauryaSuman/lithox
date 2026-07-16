use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const PAGE_SIZE: usize = 4096;
pub type PageId = u32;

pub struct DiskManager {
    file: File,
    next_page_id: PageId,
}

impl DiskManager {
    /// Opens the database file at the given path. Creates it if it doesn't exist.
    pub fn new(file_path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path)?;

        let file_size = file.metadata()?.len();
        let next_page_id = (file_size / PAGE_SIZE as u64) as PageId;

        Ok(Self { file, next_page_id })
    }

    /// Reads exactly 4KB of data from disk into the provided `page_data` buffer.
    pub fn read_page(
        &mut self,
        page_id: PageId,
        page_data: &mut [u8; PAGE_SIZE],
    ) -> io::Result<()> {
        let offset = (page_id as usize * PAGE_SIZE) as u64;
        self.file.seek(SeekFrom::Start(offset))?;

        self.file.read_exact(page_data)?;

        Ok(())
    }

    /// Writes exactly 4KB of data from `page_data` to the disk.
    pub fn write_page(&mut self, page_id: PageId, page_data: &[u8; PAGE_SIZE]) -> io::Result<()> {
        let offset = (page_id as usize * PAGE_SIZE) as u64;

        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(page_data)?;
        self.file.sync_all()?;

        Ok(())
    }

    /// Allocates a new logical page and returns its ID.
    pub fn allocate_page(&mut self) -> PageId {
        let page_id = self.next_page_id;
        self.next_page_id += 1;
        page_id
    }

    /// (Optional for V1) Deallocates a page so it can be reused later.
    pub fn deallocate_page(&mut self, _page_id: PageId) {
        // In a production database, we would maintain a "Free List" on disk
        // or a bitmap to track empty, deleted pages so we can reuse their Page IDs.
        // For V1, we can safely leave this as a no-op (doing nothing).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_allocate_page() {
        let test_file = "test_allocate.db";
        let _ = fs::remove_file(test_file);

        let mut disk_manager = DiskManager::new(test_file).unwrap();

        assert_eq!(disk_manager.allocate_page(), 0);
        assert_eq!(disk_manager.allocate_page(), 1);
        assert_eq!(disk_manager.allocate_page(), 2);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_read_write_page() {
        let test_file = "test_read_write.db";
        let _ = fs::remove_file(test_file);

        let mut disk_manager = DiskManager::new(test_file).unwrap();
        let page_id = disk_manager.allocate_page();

        let write_page = [65u8; PAGE_SIZE];
        disk_manager.write_page(page_id, &write_page).unwrap();

        let mut read_page = [0u8; PAGE_SIZE];
        disk_manager.read_page(page_id, &mut read_page).unwrap();

        assert_eq!(write_page, read_page);

        let _ = fs::remove_file(test_file);
    }
}
